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

//! Runs `browser::finder::walk` on the blocking pool so opening the fuzzy finder on a big tree
//! never freezes the render loop. The walk sends each directory's entries as it lists them, so
//! the finder has results to show while it is still indexing. Dropping the job sets its cancel
//! flag, so closing the finder stops the walk at the next directory.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use browser::finder::{Candidate, Walked, walk};
use browser::search::Limits;
use shared::Vfs;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

enum Message {
    Batch(Vec<Candidate>),
    Finished(Walked),
}

/// What `FinderJob::poll` found out about the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Still walking; more may arrive.
    Walking,
    /// The walk is over. `truncated` says a limit stopped it early.
    Finished { truncated: bool },
}

pub struct FinderJob {
    cancel: Arc<AtomicBool>,
    rx: UnboundedReceiver<Message>,
    finished: bool,
}

impl FinderJob {
    /// Starts indexing `root` on `handle`'s blocking pool. Returns at once.
    pub fn start(
        handle: &tokio::runtime::Handle,
        vfs: Arc<dyn Vfs>,
        root: PathBuf,
        show_hidden: bool,
    ) -> Self {
        let (tx, rx) = unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_bg = Arc::clone(&cancel);
        handle.spawn_blocking(move || {
            let outcome = walk(
                vfs.as_ref(),
                &root,
                show_hidden,
                Limits::default(),
                &cancel_bg,
                |batch| {
                    let _ = tx.send(Message::Batch(batch));
                },
            );
            let _ = tx.send(Message::Finished(outcome));
        });
        Self {
            cancel,
            rx,
            finished: false,
        }
    }

    /// Moves everything that has arrived into `into` and says where the walk stands. A walk whose
    /// thread died without finishing reads as finished, so the finder stops claiming to index.
    pub fn poll(&mut self, into: &mut Vec<Candidate>) -> Progress {
        if self.finished {
            return Progress::Finished { truncated: false };
        }
        loop {
            match self.rx.try_recv() {
                Ok(Message::Batch(batch)) => into.extend(batch),
                Ok(Message::Finished(Walked::Done { truncated })) => {
                    self.finished = true;
                    return Progress::Finished { truncated };
                }
                Ok(Message::Finished(Walked::Cancelled)) | Err(TryRecvError::Disconnected) => {
                    self.finished = true;
                    return Progress::Finished { truncated: false };
                }
                Err(TryRecvError::Empty) => return Progress::Walking,
            }
        }
    }
}

impl Drop for FinderJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use shared::MemVfs;

    use super::*;
    use std::time::{Duration, Instant};

    fn drain(job: &mut FinderJob) -> (Vec<Candidate>, Progress) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut all = Vec::new();
        loop {
            let progress = job.poll(&mut all);
            if progress != Progress::Walking {
                return (all, progress);
            }
            assert!(Instant::now() < deadline, "the walk never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_job_delivers_every_entry_and_then_reports_the_end() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let vfs = MemVfs::new();
        vfs.add_file("/root/a/b/deep.txt", b"");
        vfs.add_file("/root/top.txt", b"");
        let mut job = FinderJob::start(runtime.handle(), Arc::new(vfs), "/root".into(), false);
        let (found, progress) = drain(&mut job);
        assert_eq!(progress, Progress::Finished { truncated: false });
        let mut rels: Vec<&str> = found.iter().map(|c| c.rel.as_str()).collect();
        rels.sort_unstable();
        assert_eq!(rels, ["a", "a/b", "a/b/deep.txt", "top.txt"]);
        // A finished job stays finished and delivers nothing more.
        let mut again = Vec::new();
        assert_eq!(
            job.poll(&mut again),
            Progress::Finished { truncated: false }
        );
        assert!(again.is_empty());
    }

    #[test]
    fn dropping_a_job_sets_its_cancel_flag() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let job = FinderJob::start(runtime.handle(), Arc::new(MemVfs::new()), "/".into(), false);
        let flag = Arc::clone(&job.cancel);
        assert!(!flag.load(Ordering::Relaxed));
        drop(job);
        assert!(flag.load(Ordering::Relaxed));
    }
}
