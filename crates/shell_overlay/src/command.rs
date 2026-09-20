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

//! Runs one non-interactive shell command line to completion and captures what it prints, for
//! the `:` command prompt. Unlike `spawn_shell` (which hands the real terminal to a shell) and
//! `PopupShell` (a pty-backed shell you type into), nothing here is interactive: stdin is closed,
//! so a program that waits for input sees end-of-file instead of hanging the app.

use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How much of a command's output is kept. The rest is read and discarded so the child never
/// blocks on a full pipe, but a runaway `yes` can't grow memory without bound.
const MAX_CAPTURED_BYTES: usize = 64 * 1024;

/// How often the wait loop checks whether the child exited or the caller cancelled.
const POLL_INTERVAL: Duration = Duration::from_millis(15);

/// How long to wait, after the shell itself exits, for the output pipe to close. A command that
/// backgrounds a process (`sleep 60 &`) leaves it holding the pipe open, and its output must not
/// keep the caller waiting.
const OUTPUT_GRACE: Duration = Duration::from_millis(200);

/// What a finished command printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// `None` when the process was killed by a signal.
    pub code: Option<i32>,
    /// Standard output and standard error interleaved in the order they were written.
    pub text: String,
    /// Whether output past `MAX_CAPTURED_BYTES` was dropped.
    pub truncated: bool,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    Finished(CommandOutput),
    /// `cancel` was set and the shell was killed before it finished.
    Cancelled,
}

/// Runs `command` through `sh -c` with `cwd` as its working directory and blocks until it
/// finishes or `cancel` is set (which kills the shell — a process it already forked into the
/// background is not tracked and survives).
///
/// # Errors
///
/// Returns the I/O error if `sh` cannot be spawned or waited on.
pub fn run_command(cwd: &Path, command: &str, cancel: &AtomicBool) -> io::Result<CommandOutcome> {
    // One pipe for both streams keeps stdout and stderr in the order they were written, the way
    // a terminal would show them.
    let (mut reader, writer) = io::pipe()?;
    let mut child = {
        let mut sh = Command::new("sh");
        sh.arg("-c")
            .arg(command)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(writer.try_clone()?)
            .stderr(writer);
        sh.spawn()?
        // `sh` (and the write ends it owns) drops here: if it lingered, the reader below would
        // never see end-of-file once the child exits.
    };

    let captured = Arc::new(Mutex::new((Vec::new(), false)));
    let sink = Arc::clone(&captured);
    let reader_thread = std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(read @ 1..) = reader.read(&mut chunk) {
            let mut guard = sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let room = MAX_CAPTURED_BYTES.saturating_sub(guard.0.len());
            guard.0.extend_from_slice(&chunk[..read.min(room)]);
            guard.1 |= read > room;
        }
    });

    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            child.kill()?;
            child.wait()?;
            return Ok(CommandOutcome::Cancelled);
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    let deadline = Instant::now() + OUTPUT_GRACE;
    while !reader_thread.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }

    let (bytes, truncated) = {
        let guard = captured
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (guard.0.clone(), guard.1)
    };
    Ok(CommandOutcome::Finished(CommandOutput {
        code: status.code(),
        text: String::from_utf8_lossy(&bytes).into_owned(),
        truncated,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(command: &str) -> CommandOutput {
        let cwd = std::env::temp_dir();
        match run_command(&cwd, command, &AtomicBool::new(false)).unwrap() {
            CommandOutcome::Finished(output) => output,
            CommandOutcome::Cancelled => panic!("nothing set the cancel flag"),
        }
    }

    #[test]
    fn captures_both_streams_and_a_zero_exit_code() {
        let output = run("echo out; echo err 1>&2");
        assert!(output.success());
        assert_eq!(output.text, "out\nerr\n");
        assert!(!output.truncated);
    }

    #[test]
    fn reports_a_failing_exit_code() {
        let output = run("echo nope; exit 3");
        assert_eq!(output.code, Some(3));
        assert!(!output.success());
        assert_eq!(output.text, "nope\n");
    }

    #[test]
    fn runs_in_the_given_directory() {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-run-command-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let outcome = run_command(&dir, "touch marker && pwd", &AtomicBool::new(false)).unwrap();

        assert!(dir.join("marker").exists());
        let CommandOutcome::Finished(output) = outcome else {
            panic!("command was not cancelled");
        };
        assert_eq!(
            Path::new(output.text.trim()).canonicalize().unwrap(),
            dir.canonicalize().unwrap()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_program_reading_stdin_sees_end_of_file_instead_of_hanging() {
        let output = run("cat; echo done");
        assert_eq!(output.text, "done\n");
    }

    #[test]
    fn output_beyond_the_cap_is_dropped_but_the_command_still_finishes() {
        let output = run("head -c 300000 /dev/zero | tr '\\0' x");
        assert!(output.success());
        assert!(output.truncated);
        assert_eq!(output.text.len(), MAX_CAPTURED_BYTES);
    }

    #[test]
    fn cancelling_kills_a_long_running_command_promptly() {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let started = Instant::now();
        let setter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            flag.store(true, Ordering::Relaxed);
        });

        let outcome = run_command(&std::env::temp_dir(), "exec sleep 30", &cancel).unwrap();

        setter.join().unwrap();
        assert_eq!(outcome, CommandOutcome::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_backgrounded_process_does_not_hold_the_result_hostage() {
        let started = Instant::now();
        let output = run("sleep 5 & echo started");
        assert_eq!(output.text, "started\n");
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
