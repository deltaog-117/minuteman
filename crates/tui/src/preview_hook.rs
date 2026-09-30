// Minuteman - a fast, Ranger-inspired terminal file manager
// Copyright (C) 2026  Davi Oliveira Gonçalves
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Runs a configured `[[preview_hook]]` command (see `theming::PreviewHook`) to produce a
//! thumbnail image or extracted text for a file extension the built-in previews can't otherwise
//! show — a PDF, a video, or anything else a `pdftoppm`/`ffmpegthumbnailer`-style tool handles.
//! Spawned with the same spawn/poll/timeout/kill shape `git_status` already uses for `git`: a
//! user-chosen external tool is exactly as capable of hanging or never finishing as `git status`
//! is, and must never block the render loop.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use image::DynamicImage;
use theming::{HookKind, PreviewHook};

use crate::open::shell_quote;

/// A hook that doesn't set its own `timeout_ms` is killed after this long.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// How often a running hook is checked for exit.
const POLL_EVERY: Duration = Duration::from_millis(15);

/// Caps how much of a `Text`-kind hook's output is read back — the same size a browsed text file
/// is capped at, for the same reason: a runaway tool's output must not stall the read or bloat
/// memory.
const MAX_TEXT_BYTES: u64 = 1 << 20;

#[derive(Debug)]
pub enum HookOutcome {
    Image(DynamicImage),
    Text(String),
    /// The program is missing, exited non-zero, timed out, or its output couldn't be read back
    /// as its declared `kind` — the caller falls back to just the file's name, quietly.
    Failed,
}

/// The first configured hook whose `extensions` names `path`'s extension, case-insensitively.
pub fn hook_for<'a>(hooks: &'a [PreviewHook], path: &Path) -> Option<&'a PreviewHook> {
    let extension = path.extension()?.to_str()?;
    hooks.iter().find(|hook| {
        hook.extensions
            .iter()
            .any(|e| e.eq_ignore_ascii_case(extension))
    })
}

/// Runs `hook` on `path`: substitutes `{in}`/`{out}` into its command, waits for it to exit (or
/// kills it after its timeout), and reads back whatever it left at `{out}` per `hook.kind`. The
/// scratch file is always removed before returning, success or not.
pub fn run(hook: &PreviewHook, path: &Path) -> HookOutcome {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let tag = COUNTER.fetch_add(1, Ordering::Relaxed);
    let out_path = std::env::temp_dir().join(format!(
        "minuteman-preview-hook-{}-{tag}",
        std::process::id()
    ));
    let command = hook
        .command
        .replace("{in}", &shell_quote(&path.to_string_lossy()))
        .replace("{out}", &shell_quote(&out_path.to_string_lossy()));
    let outcome = if spawn_and_wait(&command, hook.timeout_ms.map(Duration::from_millis)) {
        read_output(&out_path, hook.kind).unwrap_or(HookOutcome::Failed)
    } else {
        HookOutcome::Failed
    };
    let _ = std::fs::remove_file(&out_path);
    outcome
}

/// Runs `command` under `sh -c`, polling until it exits or `timeout` (the hook's own, or
/// [`DEFAULT_TIMEOUT`]) elapses — the same shape `git_status::run_status` uses for `git`, since a
/// user-chosen tool is exactly as capable of hanging. Returns whether it exited successfully.
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

fn read_output(path: &Path, kind: HookKind) -> Option<HookOutcome> {
    match kind {
        // A hook's scratch output is always a file on this machine, whatever backend is browsed.
        HookKind::Image => preview::load_image(&shared::LocalVfs, path).map(HookOutcome::Image),
        HookKind::Text => {
            let metadata = std::fs::metadata(path).ok()?;
            if metadata.len() > MAX_TEXT_BYTES {
                return None;
            }
            String::from_utf8(std::fs::read(path).ok()?)
                .ok()
                .map(HookOutcome::Text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(command: &str, kind: HookKind) -> PreviewHook {
        PreviewHook {
            extensions: vec!["pdf".into()],
            command: command.into(),
            kind,
            timeout_ms: None,
        }
    }

    #[test]
    fn a_matching_extension_is_found_case_insensitively() {
        let hooks = vec![hook("true", HookKind::Text)];
        assert!(hook_for(&hooks, Path::new("report.PDF")).is_some());
        assert!(hook_for(&hooks, Path::new("report.txt")).is_none());
        assert!(hook_for(&hooks, Path::new("no_extension")).is_none());
    }

    #[test]
    fn a_successful_text_hook_reads_back_what_it_wrote() {
        let outcome = run(
            &hook("printf hello > {out}", HookKind::Text),
            Path::new("in.pdf"),
        );
        assert!(matches!(outcome, HookOutcome::Text(ref t) if t == "hello"));
    }

    #[test]
    fn a_successful_image_hook_decodes_what_it_wrote() {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-preview-hook-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("pixel.png");
        image::RgbImage::from_pixel(2, 2, image::Rgb([10, 20, 30]))
            .save(&source)
            .unwrap();

        let command = format!("cp {} {{out}}", shell_quote(&source.to_string_lossy()));
        let outcome = run(&hook(&command, HookKind::Image), Path::new("in.pdf"));
        match outcome {
            HookOutcome::Image(image) => assert_eq!((image.width(), image.height()), (2, 2)),
            other => panic!("expected an image, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_program_a_failing_exit_and_a_timeout_all_fail_quietly() {
        assert!(matches!(
            run(
                &hook("minuteman-no-such-program", HookKind::Text),
                Path::new("in.pdf")
            ),
            HookOutcome::Failed
        ));
        assert!(matches!(
            run(&hook("false", HookKind::Text), Path::new("in.pdf")),
            HookOutcome::Failed
        ));

        let mut slow = hook("sleep 5", HookKind::Text);
        slow.timeout_ms = Some(50);
        let started = Instant::now();
        assert!(matches!(
            run(&slow, Path::new("in.pdf")),
            HookOutcome::Failed
        ));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "a timed-out hook should be killed near-immediately, not waited out"
        );
    }

    #[test]
    fn oversized_text_output_is_rejected_rather_than_read_in_full() {
        let command = format!("yes | head -c {} > {{out}}", MAX_TEXT_BYTES + 1);
        let outcome = run(&hook(&command, HookKind::Text), Path::new("in.pdf"));
        assert!(matches!(outcome, HookOutcome::Failed));
    }
}
