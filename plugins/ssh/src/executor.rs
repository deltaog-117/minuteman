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

//! Runs asynchronous SSH work from the synchronous [`shared::Vfs`] methods.
//!
//! The file manager calls a `Vfs` from its render thread and from plain background threads, so
//! the SFTP client lives on a small runtime of its own and a caller waits on a standard channel.
//! That never touches the caller's own runtime, so it works from inside one as well.
//!
//! Work that only reads may be given up ([`Executor::run_cancellable`]): when the calling thread's
//! `shared::cancel` token is set, the task is aborted and the caller returns at once. The SFTP
//! client's futures are cancel safe, so an aborted read leaves the session usable.

use std::future::Future;
use std::io;
use std::sync::mpsc;
use std::time::Duration;

use shared::cancel::{self, Waited};
use tokio::runtime::{Builder, Runtime};
use tokio::task::AbortHandle;

/// Worker threads are few: the work is waiting on a network, and SFTP requests are pipelined.
const WORKERS: usize = 2;

pub(crate) struct Executor {
    runtime: Option<Runtime>,
}

/// The runtime stopped, or the work panicked, before an answer came back.
#[derive(Debug)]
pub(crate) struct Lost;

/// Why work run with [`Executor::run_cancellable`] has no output.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unfinished {
    Lost,
    /// The calling thread's job was given up, and the work with it.
    Cancelled,
}

impl Executor {
    pub(crate) fn new() -> io::Result<Self> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(WORKERS)
            .thread_name("minuteman-ssh")
            .enable_all()
            .build()?;
        Ok(Self {
            runtime: Some(runtime),
        })
    }

    /// Runs `work` to completion and returns its output, blocking the calling thread.
    pub(crate) fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = T> + Send + 'static,
    ) -> Result<T, Lost> {
        let runtime = self.runtime.as_ref().ok_or(Lost)?;
        let (send, receive) = mpsc::sync_channel(1);
        runtime.spawn(async move {
            // The receiver only goes away when the caller has given up, so there is no one to tell.
            let _ = send.send(work.await);
        });
        receive.recv().map_err(|_| Lost)
    }

    /// As [`Self::run`], but aborts `work` and returns as soon as the calling thread's job is given
    /// up. Only for work that changes nothing, since it may stop anywhere.
    pub(crate) fn run_cancellable<T: Send + 'static>(
        &self,
        work: impl Future<Output = T> + Send + 'static,
    ) -> Result<T, Unfinished> {
        let runtime = self.runtime.as_ref().ok_or(Unfinished::Lost)?;
        let (send, receive) = mpsc::sync_channel(1);
        let task = runtime.spawn(async move {
            let _ = send.send(work.await);
        });
        match cancel::recv(&receive, Duration::MAX) {
            Waited::Got(output) => Ok(output),
            Waited::Cancelled => {
                task.abort();
                Err(Unfinished::Cancelled)
            }
            Waited::TimedOut | Waited::Disconnected => Err(Unfinished::Lost),
        }
    }

    /// Starts `work` without waiting for it, for work several callers share (a connection being
    /// made). The handle stops it.
    pub(crate) fn spawn(
        &self,
        work: impl Future<Output = ()> + Send + 'static,
    ) -> Result<AbortHandle, Lost> {
        let runtime = self.runtime.as_ref().ok_or(Lost)?;
        Ok(runtime.spawn(work).abort_handle())
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        // Dropping a runtime waits for its tasks, and this may run where blocking is not allowed.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_runs_and_its_output_comes_back() {
        let executor = Executor::new().unwrap();
        assert_eq!(executor.run(async { 2 + 2 }).unwrap(), 4);
    }

    #[test]
    fn it_can_be_called_from_inside_another_runtime() {
        let outer = Builder::new_current_thread().build().unwrap();
        let executor = Executor::new().unwrap();
        let answer = outer.block_on(async { executor.run(async { 7 }).unwrap() });
        assert_eq!(answer, 7);
    }

    #[test]
    fn work_that_panics_is_reported_not_hung() {
        let executor = Executor::new().unwrap();
        assert!(executor.run(async { panic!("boom") }).is_err());
        assert_eq!(
            executor.run_cancellable(async { panic!("boom") }),
            Err::<(), _>(Unfinished::Lost)
        );
    }

    #[test]
    fn cancellable_work_runs_to_its_output_when_nobody_gives_up() {
        let executor = Executor::new().unwrap();
        let _scope = shared::Cancel::new().enter();
        assert_eq!(executor.run_cancellable(async { 5 }), Ok(5));
    }

    #[test]
    fn giving_up_aborts_the_work_and_returns_at_once() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Instant;

        /// Set when the future holding it is dropped, which an abort does.
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }

        let executor = Executor::new().unwrap();
        let token = shared::Cancel::new();
        let canceller = token.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            canceller.cancel();
        });
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Dropped(Arc::clone(&dropped));
        let _scope = token.enter();
        let started = Instant::now();
        let outcome = executor.run_cancellable(async move {
            let _guard = guard;
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        assert_eq!(outcome, Err(Unfinished::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(5));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !dropped.load(Ordering::Relaxed) && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(
            dropped.load(Ordering::Relaxed),
            "the aborted work was dropped"
        );
    }
}
