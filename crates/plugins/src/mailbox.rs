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

//! A plugin's mailbox for the two events that follow the user around, `dir_changed` and
//! `selection_changed`. It holds at most one of each, so whatever a plugin's speed, holding `j`
//! can never build a queue behind it: a change that arrives while one is still waiting is folded
//! into it, keeping where the burst began and where it has got to. Taking the events out is the
//! plugin writer's job; while the plugin is not reading, its stdin pipe fills, the writer waits on
//! it, and the mailbox keeps folding, so a stuck plugin costs two slots of memory.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use tokio::sync::Notify;

use crate::protocol::{EVENT_DIR_CHANGED, EVENT_SELECTION_CHANGED, HostEvent};

/// One waiting change: what moved, plus the directory and marks as they were when it last did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub from: Option<PathBuf>,
    pub to: Option<PathBuf>,
    pub cwd: PathBuf,
    pub marks: Vec<PathBuf>,
}

#[derive(Debug, Default)]
struct Pending {
    dir: Option<Queued>,
    selection: Option<Queued>,
}

/// Folds `new` into whatever `slot` already holds: the earlier `from` stays, the later everything
/// else wins. With `cancel_round_trip`, a burst that ended where it began leaves nothing to say
/// and empties the slot.
fn fold(slot: &mut Option<Queued>, mut new: Queued, cancel_round_trip: bool) {
    if let Some(old) = slot.take() {
        new.from = old.from;
    }
    if !(cancel_round_trip && new.from == new.to) {
        *slot = Some(new);
    }
}

#[derive(Debug, Default)]
pub struct Mailbox {
    pending: Mutex<Pending>,
    wake: Notify,
}

impl Mailbox {
    /// Leaves a `dir_changed` for the writer, folded into one already waiting.
    pub fn post_dir(&self, change: Queued) {
        fold(&mut self.lock().dir, change, true);
        self.wake.notify_one();
    }

    /// Leaves a `selection_changed` for the writer, folded into one already waiting. A change of
    /// marks alone has the same `from` and `to`, so no round trip is cancelled here.
    pub fn post_selection(&self, change: Queued) {
        fold(&mut self.lock().selection, change, false);
        self.wake.notify_one();
    }

    /// Resolves once something has been posted since the last wait (a post made while nobody was
    /// waiting is not lost).
    pub async fn posted(&self) {
        self.wake.notified().await;
    }

    /// Takes everything waiting, as ready-to-write lines: the directory change first.
    pub fn take_lines(&self) -> Vec<String> {
        let Pending { dir, selection } = std::mem::take(&mut *self.lock());
        [
            (EVENT_DIR_CHANGED, dir),
            (EVENT_SELECTION_CHANGED, selection),
        ]
        .into_iter()
        .filter_map(|(name, queued)| {
            let Queued {
                from,
                to,
                cwd,
                marks,
            } = queued?;
            Some(
                HostEvent::new(name, cwd, marks)
                    .with_change(from, to)
                    .to_line(),
            )
        })
        .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Pending> {
        // Nothing here can panic while holding the lock, and a poisoned one still holds a valid
        // `Pending`, so carry on rather than take the writer down.
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::Value;

    fn queued(from: &str, to: &str) -> Queued {
        Queued {
            from: Some(PathBuf::from(from)),
            to: Some(PathBuf::from(to)),
            cwd: PathBuf::from(to),
            marks: Vec::new(),
        }
    }

    fn parsed(lines: &[String]) -> Vec<Value> {
        lines
            .iter()
            .map(|line| serde_json::from_str(line.trim()).unwrap())
            .collect()
    }

    #[test]
    fn an_empty_mailbox_has_nothing_to_take() {
        assert!(Mailbox::default().take_lines().is_empty());
    }

    #[test]
    fn a_burst_becomes_one_event_from_where_it_began_to_where_it_ended() {
        let mailbox = Mailbox::default();
        mailbox.post_selection(queued("/a", "/b"));
        mailbox.post_selection(queued("/b", "/c"));
        mailbox.post_selection(queued("/c", "/d"));

        let events = parsed(&mailbox.take_lines());

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["params"]["name"], "selection_changed");
        assert_eq!(events[0]["params"]["from"], "/a");
        assert_eq!(events[0]["params"]["to"], "/d");
    }

    #[test]
    fn a_directory_burst_that_ends_where_it_began_says_nothing() {
        let mailbox = Mailbox::default();
        mailbox.post_dir(queued("/a", "/b"));
        mailbox.post_dir(queued("/b", "/a"));

        assert!(mailbox.take_lines().is_empty());
    }

    #[test]
    fn a_change_of_marks_alone_is_still_delivered() {
        let mailbox = Mailbox::default();
        mailbox.post_selection(Queued {
            from: Some(PathBuf::from("/a")),
            to: Some(PathBuf::from("/a")),
            cwd: PathBuf::from("/"),
            marks: vec![PathBuf::from("/a")],
        });

        let events = parsed(&mailbox.take_lines());

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["params"]["selection"][0], "/a");
    }

    #[test]
    fn the_directory_change_is_written_before_the_selection_change() {
        let mailbox = Mailbox::default();
        mailbox.post_selection(queued("/a/x", "/b/y"));
        mailbox.post_dir(queued("/a", "/b"));

        let names: Vec<_> = parsed(&mailbox.take_lines())
            .iter()
            .map(|event| event["params"]["name"].as_str().unwrap().to_string())
            .collect();

        assert_eq!(names, ["dir_changed", "selection_changed"]);
    }

    #[test]
    fn taking_empties_the_mailbox() {
        let mailbox = Mailbox::default();
        mailbox.post_dir(queued("/a", "/b"));
        assert_eq!(mailbox.take_lines().len(), 1);
        assert!(mailbox.take_lines().is_empty());
    }

    #[tokio::test]
    async fn a_post_made_before_anyone_waits_is_not_lost() {
        let mailbox = Mailbox::default();
        mailbox.post_dir(queued("/a", "/b"));
        tokio::time::timeout(std::time::Duration::from_secs(1), mailbox.posted())
            .await
            .expect("the earlier post should still wake a waiter");
    }

    proptest! {
        /// However many changes pile up, a plugin never has more than one event of each kind
        /// waiting, it starts where the first began and ends where the last arrived.
        #[test]
        fn any_run_of_selection_changes_folds_into_one(
            steps in proptest::collection::vec("[a-z]{1,6}", 1..200),
        ) {
            let mailbox = Mailbox::default();
            let mut at = "start".to_string();
            for step in &steps {
                mailbox.post_selection(queued(&at, step));
                at.clone_from(step);
            }

            let events = parsed(&mailbox.take_lines());

            prop_assert_eq!(events.len(), 1);
            prop_assert_eq!(events[0]["params"]["from"].as_str(), Some("start"));
            prop_assert_eq!(events[0]["params"]["to"].as_str(), Some(at.as_str()));
        }

        #[test]
        fn any_run_of_directory_changes_leaves_at_most_one_event_that_really_moved(
            steps in proptest::collection::vec("[a-c]", 1..200),
        ) {
            let mailbox = Mailbox::default();
            let mut at = "a".to_string();
            for step in &steps {
                if *step != at {
                    mailbox.post_dir(queued(&at, step));
                    at.clone_from(step);
                }
            }

            let events = parsed(&mailbox.take_lines());

            prop_assert!(events.len() <= 1);
            if let Some(event) = events.first() {
                prop_assert_eq!(event["params"]["from"].as_str(), Some("a"));
                prop_assert_eq!(event["params"]["to"].as_str(), Some(at.as_str()));
                prop_assert_ne!(&event["params"]["from"], &event["params"]["to"]);
            } else {
                prop_assert_eq!(at, "a");
            }
        }
    }
}
