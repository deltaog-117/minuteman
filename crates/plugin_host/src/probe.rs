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

//! A one-shot health check of a plugin's program: start it, send `init`, watch for a short
//! window, stop it. A plugin does not answer `init`, so there is no reply to wait for; what the
//! window can show is that the program runs, that it did not end at once, and what it logged at
//! `error` level (a plugin built for another `api_version` says so there and exits).
//!
//! It is passive and costs nothing between uses: the wait is a timed receive on a channel, not a
//! poll, it ends early when the process does, and nothing is left running afterwards.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::PluginError;
use crate::protocol::{self, EVENT_INIT, HostEvent, Incoming, LogLevel};

/// How long the probe watches a plugin that is still running.
pub const WINDOW: Duration = Duration::from_millis(500);

/// A process that closed its output exits within this long, or is taken to be running on.
const EXIT_GRACE: Duration = Duration::from_millis(50);

/// How the process stood when the probe was done with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Still running when the window closed; the probe stopped it.
    Stayed,
    /// Exited with status 0: allowed, as a plugin may do its job and finish.
    Finished,
    /// Exited with a failing status, worded by the system (`exit status: 1`, `signal: 11`).
    Crashed(String),
    /// Closed its output but kept running; the probe stopped it.
    ClosedOutput,
}

/// What the probe saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub outcome: Outcome,
    /// What the plugin logged at `error` level, in order.
    pub errors: Vec<String>,
}

impl Report {
    /// One line for the user: `Ok` when the plugin looks healthy, otherwise `Err` with the reason.
    ///
    /// # Errors
    ///
    /// The plugin crashed, closed its output, or logged an error; the first error it logged is
    /// named, as it is the one that explains the rest.
    pub fn verdict(&self, name: &str) -> Result<String, String> {
        if let Some(first) = self.errors.first() {
            return Err(format!("{name} reported: {first}"));
        }
        match &self.outcome {
            Outcome::Stayed => Ok(format!("{name} starts and keeps running")),
            Outcome::Finished => Ok(format!("{name} starts and finishes at once")),
            Outcome::Crashed(status) => Err(format!("{name} ended at start: {status}")),
            Outcome::ClosedOutput => Err(format!("{name} closed its output at start")),
        }
    }
}

/// Starts `command` with `args`, sends it `init` for `cwd`, and watches it for up to `window`.
///
/// # Errors
///
/// [`PluginError::Spawn`] when the program cannot be started.
pub fn probe(
    name: &str,
    command: &str,
    args: &[String],
    cwd: &Path,
    window: Duration,
) -> Result<Report, PluginError> {
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| PluginError::Spawn {
            name: name.to_string(),
            command: command.to_string(),
            source,
        })?;
    let mut stdin = child.stdin.take().expect("spawned with a piped stdin");
    let stdout = child.stdout.take().expect("spawned with a piped stdout");

    // The reader owns the sender, so the channel disconnects when the output closes.
    let (lines_tx, lines_rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if lines_tx.send(line).is_err() {
                return;
            }
        }
    });

    // A plugin that already ended cannot be written to; what it logged still tells why.
    let init = HostEvent::new(EVENT_INIT, cwd.to_path_buf(), Vec::new());
    let _ = stdin
        .write_all(init.to_line().as_bytes())
        .and_then(|()| stdin.flush());

    let deadline = Instant::now() + window;
    let mut errors = Vec::new();
    let output_closed = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match lines_rx.recv_timeout(left) {
            Ok(line) => {
                if let Ok(Incoming::Log {
                    level: LogLevel::Error,
                    message,
                }) = protocol::parse_line(&line)
                {
                    errors.push(message);
                }
            }
            Err(RecvTimeoutError::Timeout) => break false,
            Err(RecvTimeoutError::Disconnected) => break true,
        }
    };

    let outcome = if output_closed {
        match exit_within(&mut child, EXIT_GRACE) {
            Some(status) if status.success() => Outcome::Finished,
            Some(status) => Outcome::Crashed(status.to_string()),
            None => Outcome::ClosedOutput,
        }
    } else {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => Outcome::Finished,
            Ok(Some(status)) => Outcome::Crashed(status.to_string()),
            _ => Outcome::Stayed,
        }
    };
    // Reaps the process whatever the outcome, so none is left behind.
    let _ = child.kill();
    let _ = child.wait();
    Ok(Report { outcome, errors })
}

/// The status of `child` once it has exited, waiting up to `grace` for it to do so. A plugin that
/// has just closed its output is a moment from exiting, so this is a few short naps at most, and
/// only in that case.
fn exit_within(
    child: &mut std::process::Child,
    grace: Duration,
) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + grace;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const LONG: Duration = Duration::from_secs(5);

    fn sh(script: &str, window: Duration) -> Result<Report, PluginError> {
        probe(
            "t",
            "sh",
            &["-c".to_string(), script.to_string()],
            Path::new("/"),
            window,
        )
    }

    fn error_line(message: &str) -> String {
        format!(
            r#"printf '%s\n' '{{"jsonrpc":"2.0","method":"log","params":{{"level":"error","message":"{message}"}}}}'"#
        )
    }

    #[test]
    fn a_program_that_keeps_running_stays_and_is_stopped() {
        let started = Instant::now();
        let report = sh("sleep 30", Duration::from_millis(100)).unwrap();
        assert_eq!(report.outcome, Outcome::Stayed);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_program_that_exits_at_once_ends_the_probe_long_before_the_window() {
        let started = Instant::now();
        let report = sh("exit 3", LONG).unwrap();
        assert_eq!(report.outcome, Outcome::Crashed("exit status: 3".into()));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the wait must end with the process, took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_program_that_finishes_cleanly_is_not_a_failure() {
        let report = sh("exit 0", LONG).unwrap();
        assert_eq!(report.outcome, Outcome::Finished);
        assert_eq!(
            report.verdict("t"),
            Ok("t starts and finishes at once".into())
        );
    }

    #[test]
    fn an_error_it_logs_is_named_even_when_it_then_exits_cleanly() {
        let report = sh(&format!("{}; exit 0", error_line("api_version 9")), LONG).unwrap();
        assert_eq!(report.errors, ["api_version 9"]);
        assert_eq!(report.verdict("t"), Err("t reported: api_version 9".into()));
    }

    #[test]
    fn a_program_that_closes_its_output_but_runs_on_is_reported() {
        let report = sh("exec >&-; sleep 30", LONG).unwrap();
        assert_eq!(report.outcome, Outcome::ClosedOutput);
    }

    #[test]
    fn a_program_that_cannot_start_is_a_spawn_error() {
        let result = probe("t", "/no/such/program-minuteman", &[], Path::new("/"), LONG);
        assert!(matches!(result, Err(PluginError::Spawn { .. })));
    }

    #[test]
    fn init_reaches_the_program() {
        // `read` returns only once a line arrives on stdin, so the error below proves `init` was sent.
        let report = sh(
            r#"read line; printf '%s\n' '{"jsonrpc":"2.0","method":"log","params":{"level":"error","message":"got init"}}'"#,
            LONG,
        )
        .unwrap();
        assert_eq!(report.errors, ["got init"]);
    }

    proptest! {
        #[test]
        fn healthy_only_when_it_ran_cleanly_and_logged_no_error(
            errors in proptest::collection::vec("[a-z ]{1,12}", 0..3),
            code in 0u8..4,
            stayed in any::<bool>(),
        ) {
            let outcome = match (stayed, code) {
                (true, _) => Outcome::Stayed,
                (false, 0) => Outcome::Finished,
                (false, c) => Outcome::Crashed(format!("exit status: {c}")),
            };
            let report = Report { outcome: outcome.clone(), errors: errors.clone() };
            let healthy = report.verdict("t").is_ok();
            prop_assert_eq!(
                healthy,
                errors.is_empty() && matches!(outcome, Outcome::Stayed | Outcome::Finished)
            );
        }
    }
}
