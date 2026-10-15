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
//! (`fail_all`), its timeout, its caller's job being given up (`shared::cancel`).
//!
//! A request whose caller stops waiting (a timeout or a cancel) is not forgotten but *abandoned*:
//! its entry stays until the reply comes, because a reply that opened a file would otherwise hold
//! the plugin's handle for good. Per id the states are `Waiting{opens}` -> `Abandoned{opens}` ->
//! gone, or `Waiting` -> gone, the step to gone being the reply or `fail_all`. Invariant: a handle
//! opened for a caller that left is closed exactly once. If the reply came first it sits in the
//! caller's channel, which the caller drains after abandoning; if it comes after, `answer` finds
//! the entry abandoned and hands back the `close`. The table's lock orders the two, so exactly one
//! of them sees the reply. Only a cancel tells the plugin (`vfs_cancel`): after a timeout the work
//! may be a long change worth finishing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use plugin_api::{
    Answer, Epoch, Opened, Peer, PeerError, Reply, VfsFault, VfsRequest, cancel_line,
    request_to_line,
};
use serde_json::Value;
use shared::cancel::{self, Waited};
use tokio::sync::mpsc::UnboundedSender;

use crate::life::{Exit, Life};

/// How long one request waits for the plugin. Long enough for a large archive's table of contents
/// to be read over a slow link; the wait also ends the moment the plugin's process does.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(120);

type Waiter = Sender<Result<Value, PeerError>>;

/// A request with no reply yet. `opens` says its reply carries a handle ([`Opened`]).
enum Entry {
    Waiting {
        waiter: Waiter,
        opens: bool,
    },
    /// Nobody waits for the reply any more (see the module documentation).
    Abandoned {
        opens: bool,
    },
}

#[derive(Default)]
struct Table {
    /// Set when the process has ended: nothing will ever answer, so nothing new may wait.
    closed: bool,
    waiting: HashMap<u64, Entry>,
}

/// Whether `request`'s reply hands out a handle that must be closed again.
fn opens(request: &VfsRequest) -> bool {
    matches!(request, VfsRequest::Open { .. } | VfsRequest::Create { .. })
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
    fn expect(&self, opens: bool) -> Option<(u64, Receiver<Result<Value, PeerError>>)> {
        let mut table = self.table();
        if table.closed {
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = channel();
        table
            .waiting
            .insert(id, Entry::Waiting { waiter: tx, opens });
        Some((id, rx))
    }

    /// Registers a request whose reply nobody will wait for, or `None` once the process ended.
    fn expect_none(&self) -> Option<u64> {
        let mut table = self.table();
        if table.closed {
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        table.waiting.insert(id, Entry::Abandoned { opens: false });
        Some(id)
    }

    fn forget(&self, id: u64) {
        self.table().waiting.remove(&id);
    }

    /// The caller of `id`, reading from `reply`, stops waiting. Returns the `close` to write when
    /// the reply already came and opened a file (`opens`); one still to come is left to
    /// [`Self::answer`].
    fn give_up(
        &self,
        id: u64,
        opens: bool,
        reply: &Receiver<Result<Value, PeerError>>,
    ) -> Option<String> {
        {
            let mut table = self.table();
            if let Some(entry) = table.waiting.get_mut(&id) {
                if let Entry::Waiting { opens, .. } = *entry {
                    *entry = Entry::Abandoned { opens };
                }
                return None;
            }
        }
        // Not in the table, so `answer` already sent the reply to the channel.
        match reply.try_recv() {
            Ok(Ok(value)) if opens => self.close_line(&value),
            _ => None,
        }
    }

    /// The `close` line for the handle in `reply`, registered so its own reply is dropped.
    fn close_line(&self, reply: &Value) -> Option<String> {
        let opened: Opened = serde_json::from_value(reply.clone()).ok()?;
        let id = self.expect_none()?;
        Some(request_to_line(
            id,
            &VfsRequest::Close {
                handle: opened.handle,
            },
        ))
    }

    /// Hands the plugin's reply `id` to the request that waits for it. Returns the line to write
    /// back to the plugin when the reply opened a file for a request nobody waits for any more:
    /// the `close` that lets the file go.
    pub fn answer(&self, id: &Value, outcome: Result<Value, VfsFault>) -> Option<String> {
        let id = id.as_u64()?;
        let entry = self.table().waiting.remove(&id)?;
        match entry {
            Entry::Waiting { waiter, .. } => {
                let _ = waiter.send(outcome.map_err(PeerError::Fault));
                None
            }
            Entry::Abandoned { opens: true } => self.close_line(outcome.as_ref().ok()?),
            Entry::Abandoned { opens: false } => None,
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
        for entry in waiting.into_values() {
            if let Entry::Waiting { waiter, .. } = entry {
                let _ = waiter.send(Err(PeerError::Stopped));
            }
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
        let Some(id) = conn.pending.expect_none() else {
            return;
        };
        let _ = conn.stdin.send(request_to_line(id, request));
    }

    fn call(&self, request: &VfsRequest, after: Option<Epoch>) -> Result<Answer, PeerError> {
        self.submit(request, after)?.wait()
    }

    fn submit(
        &self,
        request: &VfsRequest,
        after: Option<Epoch>,
    ) -> Result<Box<dyn Reply>, PeerError> {
        let state = self.state();
        let conn = state.conn.clone().ok_or(PeerError::Unavailable)?;
        if after.is_some_and(|epoch| epoch != conn.epoch) {
            return Err(PeerError::Stale);
        }
        if conn.life.exit() != Exit::Running {
            return Err(PeerError::Unavailable);
        }
        // A job given up on asks nothing more.
        if cancel::cancelled() {
            return Err(PeerError::Cancelled);
        }
        let opens = opens(request);
        let Some((id, reply)) = conn.pending.expect(opens) else {
            return Err(PeerError::Stopped);
        };
        // Written while the lock is held, so an `attach` cannot slip between the epoch check
        // and the write.
        if conn.stdin.send(request_to_line(id, request)).is_err() {
            conn.pending.forget(id);
            return Err(PeerError::Stopped);
        }
        Ok(Box::new(Waiting {
            conn,
            id,
            opens,
            reply,
        }))
    }
}

/// A request written to the plugin whose reply is still to come.
struct Waiting {
    conn: Conn,
    id: u64,
    opens: bool,
    reply: Receiver<Result<Value, PeerError>>,
}

impl Waiting {
    /// Stops waiting for the reply, closing the file it opened if it already came.
    fn give_up(&self) {
        if let Some(close) = self.conn.pending.give_up(self.id, self.opens, &self.reply) {
            let _ = self.conn.stdin.send(close);
        }
    }
}

impl Reply for Waiting {
    fn wait(self: Box<Self>) -> Result<Answer, PeerError> {
        match cancel::recv(&self.reply, REPLY_TIMEOUT) {
            Waited::Got(Ok(value)) => Ok(Answer {
                epoch: self.conn.epoch,
                value,
            }),
            Waited::Got(Err(error)) => Err(error),
            Waited::TimedOut => {
                self.give_up();
                Err(PeerError::TimedOut)
            }
            Waited::Cancelled => {
                self.give_up();
                let _ = self.conn.stdin.send(cancel_line(self.id));
                Err(PeerError::Cancelled)
            }
            Waited::Disconnected => Err(PeerError::Stopped),
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
    fn a_second_request_is_written_before_the_first_is_answered() {
        let link = ServeLink::new();
        let mut fake = attached(&link);
        let first = link.submit(&list(), None).unwrap();
        let second = link.submit(&list(), None).unwrap();
        // Answered in the opposite order to prove each reply finds its own request.
        let sent_first: Value =
            serde_json::from_str(&fake.lines.try_recv().expect("first written")).unwrap();
        let sent_second: Value =
            serde_json::from_str(&fake.lines.try_recv().expect("second written")).unwrap();
        fake.pending.answer(&sent_second["id"], Ok(json!("two")));
        fake.pending.answer(&sent_first["id"], Ok(json!("one")));
        assert_eq!(first.wait().unwrap().value, json!("one"));
        assert_eq!(second.wait().unwrap().value, json!("two"));
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
        assert_eq!(pending.answer(&json!(99), Ok(json!({}))), None);
        assert_eq!(pending.answer(&json!("not a number"), Ok(json!({}))), None);
    }

    fn open() -> VfsRequest {
        VfsRequest::Open {
            path: "ssh://h/f".into(),
        }
    }

    /// `request` written to the attached process, as `submit` does, but as the concrete type.
    fn written(link: &ServeLink, request: &VfsRequest) -> Waiting {
        let conn = link.state().conn.clone().unwrap();
        let opens = opens(request);
        let (id, reply) = conn.pending.expect(opens).unwrap();
        conn.stdin.send(request_to_line(id, request)).unwrap();
        Waiting {
            conn,
            id,
            opens,
            reply,
        }
    }

    /// The next line written to the fake process, waiting for it.
    fn next_line(fake: &mut Fake) -> Value {
        loop {
            match fake.lines.try_recv() {
                Ok(line) => return serde_json::from_str(&line).unwrap(),
                Err(_) => thread::yield_now(),
            }
        }
    }

    /// One step of a caller and the plugin racing over one request.
    #[derive(Debug, Clone, Copy)]
    enum Step {
        Reply,
        GiveUp,
        Take,
    }

    fn closed_handle(line: &str) -> u64 {
        let line: Value = serde_json::from_str(line).unwrap();
        assert_eq!(line["params"]["op"], "close");
        line["params"]["handle"].as_u64().unwrap()
    }

    proptest::proptest! {
        /// Whatever order replies, give-ups and takes come in, a file opened for a caller that
        /// left is closed exactly once, one a caller took is never closed, nothing else is ever
        /// closed, and no request is left in the table once each has been answered.
        #[test]
        fn an_orphaned_handle_is_closed_exactly_once(
            requests in proptest::collection::vec(
                (proptest::bool::ANY, proptest::collection::vec(0u8..3, 0..6)),
                1..8,
            ),
        ) {
            let pending = Pending::default();
            let mut closes = Vec::new();
            let mut expected = Vec::new();
            for (n, (opens, steps)) in requests.into_iter().enumerate() {
                let handle = n as u64 + 100;
                let (id, rx) = pending.expect(opens).unwrap();
                let (mut replied, mut left, mut took) = (false, false, false);
                let steps = steps.into_iter().map(|s| [Step::Reply, Step::GiveUp, Step::Take][s as usize]);
                // Every request is answered in the end, as a live plugin answers all.
                for step in steps.chain([Step::Reply]) {
                    match step {
                        Step::Reply if !replied => {
                            replied = true;
                            let value = json!({ "handle": handle, "len": 0 });
                            closes.extend(pending.answer(&json!(id), Ok(value)));
                        }
                        Step::GiveUp if !left && !took => {
                            left = true;
                            closes.extend(pending.give_up(id, opens, &rx));
                        }
                        Step::Take if !left && !took => {
                            took = rx.try_recv().is_ok();
                        }
                        _ => {}
                    }
                }
                if opens && left {
                    expected.push(handle);
                }
                proptest::prop_assert!(!pending.table().waiting.contains_key(&id));
            }
            let mut closed: Vec<u64> = closes.iter().map(|line| closed_handle(line)).collect();
            closed.sort_unstable();
            proptest::prop_assert_eq!(closed, expected);
        }
    }

    #[test]
    fn a_cancelled_caller_stops_waiting_at_once_and_the_plugin_is_told() {
        let link = ServeLink::new();
        let mut fake = attached(&link);
        let token = shared::Cancel::new();
        let waiter = {
            let (link, token) = (Arc::clone(&link), token.clone());
            thread::spawn(move || {
                let _scope = token.enter();
                link.call(&list(), None)
            })
        };
        let sent = next_line(&mut fake);
        let started = std::time::Instant::now();
        token.cancel();
        assert_eq!(waiter.join().unwrap(), Err(PeerError::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(5));
        let cancel = next_line(&mut fake);
        assert_eq!(cancel["method"], plugin_api::METHOD_VFS_CANCEL);
        assert_eq!(cancel["params"]["id"], sent["id"]);
        // The late reply finds the request abandoned and asks for nothing back.
        assert_eq!(fake.pending.answer(&sent["id"], Ok(json!([]))), None);
    }

    #[test]
    fn a_job_already_given_up_sends_nothing() {
        let link = ServeLink::new();
        let mut fake = attached(&link);
        let token = shared::Cancel::new();
        token.cancel();
        let _scope = token.enter();
        assert_eq!(link.call(&list(), None), Err(PeerError::Cancelled));
        assert!(fake.lines.try_recv().is_err());
    }

    #[test]
    fn a_file_opened_after_its_caller_left_is_closed() {
        let link = ServeLink::new();
        let mut fake = attached(&link);
        let token = shared::Cancel::new();
        let waiter = {
            let (link, token) = (Arc::clone(&link), token.clone());
            thread::spawn(move || {
                let _scope = token.enter();
                link.call(&open(), None)
            })
        };
        let sent = next_line(&mut fake);
        token.cancel();
        assert_eq!(waiter.join().unwrap(), Err(PeerError::Cancelled));
        let close = fake
            .pending
            .answer(&sent["id"], Ok(json!({ "handle": 5, "len": 9 })))
            .expect("a close for the orphaned handle");
        let close: Value = serde_json::from_str(&close).unwrap();
        assert_eq!(close["params"], json!({ "op": "close", "handle": 5 }));
        // Whose own reply is expected by nobody.
        assert_eq!(fake.pending.answer(&close["id"], Ok(json!({}))), None);
    }

    #[test]
    fn a_file_opened_just_before_its_caller_left_is_closed_by_the_caller() {
        let link = ServeLink::new();
        let mut fake = attached(&link);
        let reply = link.submit(&open(), None).unwrap();
        let sent = next_line(&mut fake);
        // The reply lands in the caller's channel, and only then is the caller given up on.
        assert_eq!(
            fake.pending
                .answer(&sent["id"], Ok(json!({ "handle": 8, "len": 1 }))),
            None
        );
        let token = shared::Cancel::new();
        token.cancel();
        let _scope = token.enter();
        // A reply already there is taken, not thrown away: the caller got its file.
        assert!(reply.wait().is_ok());

        // The narrow case `recv` cannot see: the reply lands after its last look and before the
        // caller abandons the request.
        drop(_scope);
        let waiting = written(&link, &open());
        let sent = next_line(&mut fake);
        fake.pending
            .answer(&sent["id"], Ok(json!({ "handle": 9, "len": 1 })));
        waiting.give_up();
        let close = next_line(&mut fake);
        assert_eq!(close["params"], json!({ "op": "close", "handle": 9 }));
    }
}
