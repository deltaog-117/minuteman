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

//! Runs `browser::search::find_below` on the blocking pool so a search from `~` never freezes the
//! render loop, the way `live_refresh` does for directory listings. One `SearchJob` is one
//! search; the `/` prompt starts a fresh one on every keystroke and drops the one before it.
//! Dropping a job sets its cancel flag, and each job owns its own channel, so a slow search for
//! `aer` can neither keep walking after the query became `aere` nor deliver its answer late and
//! move the cursor somewhere the current query wouldn't.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use browser::search::{Limits, Outcome, Query, find_below};
use shared::LocalVfs;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// Where the `/` prompt's current search stands, for its status line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchState {
    /// Nothing typed yet, or the search was stopped.
    #[default]
    Idle,
    Searching,
    Found,
    /// Nothing matched; `truncated` says a search limit stopped the walk early.
    NotFound {
        truncated: bool,
    },
}

impl SearchState {
    /// A short note to show in front of the query, if this state has one.
    pub fn note(self) -> Option<&'static str> {
        match self {
            SearchState::Idle | SearchState::Found => None,
            SearchState::Searching => Some("searching…"),
            SearchState::NotFound { truncated: false } => Some("no match"),
            SearchState::NotFound { truncated: true } => Some("no match (search limit reached)"),
        }
    }
}

pub struct SearchJob {
    cancel: Arc<AtomicBool>,
    rx: UnboundedReceiver<Outcome>,
}

impl SearchJob {
    /// Starts searching `root` for `query` on `handle`'s blocking pool. Returns at once.
    pub fn start(
        handle: &tokio::runtime::Handle,
        root: PathBuf,
        query: Query,
        show_hidden: bool,
    ) -> Self {
        let (tx, rx) = unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_bg = Arc::clone(&cancel);
        handle.spawn_blocking(move || {
            let outcome = find_below(
                &LocalVfs,
                &root,
                &query,
                show_hidden,
                Limits::default(),
                &cancel_bg,
            );
            let _ = tx.send(outcome);
        });
        Self { cancel, rx }
    }

    /// The finished search's outcome, once there is one. A search whose thread died without
    /// answering reads as cancelled, so the prompt stops claiming to be searching.
    pub fn poll(&mut self) -> Option<Outcome> {
        match self.rx.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Outcome::Cancelled),
        }
    }
}

impl Drop for SearchJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-search-job-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wait_for(job: &mut SearchJob) -> Outcome {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(outcome) = job.poll() {
                return outcome;
            }
            assert!(Instant::now() < deadline, "the search never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_job_finds_a_name_several_directories_down() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("found");
        std::fs::create_dir_all(root.join("a").join("b")).unwrap();
        std::fs::write(root.join("a").join("b").join("aerend.md"), b"").unwrap();

        let mut job = SearchJob::start(
            runtime.handle(),
            root.clone(),
            Query::new("AEREND").unwrap(),
            false,
        );
        assert_eq!(
            wait_for(&mut job),
            Outcome::Found(root.join("a").join("b").join("aerend.md"))
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_job_reports_a_missing_name() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("missing");
        std::fs::write(root.join("something"), b"").unwrap();

        let mut job = SearchJob::start(
            runtime.handle(),
            root.clone(),
            Query::new("nothing-like-it").unwrap(),
            false,
        );
        assert_eq!(wait_for(&mut job), Outcome::NotFound { truncated: false });
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn dropping_a_job_sets_its_cancel_flag() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("drop");
        let job = SearchJob::start(
            runtime.handle(),
            root.clone(),
            Query::new("x").unwrap(),
            false,
        );
        let flag = Arc::clone(&job.cancel);
        assert!(!flag.load(Ordering::Relaxed));
        drop(job);
        assert!(flag.load(Ordering::Relaxed));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn only_a_search_in_progress_or_a_miss_has_a_note() {
        assert_eq!(SearchState::Idle.note(), None);
        assert_eq!(SearchState::Found.note(), None);
        assert!(SearchState::Searching.note().is_some());
        assert_ne!(
            SearchState::NotFound { truncated: false }.note(),
            SearchState::NotFound { truncated: true }.note()
        );
    }
}
