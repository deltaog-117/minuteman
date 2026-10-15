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

//! Giving up on work that a background thread is waiting for: a folder being opened on another
//! machine after `Esc`, a listing of a folder the cursor has already left.
//!
//! The `Vfs` methods take no token, so the one that applies travels with the thread instead: the
//! job that may be given up [`enter`](Cancel::enter)s its `Cancel` on the thread that does the
//! work, and whatever deep inside waits on a slow answer (the request to a plugin, the plugin's
//! wait on a connection) asks [`cancelled`] or waits through [`recv`] or [`wait_until`], which
//! give up within [`POLL`] of the token being set. A thread with no scope entered waits as it
//! always did, so nothing changes for a caller that never cancels.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// How often a wait looks at its token. Short enough that `Esc` feels immediate, long enough that
/// a waiting thread costs nothing measurable.
pub const POLL: Duration = Duration::from_millis(20);

/// A flag shared by a job and whoever may give up on it. Clones are the same flag.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

thread_local! {
    static CURRENT: RefCell<Option<Cancel>> = const { RefCell::new(None) };
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Gives up on the job. It cannot be taken back.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// The flag itself, for code that already takes one (the search walk).
    pub fn flag(&self) -> &AtomicBool {
        &self.0
    }

    /// Makes this the token of the calling thread until the returned scope is dropped, when the
    /// one before it (if any) applies again.
    #[must_use = "the token applies only while the scope is alive"]
    pub fn enter(&self) -> Scope {
        let previous = CURRENT.with(|current| current.replace(Some(self.clone())));
        Scope {
            previous,
            _same_thread: PhantomData,
        }
    }
}

/// The time a [`Cancel`] is the calling thread's token. It restores the one before it on drop,
/// on the thread that entered it, which is why it cannot be sent elsewhere.
pub struct Scope {
    previous: Option<Cancel>,
    _same_thread: PhantomData<*const ()>,
}

impl Drop for Scope {
    fn drop(&mut self) {
        let previous = self.previous.take();
        CURRENT.with(|current| *current.borrow_mut() = previous);
    }
}

/// The calling thread's token, if a job entered one.
pub fn current() -> Option<Cancel> {
    CURRENT.with(|current| current.borrow().clone())
}

/// Whether the calling thread's job has been given up on.
pub fn cancelled() -> bool {
    CURRENT.with(|current| current.borrow().as_ref().is_some_and(Cancel::is_cancelled))
}

/// How a [`recv`] ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Waited<T> {
    Got(T),
    TimedOut,
    /// The calling thread's token was set. Nothing was taken from the channel.
    Cancelled,
    /// The sender is gone without sending.
    Disconnected,
}

/// Receives from `rx` within `timeout`, giving up early if the calling thread's token is set. A
/// value already waiting is taken even then, so a job that finished as it was cancelled is not
/// thrown away for nothing.
pub fn recv<T>(rx: &Receiver<T>, timeout: Duration) -> Waited<T> {
    let deadline = Instant::now().checked_add(timeout);
    let Some(token) = current() else {
        return match deadline {
            Some(_) => match rx.recv_timeout(timeout) {
                Ok(value) => Waited::Got(value),
                Err(RecvTimeoutError::Timeout) => Waited::TimedOut,
                Err(RecvTimeoutError::Disconnected) => Waited::Disconnected,
            },
            None => rx.recv().map_or(Waited::Disconnected, Waited::Got),
        };
    };
    loop {
        match rx.try_recv() {
            Ok(value) => return Waited::Got(value),
            Err(TryRecvError::Disconnected) => return Waited::Disconnected,
            Err(TryRecvError::Empty) => {}
        }
        if token.is_cancelled() {
            return Waited::Cancelled;
        }
        let slice = match deadline {
            Some(deadline) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Waited::TimedOut;
                }
                left.min(POLL)
            }
            None => POLL,
        };
        match rx.recv_timeout(slice) {
            Ok(value) => return Waited::Got(value),
            Err(RecvTimeoutError::Disconnected) => return Waited::Disconnected,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Waits on `done` until `ready` returns something, or `None` once the calling thread's token is
/// set. `ready` runs with `lock` held, as a condition variable's check must.
pub fn wait_until<T, R>(
    lock: &Mutex<T>,
    done: &Condvar,
    mut ready: impl FnMut(&mut T) -> Option<R>,
) -> Option<R> {
    let token = current();
    let mut guard: MutexGuard<'_, T> = lock.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        if let Some(value) = ready(&mut guard) {
            return Some(value);
        }
        guard = match &token {
            Some(token) if token.is_cancelled() => return None,
            Some(_) => {
                done.wait_timeout(guard, POLL)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
            None => done.wait(guard).unwrap_or_else(PoisonError::into_inner),
        };
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;
    use std::thread;

    use proptest::prelude::*;

    use super::*;

    #[test]
    fn a_thread_with_no_scope_has_no_token_and_is_never_cancelled() {
        assert!(current().is_none());
        assert!(!cancelled());
    }

    #[test]
    fn a_scope_applies_until_it_is_dropped() {
        let token = Cancel::new();
        {
            let _scope = token.enter();
            assert!(!cancelled());
            token.cancel();
            assert!(cancelled());
        }
        assert!(!cancelled());
    }

    #[test]
    fn a_token_belongs_to_the_thread_that_entered_it() {
        let token = Cancel::new();
        token.cancel();
        let _scope = token.enter();
        assert!(thread::spawn(|| !cancelled()).join().unwrap());
    }

    #[test]
    fn a_value_already_sent_is_received_even_when_cancelled() {
        let (tx, rx) = channel();
        tx.send(7).unwrap();
        let token = Cancel::new();
        token.cancel();
        let _scope = token.enter();
        assert_eq!(recv(&rx, Duration::from_secs(5)), Waited::Got(7));
    }

    #[test]
    fn a_cancel_from_another_thread_ends_the_wait_long_before_its_timeout() {
        let (_tx, rx) = channel::<()>();
        let token = Cancel::new();
        let canceller = token.clone();
        let _scope = token.enter();
        let started = Instant::now();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            canceller.cancel();
        });
        assert_eq!(recv(&rx, Duration::from_secs(30)), Waited::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_wait_with_a_token_still_times_out() {
        let (_tx, rx) = channel::<()>();
        let _scope = Cancel::new().enter();
        assert_eq!(recv(&rx, Duration::from_millis(30)), Waited::TimedOut);
    }

    #[test]
    fn a_wait_ends_when_the_sender_goes() {
        let (tx, rx) = channel::<()>();
        drop(tx);
        let _scope = Cancel::new().enter();
        assert_eq!(recv(&rx, Duration::from_secs(5)), Waited::Disconnected);
        assert_eq!(recv(&rx, Duration::MAX), Waited::Disconnected);
    }

    #[test]
    fn a_condition_wait_returns_what_is_ready_or_gives_up_when_cancelled() {
        let state = Arc::new((Mutex::new(None::<u8>), Condvar::new()));
        let setter = Arc::clone(&state);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            *setter.0.lock().unwrap() = Some(3);
            setter.1.notify_all();
        });
        assert_eq!(wait_until(&state.0, &state.1, |value| *value), Some(3));

        let never = (Mutex::new(None::<u8>), Condvar::new());
        let token = Cancel::new();
        let _scope = token.enter();
        token.cancel();
        assert_eq!(wait_until(&never.0, &never.1, |value| *value), None);
    }

    proptest! {
        /// However scopes nest, the token in force is the innermost one still alive, and leaving
        /// them all leaves none.
        #[test]
        fn nested_scopes_restore_the_one_outside(depth in 0usize..12, cancelled_at in proptest::collection::vec(any::<bool>(), 12)) {
            let tokens: Vec<Cancel> = (0..depth).map(|_| Cancel::new()).collect();
            let mut scopes = Vec::new();
            for (token, cancel) in tokens.iter().zip(&cancelled_at) {
                if *cancel {
                    token.cancel();
                }
                scopes.push(token.enter());
                prop_assert_eq!(cancelled(), *cancel);
            }
            while let Some(scope) = scopes.pop() {
                drop(scope);
                let expected = scopes.len().checked_sub(1).map(|i| cancelled_at[i]).unwrap_or(false);
                prop_assert_eq!(cancelled(), expected);
            }
            prop_assert!(current().is_none());
        }
    }
}
