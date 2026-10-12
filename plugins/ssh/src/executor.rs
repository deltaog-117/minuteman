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

use std::future::Future;
use std::io;
use std::sync::mpsc;

use tokio::runtime::{Builder, Runtime};

/// Worker threads are few: the work is waiting on a network, and SFTP requests are pipelined.
const WORKERS: usize = 2;

pub(crate) struct Executor {
    runtime: Option<Runtime>,
}

/// The runtime stopped, or the work panicked, before an answer came back.
#[derive(Debug)]
pub(crate) struct Lost;

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
    }
}
