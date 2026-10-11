//! Runs a configured `[[preview_hook]]`: one that extracts text — `pdftotext`, `catdoc`, anything
//! that turns a file the built-in previews cannot read into text — and hands back what it wrote,
//! or one that renders a picture — `pdftoppm`, `ffmpegthumbnailer` — whose output file is left in
//! the temporary folder for the interface to take (see `plugin_api::handoff_path`). The host
//! decides which hooks reach this plugin (see `bundled` in `tui`).
//!
//! A user-chosen external tool is as capable of hanging as `git status` is, so it is spawned,
//! polled against a timeout and killed, never waited on without bound. A missing program, a
//! non-zero exit, a timeout or output that is not usable all come back as `None`, and the caller
//! shows just the file's name.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use plugin_api::{ImageSource, MAX_TEXT_CHARS, PreviewContent, handoff_path};
use serde::Deserialize;

/// A hook that doesn't set its own `timeout_ms` is killed after this long.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// How often a running hook is checked for exit.
const POLL_EVERY: Duration = Duration::from_millis(15);

/// Caps how much of a hook's output is read back: the size a browsed text file is capped at, for
/// the same reason — a runaway tool's output must not stall the read or bloat memory.
const MAX_OUTPUT_BYTES: u64 = MAX_TEXT_CHARS as u64;

/// Caps the size of a picture a hook may leave: a runaway tool must not fill the temporary folder
/// or hand the interface something it would take seconds to decode.
const MAX_IMAGE_BYTES: u64 = 64 << 20;

/// What a hook's output is.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Text,
    Image,
}

/// One hook, as the host passes it on the command line (`--text-hook <json>` or
/// `--image-hook <json>`; the second sets `kind`).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Hook {
    /// Extensions this hook applies to, without the dot, matched case-insensitively.
    pub extensions: Vec<String>,
    /// The command, with `{in}` and `{out}` standing for the quoted input path and the file the
    /// hook writes its text to.
    pub command: String,
    /// How long it may run before being killed. `None` means the built-in default.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub kind: Kind,
}

/// The first hook whose `extensions` names `path`'s extension, case-insensitively.
pub fn hook_for<'a>(hooks: &'a [Hook], path: &Path) -> Option<&'a Hook> {
    let extension = path.extension()?.to_str()?;
    hooks.iter().find(|hook| {
        hook.extensions
            .iter()
            .any(|e| e.eq_ignore_ascii_case(extension))
    })
}

/// `text` as one shell word. Single quotes make everything literal, so the only character that
/// needs care is a single quote itself: close the quote, add an escaped one, reopen.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The hook's command for `path`, writing to `out_path`.
fn command_for(hook: &Hook, path: &Path, out_path: &Path) -> String {
    hook.command
        .replace("{in}", &shell_quote(&path.to_string_lossy()))
        .replace("{out}", &shell_quote(&out_path.to_string_lossy()))
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Runs a text `hook` on `path`, a file on this machine's disk, and reads back what it left at
/// `{out}`. The scratch file is always removed before returning, success or not.
pub fn run(hook: &Hook, path: &Path) -> Option<String> {
    let tag = COUNTER.fetch_add(1, Ordering::Relaxed);
    let out_path = std::env::temp_dir().join(format!(
        "minuteman-preview-hook-{}-{tag}",
        std::process::id()
    ));
    let command = command_for(hook, path, &out_path);
    let text = spawn_and_wait(&command, hook.timeout_ms.map(Duration::from_millis))
        .then(|| read_output(&out_path))
        .flatten();
    let _ = std::fs::remove_file(&out_path);
    text
}

/// Runs a picture `hook` on `path`, a file on this machine's disk, and returns the file it wrote,
/// named so the interface may take it (`plugin_api::handoff_path`). The file is kept: the
/// interface deletes it once decoded. Nothing is left behind when the hook fails, or writes
/// something that is not a plain file or is too big.
pub fn run_image(hook: &Hook, path: &Path) -> Option<PathBuf> {
    let out_path = handoff_path(COUNTER.fetch_add(1, Ordering::Relaxed));
    let command = command_for(hook, path, &out_path);
    let made = spawn_and_wait(&command, hook.timeout_ms.map(Duration::from_millis))
        // `symlink_metadata`, so a hook that leaves a link is refused instead of followed.
        && std::fs::symlink_metadata(&out_path)
            .is_ok_and(|meta| meta.is_file() && (1..=MAX_IMAGE_BYTES).contains(&meta.len()));
    if !made {
        let _ = std::fs::remove_file(&out_path);
        return None;
    }
    Some(out_path)
}

/// Deletes the scratch picture a document that will not be sent was holding, if it held one.
pub fn discard(content: &PreviewContent) {
    if let PreviewContent::Image {
        source: ImageSource::Handoff { file },
    } = content
    {
        let _ = std::fs::remove_file(file);
    }
}

/// Runs `command` under `sh -c`, polling until it exits or `timeout` (the hook's own, or
/// [`DEFAULT_TIMEOUT`]) elapses. Returns whether it exited successfully.
fn spawn_and_wait(command: &str, timeout: Option<Duration>) -> bool {
    let Ok(mut child) = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => std::thread::sleep(POLL_EVERY),
            Err(_) => return false,
        }
    }
}

fn read_output(path: &Path) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > MAX_OUTPUT_BYTES {
        return None;
    }
    String::from_utf8(std::fs::read(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(command: &str) -> Hook {
        Hook {
            extensions: vec!["pdf".into()],
            command: command.into(),
            timeout_ms: None,
            kind: Kind::Text,
        }
    }

    fn image_hook(command: &str) -> Hook {
        Hook {
            kind: Kind::Image,
            ..hook(command)
        }
    }

    #[test]
    fn a_matching_extension_is_found_case_insensitively() {
        let hooks = vec![hook("true")];
        assert!(hook_for(&hooks, Path::new("report.PDF")).is_some());
        assert!(hook_for(&hooks, Path::new("report.txt")).is_none());
        assert!(hook_for(&hooks, Path::new("no_extension")).is_none());
    }

    #[test]
    fn a_successful_hook_reads_back_what_it_wrote() {
        assert_eq!(
            run(&hook("printf hello > {out}"), Path::new("in.pdf")).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn the_input_path_reaches_the_command_quoted() {
        let dir = std::env::temp_dir().join(format!("minuteman-hook-quote-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tricky = dir.join("it's a file.pdf");
        std::fs::write(&tricky, "body").unwrap();
        assert_eq!(
            run(&hook("cat {in} > {out}"), &tricky).as_deref(),
            Some("body")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_program_a_failing_exit_and_a_timeout_all_fail_quietly() {
        assert_eq!(
            run(&hook("minuteman-no-such-program"), Path::new("in.pdf")),
            None
        );
        assert_eq!(run(&hook("false"), Path::new("in.pdf")), None);

        let mut slow = hook("sleep 5");
        slow.timeout_ms = Some(50);
        let started = Instant::now();
        assert_eq!(run(&slow, Path::new("in.pdf")), None);
        assert!(started.elapsed() < Duration::from_secs(4), "it was killed");
    }

    #[test]
    fn output_that_is_not_text_or_is_too_big_is_refused() {
        assert_eq!(
            run(&hook("printf '\\377\\376' > {out}"), Path::new("in.pdf")),
            None
        );
        let big = hook("head -c 1100000 /dev/zero | tr '\\0' a > {out}");
        assert_eq!(run(&big, Path::new("in.pdf")), None);
    }

    #[test]
    fn the_scratch_file_is_removed() {
        let dir =
            std::env::temp_dir().join(format!("minuteman-hook-scratch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let note = dir.join("where-it-wrote");
        // The command records its own `{out}` path, so the test can look for exactly that file.
        let command = format!(
            "echo {{out}} > {} && printf x > {{out}}",
            shell_quote(&note.to_string_lossy())
        );
        assert_eq!(
            run(&hook(&command), Path::new("in.pdf")).as_deref(),
            Some("x")
        );
        let out = std::fs::read_to_string(&note).unwrap();
        assert!(!Path::new(out.trim()).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_picture_hook_leaves_a_file_the_interface_may_take() {
        let file = run_image(&image_hook("printf PNGDATA > {out}"), Path::new("in.pdf"))
            .expect("the hook wrote a file");
        assert!(plugin_api::is_handoff_path(&file));
        assert_eq!(std::fs::read(&file).unwrap(), b"PNGDATA");
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn a_picture_hook_that_fails_or_writes_nothing_usable_leaves_nothing() {
        let note_dir =
            std::env::temp_dir().join(format!("minuteman-hook-img-{}", std::process::id()));
        std::fs::create_dir_all(&note_dir).unwrap();
        let note = note_dir.join("where");
        for command in [
            "minuteman-no-such-program".to_string(),
            "false".to_string(),
            "true".to_string(),
            "head -c 70000000 /dev/zero > {out}".to_string(),
            "ln -s /etc/passwd {out}".to_string(),
        ] {
            // Record the path the hook was given, then do its work, so the test can look for
            // exactly that file afterwards.
            let command = format!(
                "echo {{out}} > {} ; {command}",
                shell_quote(&note.to_string_lossy())
            );
            let out = run_image(&image_hook(&command), Path::new("in.pdf"));
            assert_eq!(out, None, "{command}");
            let named = std::fs::read_to_string(&note).unwrap();
            assert!(
                std::fs::symlink_metadata(named.trim()).is_err(),
                "{command} left its file behind"
            );
        }
        std::fs::remove_dir_all(&note_dir).unwrap();
    }

    #[test]
    fn a_picture_hook_is_killed_at_its_timeout() {
        let mut slow = image_hook("sleep 5");
        slow.timeout_ms = Some(50);
        let started = Instant::now();
        assert_eq!(run_image(&slow, Path::new("in.pdf")), None);
        assert!(started.elapsed() < Duration::from_secs(4), "it was killed");
    }

    #[test]
    fn a_hook_without_a_kind_is_a_text_hook() {
        let hook: Hook = serde_json::from_str(r#"{"extensions":["pdf"],"command":"x"}"#).unwrap();
        assert_eq!(hook.kind, Kind::Text);
        let hook: Hook =
            serde_json::from_str(r#"{"extensions":["pdf"],"command":"x","kind":"image"}"#).unwrap();
        assert_eq!(hook.kind, Kind::Image);
    }
}
