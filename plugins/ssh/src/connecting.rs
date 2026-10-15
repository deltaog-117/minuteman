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

//! One machine's session, and the one attempt at making it that every request needing it shares.
//!
//! Reaching a host can take its whole timeout. The attempt runs as a task of its own rather than
//! on the thread of the request that started it, so a request that is given up leaves without
//! holding anyone else up, and a request arriving meanwhile joins the attempt instead of starting
//! a second one.
//!
//! Model. The slot is `Idle`, `Live(s)` or `Connecting { attempt, waiters }` with `waiters >= 1`.
//! `get` on `Idle` starts an attempt with one waiter; on `Connecting` it adds a waiter; on `Live`
//! it returns the session. When the attempt ends, every waiter sees the same outcome and the first
//! to look moves the slot to `Live` (or back to `Idle` on a failure). A waiter whose job is given
//! up removes itself; the last one to leave aborts the attempt and empties the slot. Every change
//! is made under the slot's lock, which gives the invariants: at most one attempt runs per
//! machine, an attempt is never aborted while anyone waits for it, and an aborted attempt is never
//! installed (the slot no longer names it).

use std::future::Future;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use shared::cancel;
use tokio::task::AbortHandle;

use crate::executor::Executor;

/// Why `get` has no session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Failed {
    /// The attempt failed, for the reason given.
    Refused(String),
    /// The caller's job was given up while it waited.
    Cancelled,
    /// The runtime is gone, or the attempt panicked.
    Lost,
}

/// One attempt and what it came to.
struct Attempt<T> {
    outcome: Mutex<Option<Result<Arc<T>, Failed>>>,
    done: Condvar,
    task: Mutex<Option<AbortHandle>>,
}

impl<T> Attempt<T> {
    fn new() -> Self {
        Self {
            outcome: Mutex::new(None),
            done: Condvar::new(),
            task: Mutex::new(None),
        }
    }

    /// Records `outcome` unless one is already there, and wakes everyone waiting.
    fn finish(&self, outcome: Result<Arc<T>, Failed>) {
        let mut slot = self.outcome.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(outcome);
        }
        self.done.notify_all();
    }

    fn abort(&self) {
        if let Some(task) = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            task.abort();
        }
    }
}

/// Ends an attempt that stops without finishing (a panic), so no waiter waits forever.
struct Finisher<T>(Arc<Attempt<T>>);

impl<T> Drop for Finisher<T> {
    fn drop(&mut self) {
        self.0.finish(Err(Failed::Lost));
    }
}

enum State<T> {
    Idle,
    Live(Arc<T>),
    Connecting {
        attempt: Arc<Attempt<T>>,
        waiters: usize,
    },
}

/// See the module documentation.
pub(crate) struct Slot<T> {
    state: Mutex<State<T>>,
}

impl<T: Send + Sync + 'static> Slot<T> {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(State::Idle),
        }
    }

    fn state(&self) -> MutexGuard<'_, State<T>> {
        // Nothing panics while holding it, and every state is a valid one.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The live session, or the outcome of the attempt to make one, joining the attempt already
    /// running if there is one. `connect` is called only when a new attempt starts.
    pub(crate) fn get<F, Fut>(&self, executor: &Executor, connect: F) -> Result<Arc<T>, Failed>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, String>> + Send + 'static,
    {
        let attempt = {
            let mut state = self.state();
            match &mut *state {
                State::Live(session) => return Ok(Arc::clone(session)),
                State::Connecting { attempt, waiters } => {
                    *waiters += 1;
                    Arc::clone(attempt)
                }
                State::Idle => {
                    let attempt = Arc::new(Attempt::new());
                    let finisher = Finisher(Arc::clone(&attempt));
                    let work = connect();
                    let task = executor
                        .spawn(async move {
                            let outcome = work.await.map(Arc::new).map_err(Failed::Refused);
                            finisher.0.finish(outcome);
                        })
                        .map_err(|_| Failed::Lost)?;
                    *attempt.task.lock().unwrap_or_else(PoisonError::into_inner) = Some(task);
                    *state = State::Connecting {
                        attempt: Arc::clone(&attempt),
                        waiters: 1,
                    };
                    attempt
                }
            }
        };
        let outcome =
            cancel::wait_until(&attempt.outcome, &attempt.done, |outcome| outcome.clone());
        let mut state = self.state();
        let current = matches!(
            &*state,
            State::Connecting { attempt: running, .. } if Arc::ptr_eq(running, &attempt)
        );
        match outcome {
            None => {
                if let State::Connecting { waiters, .. } = &mut *state
                    && current
                {
                    *waiters -= 1;
                    if *waiters == 0 {
                        attempt.abort();
                        *state = State::Idle;
                    }
                }
                Err(Failed::Cancelled)
            }
            Some(Ok(session)) => {
                if current {
                    *state = State::Live(Arc::clone(&session));
                }
                Ok(session)
            }
            Some(Err(failed)) => {
                if current {
                    *state = State::Idle;
                }
                Err(failed)
            }
        }
    }

    /// Forgets `dead` so the next `get` connects afresh, unless a newer session already replaced
    /// it.
    pub(crate) fn forget(&self, dead: &Arc<T>) {
        let mut state = self.state();
        if matches!(&*state, State::Live(live) if Arc::ptr_eq(live, dead)) {
            *state = State::Idle;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use shared::Cancel;

    use super::*;

    /// A stand-in for making a session: counts its starts, takes `delay`, and marks its drop.
    struct Fixture {
        executor: Executor,
        slot: Slot<u32>,
        starts: Arc<AtomicUsize>,
        dropped: Arc<AtomicBool>,
    }

    struct MarkDrop(Arc<AtomicBool>);

    impl Drop for MarkDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    impl Fixture {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                executor: Executor::new().unwrap(),
                slot: Slot::new(),
                starts: Arc::new(AtomicUsize::new(0)),
                dropped: Arc::new(AtomicBool::new(false)),
            })
        }

        fn get(&self, delay: Duration, outcome: Result<u32, String>) -> Result<Arc<u32>, Failed> {
            let (starts, dropped) = (Arc::clone(&self.starts), Arc::clone(&self.dropped));
            self.slot.get(&self.executor, move || {
                let n = starts.fetch_add(1, Ordering::Relaxed);
                let mark = MarkDrop(dropped);
                async move {
                    let _mark = mark;
                    tokio::time::sleep(delay).await;
                    outcome.map(|value| value + n as u32)
                }
            })
        }

        fn starts(&self) -> usize {
            self.starts.load(Ordering::Relaxed)
        }

        fn wait_for_drop(&self) -> bool {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !self.dropped.load(Ordering::Relaxed) && Instant::now() < deadline {
                thread::yield_now();
            }
            self.dropped.load(Ordering::Relaxed)
        }
    }

    /// Calls `get` on a new thread whose job is `token`.
    fn get_on_thread(
        fixture: &Arc<Fixture>,
        token: Cancel,
        delay: Duration,
    ) -> thread::JoinHandle<Result<Arc<u32>, Failed>> {
        let fixture = Arc::clone(fixture);
        thread::spawn(move || {
            let _scope = token.enter();
            fixture.get(delay, Ok(10))
        })
    }

    /// Waits until `fixture` has started `n` attempts.
    fn until_started(fixture: &Fixture, n: usize) {
        while fixture.starts() < n {
            thread::yield_now();
        }
    }

    #[test]
    fn requests_arriving_together_share_one_attempt_and_one_session() {
        let fixture = Fixture::new();
        let first = get_on_thread(&fixture, Cancel::new(), Duration::from_millis(100));
        until_started(&fixture, 1);
        let second = get_on_thread(&fixture, Cancel::new(), Duration::from_millis(100));
        let (a, b) = (
            first.join().unwrap().unwrap(),
            second.join().unwrap().unwrap(),
        );
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(fixture.starts(), 1);
        // Live now: no new attempt.
        assert!(Arc::ptr_eq(
            &fixture.get(Duration::ZERO, Ok(0)).unwrap(),
            &a
        ));
        assert_eq!(fixture.starts(), 1);
    }

    #[test]
    fn a_failure_reaches_the_caller_and_the_next_request_tries_again() {
        let fixture = Fixture::new();
        assert_eq!(
            fixture.get(Duration::ZERO, Err("no route".into())),
            Err(Failed::Refused("no route".into()))
        );
        assert_eq!(*fixture.get(Duration::ZERO, Ok(1)).unwrap(), 2);
        assert_eq!(fixture.starts(), 2);
    }

    #[test]
    fn the_last_waiter_giving_up_aborts_the_attempt_at_once() {
        let fixture = Fixture::new();
        let token = Cancel::new();
        let waiter = get_on_thread(&fixture, token.clone(), Duration::from_secs(60));
        until_started(&fixture, 1);
        let started = Instant::now();
        token.cancel();
        assert_eq!(waiter.join().unwrap(), Err(Failed::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(fixture.wait_for_drop(), "the attempt was aborted");
        // The slot is empty again, so the next request makes a new attempt.
        assert_eq!(*fixture.get(Duration::ZERO, Ok(1)).unwrap(), 2);
    }

    #[test]
    fn an_attempt_someone_still_waits_for_survives_another_giving_up() {
        let fixture = Fixture::new();
        let leaving = Cancel::new();
        let first = get_on_thread(&fixture, leaving.clone(), Duration::from_millis(300));
        until_started(&fixture, 1);
        let staying = get_on_thread(&fixture, Cancel::new(), Duration::ZERO);
        thread::sleep(Duration::from_millis(50));
        leaving.cancel();
        assert_eq!(first.join().unwrap(), Err(Failed::Cancelled));
        assert_eq!(*staying.join().unwrap().unwrap(), 10);
        assert_eq!(fixture.starts(), 1);
    }

    #[test]
    fn a_forgotten_session_is_replaced_and_a_stale_one_is_not_forgotten() {
        let fixture = Fixture::new();
        let old = fixture.get(Duration::ZERO, Ok(0)).unwrap();
        fixture.slot.forget(&old);
        let new = fixture.get(Duration::ZERO, Ok(0)).unwrap();
        assert!(!Arc::ptr_eq(&old, &new));
        fixture.slot.forget(&old);
        assert!(Arc::ptr_eq(
            &fixture.get(Duration::ZERO, Ok(0)).unwrap(),
            &new
        ));
    }

    #[test]
    fn an_attempt_that_panics_is_reported_not_waited_for_forever() {
        let fixture = Fixture::new();
        let outcome = fixture.slot.get(&fixture.executor, || async {
            panic!("boom");
        });
        assert_eq!(outcome, Err(Failed::Lost));
    }
}
