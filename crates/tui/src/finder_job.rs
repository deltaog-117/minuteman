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

//! Everything the fuzzy finder does that costs time, off the render thread. Two jobs run on the
//! blocking pool: one walks the tree (`browser::finder::walk`) and sends each directory's entries
//! as it lists them; the other, the engine, owns the index and the ranking, takes in those entries
//! and every change of query, and sends back a `Snapshot` of the best results. The render thread
//! therefore never scores anything: it sends a query, and shows the newest snapshot it has.
//!
//! The engine works in small slices and looks at its queue between them, so a new letter typed
//! while a large index is still being scored takes over within a few milliseconds instead of
//! waiting for the old query to finish. Snapshots are sent at most every `INTERVAL`, with the
//! first one after a query change sent at once. Dropping the job cancels the walk, which ends the
//! engine, because the engine stops when every sender of its queue is gone.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use browser::finder::{Candidate, Ranking, SHOWN, Walked, walk};
use browser::search::{Limits, Query};
use shared::Vfs;

/// How many entries the engine scores between looks at its queue: a few milliseconds of work, so
/// that is also the longest a new query waits to be noticed.
const SLICE: usize = 2048;

/// The least time between two snapshots, so a walk delivering thousands of directories a second
/// does not make the render thread rebuild its results for each one.
const INTERVAL: Duration = Duration::from_millis(40);

/// Where the walk that feeds the index stands, for the finder's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indexing {
    Walking,
    Done { truncated: bool },
}

/// What the engine knows at a moment: the best results for the query it was last given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Which query these results are for (see `FinderJob::set_query`).
    pub generation: u64,
    /// The best matches, best first, at most `SHOWN`.
    pub results: Vec<Candidate>,
    /// How many entries match so far (more than `results` holds when it is a lot).
    pub matches: usize,
    /// How many entries have been indexed so far.
    pub indexed: usize,
    pub indexing: Indexing,
    /// Whether every indexed entry has been ranked against the query. False while the engine is
    /// still catching up, when `results` may yet change.
    pub settled: bool,
}

impl Snapshot {
    /// What the finder shows before the engine has said anything.
    pub fn empty() -> Self {
        Self {
            generation: 0,
            results: Vec::new(),
            matches: 0,
            indexed: 0,
            indexing: Indexing::Walking,
            settled: false,
        }
    }
}

enum Command {
    Batch(Vec<Candidate>),
    Walked(Walked),
    Query(u64, Option<Query>),
}

pub struct FinderJob {
    cancel: Arc<AtomicBool>,
    commands: Sender<Command>,
    snapshots: Receiver<Snapshot>,
}

impl FinderJob {
    /// Starts indexing `root` on `handle`'s blocking pool. Returns at once.
    pub fn start(
        handle: &tokio::runtime::Handle,
        vfs: Arc<dyn Vfs>,
        root: PathBuf,
        show_hidden: bool,
    ) -> Self {
        let (commands, queue) = channel();
        let (out, snapshots) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_bg = Arc::clone(&cancel);
        let walker = commands.clone();
        handle.spawn_blocking(move || {
            let outcome = walk(
                vfs.as_ref(),
                &root,
                show_hidden,
                Limits::default(),
                &cancel_bg,
                |batch| {
                    let _ = walker.send(Command::Batch(batch));
                },
            );
            let _ = walker.send(Command::Walked(outcome));
        });
        handle.spawn_blocking(move || Engine::default().run(&queue, &out));
        Self {
            cancel,
            commands,
            snapshots,
        }
    }

    /// Ranks against `query` from here on (`None` lists the index as met). `generation` must rise
    /// with every call; snapshots carry the one they answer, so the caller can tell a late
    /// snapshot of an old query from a current one.
    pub fn set_query(&self, generation: u64, query: Option<Query>) {
        let _ = self.commands.send(Command::Query(generation, query));
    }

    /// The newest snapshot that has arrived since the last call, if any. Older ones are dropped
    /// unread: each is a complete picture.
    pub fn poll(&self) -> Option<Snapshot> {
        self.snapshots.try_iter().last()
    }
}

impl Drop for FinderJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct Engine {
    index: Vec<Candidate>,
    ranking: Ranking,
    generation: u64,
    walked: Option<Walked>,
}

/// Why the engine did not get a command this turn.
enum Wait {
    /// Nothing arrived, but the senders are still there.
    Idle,
    /// Every sender is gone: the finder was closed.
    Gone,
}

impl From<TryRecvError> for Wait {
    fn from(e: TryRecvError) -> Self {
        match e {
            TryRecvError::Empty => Wait::Idle,
            TryRecvError::Disconnected => Wait::Gone,
        }
    }
}

impl From<RecvTimeoutError> for Wait {
    fn from(e: RecvTimeoutError) -> Self {
        match e {
            RecvTimeoutError::Timeout => Wait::Idle,
            RecvTimeoutError::Disconnected => Wait::Gone,
        }
    }
}

impl Engine {
    /// Takes in one command and says whether it changed the query.
    fn apply(&mut self, command: Command) -> bool {
        match command {
            Command::Batch(batch) => self.index.extend(batch),
            Command::Walked(outcome) => self.walked = Some(outcome),
            Command::Query(generation, query) => {
                self.generation = generation;
                self.ranking.set_query(query);
                return true;
            }
        }
        false
    }

    fn snapshot(&self, settled: bool) -> Snapshot {
        let indexing = match self.walked {
            None => Indexing::Walking,
            Some(Walked::Done { truncated }) => Indexing::Done { truncated },
            Some(Walked::Cancelled) => Indexing::Done { truncated: false },
        };
        Snapshot {
            generation: self.generation,
            results: self
                .ranking
                .top(self.index.len(), SHOWN)
                .into_iter()
                .map(|position| self.index[position].clone())
                .collect(),
            matches: self.ranking.matches(self.index.len()),
            indexed: self.index.len(),
            indexing,
            settled,
        }
    }

    /// Runs until the finder is closed. While there is ranking to do it does one slice at a time
    /// and only peeks at the queue; with nothing to do it sleeps until a command arrives or a
    /// snapshot is due.
    fn run(mut self, queue: &Receiver<Command>, out: &Sender<Snapshot>) {
        let mut settled = true;
        // Something changed since the last snapshot, so one is owed.
        let mut owed = false;
        let mut next_send = Instant::now();
        loop {
            let received = if !settled {
                queue.try_recv().map_err(Wait::from)
            } else if owed {
                queue
                    .recv_timeout(next_send.saturating_duration_since(Instant::now()))
                    .map_err(Wait::from)
            } else {
                queue.recv().map_err(|_| Wait::Gone)
            };
            match received {
                Ok(command) => {
                    let mut retyped = self.apply(command);
                    while let Ok(command) = queue.try_recv() {
                        retyped |= self.apply(command);
                    }
                    settled = false;
                    owed = true;
                    if retyped {
                        next_send = Instant::now();
                    }
                }
                Err(Wait::Idle) => {}
                Err(Wait::Gone) => return,
            }
            if !settled {
                settled = self.ranking.advance(&self.index, SLICE);
            }
            if owed && Instant::now() >= next_send {
                if out.send(self.snapshot(settled)).is_err() {
                    return;
                }
                next_send = Instant::now() + INTERVAL;
                owed = !settled;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use shared::MemVfs;

    use super::*;

    /// Polls until a snapshot answers `generation`, is settled and the walk is over.
    fn settled(job: &FinderJob, generation: u64) -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(snapshot) = job.poll()
                && snapshot.generation == generation
                && snapshot.settled
                && snapshot.indexing != Indexing::Walking
            {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "the engine never settled");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn tree() -> MemVfs {
        let vfs = MemVfs::new();
        vfs.add_file("/root/a/b/deep.txt", b"");
        vfs.add_file("/root/top.txt", b"");
        vfs
    }

    #[test]
    fn a_job_lists_every_entry_when_nothing_is_typed() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let job = FinderJob::start(runtime.handle(), Arc::new(tree()), "/root".into(), false);
        let snapshot = settled(&job, 0);
        assert_eq!(snapshot.indexing, Indexing::Done { truncated: false });
        assert_eq!(snapshot.indexed, 4);
        assert_eq!(snapshot.matches, 4);
        let mut rels: Vec<&str> = snapshot.results.iter().map(|c| c.rel.as_str()).collect();
        rels.sort_unstable();
        assert_eq!(rels, ["a", "a/b", "a/b/deep.txt", "top.txt"]);
    }

    #[test]
    fn a_query_is_answered_with_its_own_generation_and_only_its_matches() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let job = FinderJob::start(runtime.handle(), Arc::new(tree()), "/root".into(), false);
        settled(&job, 0);
        job.set_query(1, Query::new("deep"));
        let snapshot = settled(&job, 1);
        assert_eq!(snapshot.matches, 1);
        assert_eq!(snapshot.results[0].rel, "a/b/deep.txt");
        job.set_query(2, Query::new("de"));
        assert_eq!(settled(&job, 2).results[0].rel, "a/b/deep.txt");
        job.set_query(3, None);
        assert_eq!(settled(&job, 3).matches, 4);
    }

    #[test]
    fn dropping_a_job_sets_its_cancel_flag_and_ends_the_engine() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let job = FinderJob::start(runtime.handle(), Arc::new(tree()), "/root".into(), false);
        let flag = Arc::clone(&job.cancel);
        assert!(!flag.load(Ordering::Relaxed));
        drop(job);
        assert!(flag.load(Ordering::Relaxed));
        // The runtime waits for its blocking jobs when dropped, so a stuck engine hangs here.
        drop(runtime);
    }

    #[test]
    fn a_large_index_is_ranked_in_slices_and_a_new_query_takes_over() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let vfs = MemVfs::new();
        for i in 0..6000 {
            vfs.add_file(format!("/root/dir{}/file{i}.txt", i % 50), b"");
        }
        let job = FinderJob::start(runtime.handle(), Arc::new(vfs), "/root".into(), false);
        job.set_query(1, Query::new("f"));
        job.set_query(2, Query::new("file5999"));
        let snapshot = settled(&job, 2);
        assert_eq!(snapshot.results[0].rel, "dir49/file5999.txt");
        assert_eq!(snapshot.matches, 1);
    }
}
