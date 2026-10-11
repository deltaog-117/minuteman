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

//! The way from a synchronous `Vfs` call to the plugin that answers for a kind of path, and back.
//!
//! The caller blocks on a std channel; the reply arrives through the plugin's reader task, which
//! hands it to [`Pending::answer`]. The link outlives the plugin's process: it is made before any
//! plugin starts (the `Vfs` that holds it is built first), is attached to each process as it
//! starts, and counts them as epochs so a handle from a dead process cannot reach a new one.
//!
//! Model, for the parts where order matters. State: `conn` is either absent or the live process
//! with its epoch `e`. `attach` replaces `conn` and raises the epoch to `e + 1`, then fails every
//! request still waiting on the old one. A request that names a handle is sent only if the
//! handle's epoch equals the epoch of `conn` at the moment it is sent (checked under the lock that
//! `attach` takes), so no request carrying handle `h` of epoch `e` is ever written to a process
//! of epoch `e + 1`. A waiting request ends by exactly one of: its reply, its process ending
//! (`fail_all`), its timeout.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use plugin_api::{Answer, Epoch, Peer, PeerError, VfsFault, VfsRequest, request_to_line};
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::life::{Exit, Life};

/// How long one request waits for the plugin. Long enough for a large archive's table of contents
/// to be read over a slow link; the wait also ends the moment the plugin's process does.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(120);

type Waiter = Sender<Result<Value, PeerError>>;

#[derive(Default)]
struct Table {
    /// Set when the process has ended: nothing will ever answer, so nothing new may wait.
    closed: bool,
    waiting: HashMap<u64, Waiter>,
}

/// The requests written to one process of the plugin that have no reply yet.
#[derive(Default)]
pub struct Pending {
    next: AtomicU64,
    table: Mutex<Table>,
}

impl Pending {
    fn table(&self) -> MutexGuard<'_, Table> {
        // Nothing holds the lock across anything that can panic, and a poisoned table is still a
        // valid table.
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers a request to wait for, or `None` once the process has ended. Without that, a
    /// request that raced the end (the process not yet seen as gone by whoever looked) would be
    /// registered after the last `fail_all` and wait out its whole timeout for nothing.
    fn expect(&self) -> Option<(u64, std::sync::mpsc::Receiver<Result<Value, PeerError>>)> {
        let mut table = self.table();
        if table.closed {
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = channel();
        table.waiting.insert(id, tx);
        Some((id, rx))
    }

    fn forget(&self, id: u64) {
        self.table().waiting.remove(&id);
    }

    /// Hands the plugin's reply `id` to the request that waits for it. A reply for a request that
    /// already timed out is dropped.
    pub fn answer(&self, id: &Value, outcome: Result<Value, VfsFault>) {
        let Some(id) = id.as_u64() else { return };
        let waiter = self.table().waiting.remove(&id);
        if let Some(waiter) = waiter {
            let _ = waiter.send(outcome.map_err(PeerError::Fault));
        }
    }

    /// The process is gone: every request still waiting on it ends as stopped, and none can start
    /// waiting afterwards.
    pub fn fail_all(&self) {
        let waiting = {
            let mut table = self.table();
            table.closed = true;
            std::mem::take(&mut table.waiting)
        };
        for (_, waiter) in waiting {
            let _ = waiter.send(Err(PeerError::Stopped));
        }
    }
}

/// One process the link is attached to.
#[derive(Clone)]
struct Conn {
    epoch: Epoch,
    stdin: UnboundedSender<String>,
    pending: Arc<Pending>,
    life: Arc<Life>,
}

#[derive(Default)]
struct State {
    epoch: u64,
    conn: Option<Conn>,
}

/// See the module documentation.
#[derive(Default)]
pub struct ServeLink {
    state: Mutex<State>,
}

impl ServeLink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Points the link at a process that has just started, ending whatever waited on the last.
    pub(crate) fn attach(
        &self,
        stdin: UnboundedSender<String>,
        pending: Arc<Pending>,
        life: Arc<Life>,
    ) {
        let old = {
            let mut state = self.state();
            state.epoch += 1;
            let epoch = Epoch(state.epoch);
            state.conn.replace(Conn {
                epoch,
                stdin,
                pending,
                life,
            })
        };
        if let Some(old) = old {
            old.pending.fail_all();
        }
    }

    /// Whether a process is attached and has not ended.
    pub fn is_running(&self) -> bool {
        self.state()
            .conn
            .as_ref()
            .is_some_and(|conn| conn.life.exit() == Exit::Running)
    }
}

impl Peer for ServeLink {
    fn release(&self, request: &VfsRequest, after: Epoch) {
        let state = self.state();
        let Some(conn) = state.conn.as_ref() else {
            return;
        };
        if conn.epoch != after || conn.life.exit() != Exit::Running {
            return;
        }
        // The reply, when it comes, finds no one waiting and is dropped.
        let Some((id, _reply)) = conn.pending.expect() else {
            return;
        };
        let _ = conn.stdin.send(request_to_line(id, request));
    }

    fn call(&self, request: &VfsRequest, after: Option<Epoch>) -> Result<Answer, PeerError> {
        let (conn, id, reply) = {
            let state = self.state();
            let conn = state.conn.clone().ok_or(PeerError::Unavailable)?;
            if after.is_some_and(|epoch| epoch != conn.epoch) {
                return Err(PeerError::Stale);
            }
            if conn.life.exit() != Exit::Running {
                return Err(PeerError::Unavailable);
            }
            let Some((id, reply)) = conn.pending.expect() else {
                return Err(PeerError::Stopped);
            };
            // Written while the lock is held, so an `attach` cannot slip between the epoch check
            // and the write.
            if conn.stdin.send(request_to_line(id, request)).is_err() {
                conn.pending.forget(id);
                return Err(PeerError::Stopped);
            }
            (conn, id, reply)
        };
        match reply.recv_timeout(REPLY_TIMEOUT) {
            Ok(Ok(value)) => Ok(Answer {
                epoch: conn.epoch,
                value,
            }),
            Ok(Err(error)) => Err(error),
            Err(RecvTimeoutError::Timeout) => {
                conn.pending.forget(id);
                Err(PeerError::TimedOut)
            }
            Err(RecvTimeoutError::Disconnected) => Err(PeerError::Stopped),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use serde_json::json;
    use tokio::sync::mpsc::unbounded_channel;

    use super::*;

    /// A stand-in plugin process: the lines the host wrote to it, and the way to answer them.
    struct Fake {
        lines: tokio::sync::mpsc::UnboundedReceiver<String>,
        pending: Arc<Pending>,
        life: Arc<Life>,
    }

    fn attached(link: &ServeLink) -> Fake {
        let (stdin, lines) = unbounded_channel();
        let pending = Arc::new(Pending::default());
        let life = Life::new();
        link.attach(stdin, Arc::clone(&pending), Arc::clone(&life));
        Fake {
            lines,
            pending,
            life,
        }
    }

    impl Fake {
        /// Reads the next request and answers it with `result`.
        fn answer_next(&mut self, result: Result<Value, VfsFault>) -> Value {
            let line = loop {
                match self.lines.try_recv() {
                    Ok(line) => break line,
                    Err(_) => thread::yield_now(),
                }
            };
            let sent: Value = serde_json::from_str(&line).unwrap();
            self.pending.answer(&sent["id"], result);
            sent
        }
    }

    fn list() -> VfsRequest {
        VfsRequest::List {
            path: "/x/a.zip!".into(),
        }
    }

    #[test]
    fn with_no_process_attached_the_request_is_unavailable() {
        let link = ServeLink::new();
        assert!(!link.is_running());
        assert_eq!(link.call(&list(), None), Err(PeerError::Unavailable));
    }

    #[test]
    fn a_request_is_written_as_a_vfs_line_and_waits_for_its_reply() {
        let link = ServeLink::new();
        let mut fake = attached(&link);
        let waiter = {
            let link = Arc::clone(&link);
            thread::spawn(move || link.call(&list(), None))
        };
        let sent = fake.answer_next(Ok(json!([])));
        assert_eq!(sent["method"], "vfs");
        assert_eq!(sent["params"]["op"], "list");
        let answer = waiter.join().unwrap().unwrap();
        assert_eq!(answer.value, json!([]));
        assert_eq!(answer.epoch, Epoch(1));
    }

    #[test]
    fn a_refusal_comes_back_as_the_plugins_fault() {
        let link = ServeLink::new();
        let mut fake = attached(&link);
        let waiter = {
            let link = Arc::clone(&link);
            thread::spawn(move || link.call(&list(), None))
        };
        fake.answer_next(Err(VfsFault::NotFound));
        assert_eq!(
            waiter.join().unwrap(),
            Err(PeerError::Fault(VfsFault::NotFound))
        );
    }

    #[test]
    fn replies_go_to_their_own_requests_in_any_order() {
        let link = ServeLink::new();
        let mut fake = attached(&link);
        let ask = |n: u64| {
            let link = Arc::clone(&link);
            thread::spawn(move || {
                link.call(
                    &VfsRequest::Read {
                        handle: 1,
                        offset: n,
                        len: 1,
                    },
                    None,
                )
            })
        };
        let (first, second) = (ask(10), ask(20));
        let mut sent = [
            fake.answer_next(Ok(json!({ "echo": 0 }))),
            fake.answer_next(Ok(json!({ "echo": 0 }))),
        ];
        // Answer by the offset each carried, whichever order they were written in.
        sent.sort_by_key(|line| line["params"]["offset"].as_u64());
        assert_eq!(sent[0]["params"]["offset"], 10);
        assert!(first.join().unwrap().is_ok() && second.join().unwrap().is_ok());
    }

    #[test]
    fn a_process_that_ends_wakes_the_requests_waiting_on_it() {
        let link = ServeLink::new();
        let mut fake = attached(&link);
        let waiter = {
            let link = Arc::clone(&link);
            thread::spawn(move || link.call(&list(), None))
        };
        // Wait until the request is written, then end the process the way the reader task does.
        while fake.lines.try_recv().is_err() {
            thread::yield_now();
        }
        fake.pending.fail_all();
        assert_eq!(waiter.join().unwrap(), Err(PeerError::Stopped));
    }

    #[test]
    fn a_handle_from_an_earlier_process_is_never_sent_to_a_later_one() {
        let link = ServeLink::new();
        let mut first = attached(&link);
        let waiter = {
            let link = Arc::clone(&link);
            thread::spawn(move || link.call(&list(), None))
        };
        first.answer_next(Ok(json!({})));
        let epoch = waiter.join().unwrap().unwrap().epoch;

        let mut second = attached(&link);
        let read = VfsRequest::Read {
            handle: 1,
            offset: 0,
            len: 1,
        };
        assert_eq!(link.call(&read, Some(epoch)), Err(PeerError::Stale));
        assert!(
            second.lines.try_recv().is_err(),
            "nothing was written to the new process"
        );
    }

    #[test]
    fn attaching_a_new_process_ends_what_waited_on_the_old_one() {
        let link = ServeLink::new();
        let mut old = attached(&link);
        let waiter = {
            let link = Arc::clone(&link);
            thread::spawn(move || link.call(&list(), None))
        };
        while old.lines.try_recv().is_err() {
            thread::yield_now();
        }
        let _new = attached(&link);
        assert_eq!(waiter.join().unwrap(), Err(PeerError::Stopped));
    }

    #[test]
    fn a_request_that_races_the_end_of_the_process_ends_at_once_not_after_its_timeout() {
        let link = ServeLink::new();
        let mut fake = attached(&link);
        // The reader task has seen the process end, but nothing has recorded its exit yet, so the
        // link still takes it for running.
        fake.pending.fail_all();
        assert!(link.is_running());

        let started = std::time::Instant::now();
        assert_eq!(link.call(&list(), None), Err(PeerError::Stopped));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            fake.lines.try_recv().is_err(),
            "nothing is written to a process known to be gone"
        );
    }

    #[test]
    fn an_ended_process_is_unavailable_not_written_to() {
        let link = ServeLink::new();
        let fake = attached(&link);
        fake.life.set_exit(Exit::Finished);
        assert!(!link.is_running());
        assert_eq!(link.call(&list(), None), Err(PeerError::Unavailable));
    }

    #[test]
    fn a_reply_nobody_waits_for_is_dropped() {
        let pending = Pending::default();
        pending.answer(&json!(99), Ok(json!({})));
        pending.answer(&json!("not a number"), Ok(json!({})));
    }
}
