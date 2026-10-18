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

//! Runs a plugin's half-second probe (`plugin_host::probe`) on the blocking pool, so "Start and
//! test" in the plugin menu never holds the screen while it waits. One `ProbeJob` is one test.
//! Dropping a job gives it up: the probe stops by itself at the end of its window and finds
//! nobody listening.

use std::path::PathBuf;

use plugin_host::probe;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// How a running test stands.
pub enum ProbePoll {
    Pending,
    /// The verdict, as `Report::verdict` words it, or why the program could not be started.
    Done(Result<String, String>),
    /// The thread died without answering.
    Lost,
}

pub struct ProbeJob {
    rx: UnboundedReceiver<Result<String, String>>,
    /// The plugin being tested, so a verdict is only shown on the card it belongs to.
    pub name: String,
}

impl ProbeJob {
    /// Starts `command` for `window` on `handle`'s blocking pool. Returns at once.
    pub fn start(
        handle: &tokio::runtime::Handle,
        name: String,
        command: String,
        args: Vec<String>,
        cwd: PathBuf,
        window: std::time::Duration,
    ) -> Self {
        let (tx, rx) = unbounded_channel();
        let label = name.clone();
        handle.spawn_blocking(move || {
            let verdict = probe::probe(&label, &command, &args, &cwd, window)
                .map_err(|e| format!("could not start {label}: {e}"))
                .and_then(|report| report.verdict(&label));
            let _ = tx.send(verdict);
        });
        Self { rx, name }
    }

    pub fn poll(&mut self) -> ProbePoll {
        match self.rx.try_recv() {
            Ok(result) => ProbePoll::Done(result),
            Err(TryRecvError::Empty) => ProbePoll::Pending,
            Err(TryRecvError::Disconnected) => ProbePoll::Lost,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn run(script: &str) -> (Duration, ProbePoll) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let started = Instant::now();
        let mut job = ProbeJob::start(
            runtime.handle(),
            "t".into(),
            "sh".into(),
            vec!["-c".into(), script.into()],
            std::env::temp_dir(),
            Duration::from_millis(300),
        );
        let returned = started.elapsed();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match job.poll() {
                ProbePoll::Pending if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                other => return (returned, other),
            }
        }
    }

    #[test]
    fn starting_returns_before_the_window_ends_and_the_verdict_follows() {
        let (returned, polled) = run("sleep 30");
        assert!(returned < Duration::from_millis(150), "start must not wait");
        let ProbePoll::Done(verdict) = polled else {
            panic!("the job should have answered");
        };
        assert_eq!(verdict, Ok("t starts and keeps running".to_string()));
    }

    #[test]
    fn a_program_that_ends_at_once_is_a_failed_verdict() {
        let (_, polled) = run("exit 2");
        let ProbePoll::Done(verdict) = polled else {
            panic!("the job should have answered");
        };
        assert_eq!(verdict, Err("t ended at start: exit status: 2".to_string()));
    }
}
