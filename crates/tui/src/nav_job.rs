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

//! Loads a folder on the blocking pool, so opening one on a slow filesystem — most of all another
//! machine, where connecting alone can take ten seconds — never freezes the render loop. One
//! `NavJob` is one move; the browser keeps showing the old folder until the result is applied
//! (see `browser::MovePlan`). Dropping a job abandons it: the thread finishes its read, finds
//! nobody listening and discards the result.

use std::path::PathBuf;
use std::sync::Arc;

use browser::{LoadedMove, MovePlan};
use shared::{Vfs, VfsError};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// How a running move stands.
pub enum NavPoll {
    Pending,
    Done(Result<LoadedMove, VfsError>),
    /// The thread died without answering.
    Lost,
}

pub struct NavJob {
    rx: UnboundedReceiver<Result<LoadedMove, VfsError>>,
    /// Shown while loading, and after a failure.
    pub label: String,
    /// Where the move goes, so a failure can be told apart by what it was opening.
    pub target: PathBuf,
    /// The status line once the move is applied, if the caller wants one.
    pub done_status: Option<String>,
    /// Put in front of the error when the move fails, e.g. `cd failed`.
    pub error_prefix: String,
}

impl NavJob {
    /// Starts loading `plan` on `handle`'s blocking pool. Returns at once.
    pub fn start(
        handle: &tokio::runtime::Handle,
        vfs: Arc<dyn Vfs>,
        plan: MovePlan,
        done_status: Option<String>,
        error_prefix: &str,
    ) -> Self {
        let (tx, rx) = unbounded_channel();
        let label = plan.target_label();
        let target = plan.target().to_path_buf();
        handle.spawn_blocking(move || {
            let _ = tx.send(plan.load(vfs.as_ref()));
        });
        Self {
            rx,
            label,
            target,
            done_status,
            error_prefix: error_prefix.into(),
        }
    }

    pub fn poll(&mut self) -> NavPoll {
        match self.rx.try_recv() {
            Ok(result) => NavPoll::Done(result),
            Err(TryRecvError::Empty) => NavPoll::Pending,
            Err(TryRecvError::Disconnected) => NavPoll::Lost,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use browser::BrowserState;
    use shared::LocalVfs;

    use super::*;

    fn scratch(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("minuteman-nav-job-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        dir
    }

    fn wait(job: &mut NavJob) -> NavPoll {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match job.poll() {
                NavPoll::Pending if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                other => return other,
            }
        }
    }

    #[test]
    fn a_job_delivers_the_loaded_folder() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("ok");
        let mut browser = BrowserState::new(&LocalVfs, root.clone()).unwrap();
        let plan = browser.plan_goto(&root.join("inner"));
        let mut job = NavJob::start(
            runtime.handle(),
            Arc::new(LocalVfs),
            plan,
            None,
            "cd failed",
        );
        assert_eq!(job.label, "inner");

        let NavPoll::Done(Ok(loaded)) = wait(&mut job) else {
            panic!("expected a loaded folder");
        };
        assert!(browser.apply_move(loaded));
        assert_eq!(browser.current_dir(), root.join("inner"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_job_reports_the_error_and_moves_nothing() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("err");
        let browser = BrowserState::new(&LocalVfs, root.clone()).unwrap();
        let plan = browser.plan_goto(&root.join("missing"));
        let mut job = NavJob::start(
            runtime.handle(),
            Arc::new(LocalVfs),
            plan,
            None,
            "cd failed",
        );
        assert!(matches!(wait(&mut job), NavPoll::Done(Err(_))));
        assert_eq!(browser.current_dir(), root);
        let _ = std::fs::remove_dir_all(&root);
    }
}
