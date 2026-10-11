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

//! What happens to a plugin whose process has ended. A plugin that exits is not an error in
//! itself (a script that does its job on a key press and quits is a fine plugin), so the host does
//! not watch for deaths: it looks the next time something is meant for the plugin, and starts it
//! again if it is gone. A plugin that keeps dying straight away is a different matter, and gets
//! three tries before the host stops starting it until the user says `:plugins restart <name>`.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

/// How many crashes in a row, each soon after starting, a plugin gets before the host gives up.
pub const MAX_QUICK_CRASHES: u32 = 3;

/// A process that ran this long before crashing was working, so its crash starts the count again
/// rather than adding to it.
pub const STABLE_FOR: Duration = Duration::from_secs(60);

/// How one plugin process ended, as `reap` records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    Running,
    /// Exited with status 0: it finished, which is allowed.
    Finished,
    Crashed {
        /// The exit status as the system words it (`exit status: 1`, `signal: 11`).
        status: String,
        ran_for: Duration,
    },
}

/// What the host decides when it finds a plugin's process gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Start it again; `crashes` is the quick-crash count to carry forward.
    Respawn { crashes: u32 },
    /// Stop starting it.
    GiveUp { crashes: u32 },
}

/// The verdict on a crash that came after `ran_for`, given `crashes` quick ones in a row before.
pub fn after_crash(crashes: u32, ran_for: Duration) -> Verdict {
    let crashes = if ran_for >= STABLE_FOR {
        1
    } else {
        crashes.saturating_add(1)
    };
    if crashes >= MAX_QUICK_CRASHES {
        Verdict::GiveUp { crashes }
    } else {
        Verdict::Respawn { crashes }
    }
}

/// What the host and the task that waits on a process share about it.
#[derive(Debug)]
pub struct Life {
    exit: Mutex<Exit>,
    stop: Notify,
    stopped_by_host: AtomicBool,
    started: Instant,
}

impl Life {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            exit: Mutex::new(Exit::Running),
            stop: Notify::new(),
            stopped_by_host: AtomicBool::new(false),
            started: Instant::now(),
        })
    }

    pub fn exit(&self) -> Exit {
        self.exit
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_exit(&self, exit: Exit) {
        *self.exit.lock().unwrap_or_else(PoisonError::into_inner) = exit;
    }

    /// How long the process has been alive, for the crash it may be about to have.
    pub fn age(&self) -> Duration {
        self.started.elapsed()
    }

    /// Asks the task waiting on the process to kill it, for a restart; its end is then not worth
    /// a message.
    pub fn stop(&self) {
        self.stopped_by_host.store(true, Ordering::Relaxed);
        self.stop.notify_one();
    }

    pub async fn stop_requested(&self) {
        self.stop.notified().await;
    }

    pub fn was_stopped_by_host(&self) -> bool {
        self.stopped_by_host.load(Ordering::Relaxed)
    }
}

/// Where a plugin stands, as `:plugins` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    Running,
    /// Exited with status 0; it starts again the next time something is meant for it.
    Finished,
    /// Crashed; it starts again the next time something is meant for it.
    Crashed(String),
    /// Crashed too often in a row; only `:plugins restart` starts it again.
    Stopped(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInfo {
    pub name: String,
    pub state: PluginState,
    /// How many times the host started it again.
    pub restarts: u32,
}

impl fmt::Display for PluginInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.name)?;
        match &self.state {
            PluginState::Running => f.write_str("running")?,
            PluginState::Finished => f.write_str("finished")?,
            PluginState::Crashed(status) => write!(f, "crashed ({status})")?,
            PluginState::Stopped(status) => write!(f, "stopped after repeated crashes ({status})")?,
        }
        match self.restarts {
            0 => Ok(()),
            n => write!(f, ", restarted {n}x"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const QUICK: Duration = Duration::from_millis(50);

    #[test]
    fn quick_crashes_are_forgiven_twice_and_the_third_gives_up() {
        assert_eq!(after_crash(0, QUICK), Verdict::Respawn { crashes: 1 });
        assert_eq!(after_crash(1, QUICK), Verdict::Respawn { crashes: 2 });
        assert_eq!(after_crash(2, QUICK), Verdict::GiveUp { crashes: 3 });
    }

    #[test]
    fn a_crash_after_a_long_run_starts_the_count_again() {
        assert_eq!(
            after_crash(2, STABLE_FOR),
            Verdict::Respawn { crashes: 1 },
            "a plugin that worked for a minute is not in a crash loop"
        );
        assert_eq!(
            after_crash(2, STABLE_FOR - Duration::from_millis(1)),
            Verdict::GiveUp { crashes: 3 }
        );
    }

    #[test]
    fn a_new_life_is_running_until_told_otherwise() {
        let life = Life::new();
        assert_eq!(life.exit(), Exit::Running);
        assert!(!life.was_stopped_by_host());

        life.set_exit(Exit::Finished);
        assert_eq!(life.exit(), Exit::Finished);
    }

    #[tokio::test]
    async fn a_stop_asked_for_before_anyone_waits_is_not_lost() {
        let life = Life::new();
        life.stop();
        assert!(life.was_stopped_by_host());
        tokio::time::timeout(Duration::from_secs(1), life.stop_requested())
            .await
            .expect("the earlier stop should still wake a waiter");
    }

    #[test]
    fn the_list_words_each_state_and_counts_restarts() {
        let info = |state, restarts| PluginInfo {
            name: "p".into(),
            state,
            restarts,
        };
        assert_eq!(info(PluginState::Running, 0).to_string(), "p: running");
        assert_eq!(
            info(PluginState::Finished, 2).to_string(),
            "p: finished, restarted 2x"
        );
        assert_eq!(
            info(PluginState::Crashed("exit status: 1".into()), 0).to_string(),
            "p: crashed (exit status: 1)"
        );
        assert_eq!(
            info(PluginState::Stopped("signal: 11".into()), 1).to_string(),
            "p: stopped after repeated crashes (signal: 11), restarted 1x"
        );
    }

    proptest! {
        /// However the crashes are spaced, a run of quick ones is always cut off at the limit, and
        /// the count never reaches the limit without the verdict saying so.
        #[test]
        fn a_run_of_quick_crashes_always_ends_at_the_limit(
            ran_ms in proptest::collection::vec(0u64..59_000, 1..20),
        ) {
            let mut crashes = 0;
            let mut tries = 0;
            for ms in ran_ms {
                tries += 1;
                match after_crash(crashes, Duration::from_millis(ms)) {
                    Verdict::Respawn { crashes: next } => {
                        prop_assert!(next < MAX_QUICK_CRASHES);
                        crashes = next;
                    }
                    Verdict::GiveUp { crashes: at } => {
                        prop_assert_eq!(at, MAX_QUICK_CRASHES);
                        prop_assert_eq!(tries, MAX_QUICK_CRASHES);
                        return Ok(());
                    }
                }
            }
            prop_assert!(tries < MAX_QUICK_CRASHES);
        }

        #[test]
        fn a_long_run_always_leaves_room_for_more_tries(
            before in 0u32..100,
            ran_secs in 60u64..100_000,
        ) {
            prop_assert_eq!(
                after_crash(before, Duration::from_secs(ran_secs)),
                Verdict::Respawn { crashes: 1 }
            );
        }
    }
}
