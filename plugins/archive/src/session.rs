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

//! What the plugin does with each event the host writes: start a request on its own thread, or
//! raise the flag that stops one. A thread reports through the channel it was given, so the
//! session never touches a stream and a whole conversation can be played in a test.
//!
//! Progress is thinned to one message per `PROGRESS_EVERY`, because a tree of a million files
//! would otherwise be a million lines on the pipe. The last message of a request is always its
//! result, which nothing thins.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use plugin_api::client::notification_line;
use plugin_api::{
    ArchiveDone, ArchiveProgress, ArchiveStatus, EVENT_ARCHIVE, EVENT_ARCHIVE_CANCEL, EventParams,
    METHOD_ARCHIVE_DONE, METHOD_ARCHIVE_PROGRESS,
};

use crate::work::{Ended, execute};

/// The least time between two progress messages of one request.
const PROGRESS_EVERY: Duration = Duration::from_millis(40);

type Running = Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>>;

pub struct Session {
    out: Sender<String>,
    running: Running,
}

fn lock(running: &Running) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<AtomicBool>>> {
    // Nothing holds the lock across anything that can panic, and a poisoned map is still a valid
    // map, so carry on rather than take the plugin down.
    running.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Session {
    /// A session whose requests write their lines to `out`.
    pub fn new(out: Sender<String>) -> Self {
        Self {
            out,
            running: Running::default(),
        }
    }

    /// Acts on an event the host sent: an `archive` starts a request, an `archive_cancel` stops
    /// one. Anything else is not this plugin's business.
    pub fn on_event(&self, event: &EventParams) {
        match event.name.as_str() {
            EVENT_ARCHIVE => {
                if let Some(request) = &event.archive {
                    self.start(request.job, request.work.clone());
                }
            }
            EVENT_ARCHIVE_CANCEL => {
                if let Some(cancel) = &event.archive_cancel {
                    self.cancel(cancel.job);
                }
            }
            _ => {}
        }
    }

    /// Raises the flag of request `job`, if it is still running.
    pub fn cancel(&self, job: u64) {
        if let Some(flag) = lock(&self.running).get(&job) {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Stops every request, for when the host has gone.
    pub fn cancel_all(&self) {
        for flag in lock(&self.running).values() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    fn start(&self, job: u64, work: plugin_api::ArchiveWork) {
        let cancel = Arc::new(AtomicBool::new(false));
        // A host that reuses a number has lost track of the old request; it is stopped rather than
        // left running unreachable.
        if let Some(old) = lock(&self.running).insert(job, Arc::clone(&cancel)) {
            old.store(true, Ordering::Relaxed);
        }
        let out = self.out.clone();
        let running = Arc::clone(&self.running);
        let spawned = thread::Builder::new()
            .name(format!("archive-{job}"))
            .spawn({
                let (out, running, cancel) = (out.clone(), Arc::clone(&running), cancel);
                move || {
                    let ended = run_request(job, &work, &cancel, &out);
                    // Forget the request before announcing its end, so a host that reacts by
                    // reusing the number cannot have it cancelled by this one.
                    lock(&running).remove(&job);
                    let done = ArchiveDone {
                        job,
                        finished: ended.finished,
                        status: ended.status,
                    };
                    let _ = out.send(notification_line(METHOD_ARCHIVE_DONE, &done));
                }
            });
        if let Err(e) = spawned {
            lock(&running).remove(&job);
            let done = ArchiveDone {
                job,
                finished: Vec::new(),
                status: ArchiveStatus::Failed {
                    message: format!("could not start the work: {e}"),
                },
            };
            let _ = out.send(notification_line(METHOD_ARCHIVE_DONE, &done));
        }
    }
}

fn run_request(
    job: u64,
    work: &plugin_api::ArchiveWork,
    cancel: &AtomicBool,
    out: &Sender<String>,
) -> Ended {
    let mut reached = 0u64;
    let mut last_sent: Option<Instant> = None;
    execute(work, cancel, &mut |path| {
        reached += 1;
        let due = last_sent.is_none_or(|at| at.elapsed() >= PROGRESS_EVERY);
        if due {
            last_sent = Some(Instant::now());
            let name = path
                .file_name()
                .map_or_else(|| path.to_string_lossy(), |name| name.to_string_lossy())
                .into_owned();
            let progress = ArchiveProgress {
                job,
                done: reached,
                path: name,
            };
            let _ = out.send(notification_line(METHOD_ARCHIVE_PROGRESS, &progress));
        }
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    use plugin_api::{
        ArchiveWork, CompressJob, Conflict, Format, HostEvent, Level, METHOD_ARCHIVE_DONE,
    };
    use serde_json::Value;

    use super::*;

    fn event_for(job: u64, work: ArchiveWork) -> EventParams {
        let line = HostEvent::archive(job, work).to_line();
        match plugin_api::client::parse_incoming(&line).unwrap() {
            plugin_api::client::Incoming::Event(params) => *params,
            other @ plugin_api::client::Incoming::Reply { .. } => {
                panic!("an event was expected, got {other:?}")
            }
        }
    }

    fn lines_until_done(rx: &Receiver<String>) -> Vec<Value> {
        let mut seen = Vec::new();
        loop {
            let line = rx
                .recv_timeout(Duration::from_secs(30))
                .expect("the request ends");
            let value: Value = serde_json::from_str(&line).unwrap();
            let last = value["method"] == METHOD_ARCHIVE_DONE;
            seen.push(value);
            if last {
                return seen;
            }
        }
    }

    fn tree(label: &str, files: usize) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-archive-session-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        for n in 0..files {
            fs::write(dir.join("src").join(format!("f{n}.txt")), "x").unwrap();
        }
        dir
    }

    fn pack(dir: &std::path::Path) -> ArchiveWork {
        ArchiveWork::Compress {
            jobs: vec![CompressJob {
                sources: vec![dir.join("src")],
                out: dir.join("out.tar"),
            }],
            format: Format::Tar,
            level: Level::default(),
            policy: Conflict::Abort,
            password: None,
        }
    }

    #[test]
    fn progress_is_thinned_but_counts_up_and_the_result_is_always_last() {
        let dir = tree("thin", 400);
        let (tx, rx) = channel();
        let session = Session::new(tx);

        session.on_event(&event_for(5, pack(&dir)));
        let lines = lines_until_done(&rx);

        let (done, progress) = lines.split_last().unwrap();
        assert_eq!(done["params"]["status"]["kind"], "completed");
        let counts: Vec<u64> = progress
            .iter()
            .map(|line| line["params"]["done"].as_u64().unwrap())
            .collect();
        assert_eq!(
            counts.first(),
            Some(&1),
            "the first entry is always reported"
        );
        assert!(counts.windows(2).all(|w| w[0] < w[1]), "{counts:?}");
        assert!(
            counts.len() < 401,
            "a tree of 401 entries is not 401 messages: {}",
            counts.len()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_cancel_for_a_request_that_is_not_running_does_nothing() {
        let (tx, rx) = channel();
        let session = Session::new(tx);
        session.cancel(99);
        session.cancel_all();
        drop(session);
        assert!(rx.recv().is_err(), "nothing was written");
    }

    #[test]
    fn events_that_are_not_archive_work_are_ignored() {
        let (tx, rx) = channel();
        let session = Session::new(tx);
        let ignored = plugin_api::client::parse_incoming(
            &plugin_api::HostEvent::new("dir_changed", "/".into(), Vec::new()).to_line(),
        )
        .unwrap();
        if let plugin_api::client::Incoming::Event(params) = ignored {
            session.on_event(&params);
        }
        drop(session);
        assert!(rx.recv().is_err());
    }
}
