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

//! The search's state and how it answers each thing the host tells it, with no input or output of
//! its own: `apply` takes a `Command` and pushes the lines it wants written, which is what lets
//! it be tested without a process on the other end.
//!
//! One search is open at a time. Opening it picks fresh numbers for the picker and the walk, asks
//! the host for both (the walk with sizes and times, which the filters need), and from then on
//! only listens to events that carry those numbers, so a late batch from an earlier walk can
//! never end up in this one's index.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use plugin_api::client::{notification_line, request_line};
use plugin_api::{
    METHOD_PICKER_ITEMS, METHOD_SHOW_PICKER, METHOD_WALK, METHOD_WALK_CANCEL, PickerIndexing,
    PickerSpec, PickerUpdate, TreeEntry, WalkCancel, WalkEnd, WalkOutcome, WalkRequest,
};
use serde_json::json;

use crate::query::Search;
use crate::scan::Scan;

/// The name of the action this plugin provides, as the host's keymap spells it.
pub const ACTION: &str = "locate";

/// What the host tells the search, already read from its line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// The user triggered `locate` in `root`.
    Open { root: PathBuf, show_hidden: bool },
    /// What is typed into the picker changed.
    Query {
        picker: u64,
        generation: u64,
        text: String,
    },
    /// The picker was closed, or a result was chosen.
    Closed { picker: u64 },
    /// More entries of a walk.
    Batch { walk: u64, entries: Vec<TreeEntry> },
    /// A walk ended.
    Walked { walk: u64, end: WalkEnd },
    /// The answer to a request this plugin sent.
    Replied { id: u64, error: Option<String> },
    /// The host speaks a protocol version this plugin was not built for.
    Incompatible { host: u32 },
}

/// What a request in flight was for, to know what a refusal of it means.
#[derive(Debug, Clone, Copy)]
enum Pending {
    Picker(u64),
    Walk(u64),
}

/// The search while it is open: what the walk has found and which of it fits what is typed.
struct Open {
    picker: u64,
    walk: u64,
    index: Vec<TreeEntry>,
    scan: Scan,
    /// Which query the scan is for; the host keeps older results on screen until it sees it.
    generation: u64,
    walked: Option<WalkEnd>,
    /// The words of the last query that could not be read, so each is reported once rather than on
    /// every letter typed after it.
    reported: Vec<String>,
}

pub struct Session {
    /// Numbers for pickers and walks, which never repeat within a run.
    numbers: u64,
    requests: u64,
    pending: HashMap<u64, Pending>,
    open: Option<Open>,
    /// Seconds since the epoch, for ages; a field so a test can fix the time.
    clock: fn() -> u64,
}

impl Default for Session {
    fn default() -> Self {
        Self::with_clock(epoch_now)
    }
}

/// The time now, as seconds since the epoch (zero if the clock is set before it).
fn epoch_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

impl Session {
    pub fn with_clock(clock: fn() -> u64) -> Self {
        Self {
            numbers: 0,
            requests: 0,
            pending: HashMap::new(),
            open: None,
            clock,
        }
    }

    fn number(&mut self) -> u64 {
        self.numbers += 1;
        self.numbers
    }

    fn request(&mut self, what: Pending) -> u64 {
        self.requests += 1;
        self.pending.insert(self.requests, what);
        self.requests
    }

    /// Takes in `command`, pushing the lines to write onto `out`. Returns whether it changed the
    /// query (or opened the search), so the caller sends the first answer to it at once.
    pub fn apply(&mut self, command: Command, out: &mut Vec<String>) -> bool {
        match command {
            Command::Open { root, show_hidden } => {
                self.close(out);
                let (picker, walk) = (self.number(), self.number());
                let spec = PickerSpec {
                    picker,
                    title: "locate".to_string(),
                    preview: true,
                };
                let id = self.request(Pending::Picker(picker));
                out.push(request_line(id, METHOD_SHOW_PICKER, &spec));
                let request = WalkRequest {
                    walk,
                    root,
                    show_hidden,
                    metadata: true,
                };
                let id = self.request(Pending::Walk(walk));
                out.push(request_line(id, METHOD_WALK, &request));
                let mut scan = Scan::default();
                scan.set_query(Search::default(), (self.clock)());
                self.open = Some(Open {
                    picker,
                    walk,
                    index: Vec::new(),
                    scan,
                    generation: 0,
                    walked: None,
                    reported: Vec::new(),
                });
                true
            }
            Command::Query {
                picker,
                generation,
                text,
            } => {
                let now = (self.clock)();
                match self.open.as_mut() {
                    Some(open) if open.picker == picker => {
                        open.generation = generation;
                        let parsed = Search::parse(&text);
                        if parsed.ignored != open.reported {
                            if !parsed.ignored.is_empty() {
                                out.push(log_line(&format!(
                                    "locate: ignoring {}",
                                    parsed.ignored.join(" ")
                                )));
                            }
                            open.reported = parsed.ignored;
                        }
                        open.scan.set_query(parsed.search, now);
                        true
                    }
                    _ => false,
                }
            }
            Command::Closed { picker } => {
                if self.open.as_ref().is_some_and(|open| open.picker == picker) {
                    self.close(out);
                }
                false
            }
            Command::Batch { walk, entries } => {
                if let Some(open) = self.open.as_mut().filter(|open| open.walk == walk) {
                    open.index.extend(entries);
                }
                false
            }
            Command::Walked { walk, end } => {
                if let Some(open) = self.open.as_mut().filter(|open| open.walk == walk) {
                    open.walked = Some(end);
                }
                false
            }
            Command::Replied { id, error } => {
                if let (Some(what), Some(reason)) = (self.pending.remove(&id), error) {
                    self.refused(what, &reason, out);
                }
                false
            }
            // The caller stops on this before it gets here.
            Command::Incompatible { .. } => false,
        }
    }

    /// Stops the walk behind the open search and forgets its index.
    fn close(&mut self, out: &mut Vec<String>) {
        if let Some(open) = self.open.take() {
            out.push(notification_line(
                METHOD_WALK_CANCEL,
                &WalkCancel { walk: open.walk },
            ));
        }
    }

    /// The host said no to a request: say so in the status bar, and put the search away if the
    /// picker was what it refused.
    fn refused(&mut self, what: Pending, reason: &str, out: &mut Vec<String>) {
        out.push(log_line(&format!("locate: {reason}")));
        match what {
            Pending::Picker(picker) => {
                if self.open.as_ref().is_some_and(|open| open.picker == picker) {
                    self.close(out);
                }
            }
            // Without a walk there is nothing more to wait for; the picker stays usable.
            Pending::Walk(walk) => {
                if let Some(open) = self.open.as_mut().filter(|open| open.walk == walk) {
                    open.walked = Some(WalkEnd {
                        outcome: WalkOutcome::Done,
                        truncated: false,
                    });
                }
            }
        }
    }

    /// Checks at most `budget` entries toward the current query and says whether it has caught up
    /// with the whole index (trivially so while the search is closed).
    pub fn advance(&mut self, budget: usize) -> bool {
        match self.open.as_mut() {
            Some(open) => open.scan.advance(&open.index, budget),
            None => true,
        }
    }

    /// The `picker_items` line for the matches so far, or `None` with the search closed. `settled`
    /// says whether every indexed entry has been checked.
    pub fn update(&self, settled: bool) -> Option<String> {
        let open = self.open.as_ref()?;
        let indexing = match open.walked {
            None => PickerIndexing::Walking,
            Some(WalkEnd {
                outcome: WalkOutcome::Done,
                truncated: true,
            }) => PickerIndexing::Truncated,
            Some(_) => PickerIndexing::Done,
        };
        let update = PickerUpdate {
            picker: open.picker,
            generation: open.generation,
            items: open
                .scan
                .hits()
                .iter()
                .map(|&position| open.index[position].clone())
                .collect(),
            matches: open.scan.matches(),
            indexed: open.index.len(),
            indexing,
            settled,
        };
        Some(notification_line(METHOD_PICKER_ITEMS, &update))
    }
}

/// A `log` notification: the host shows it in the status bar.
pub fn log_line(message: &str) -> String {
    notification_line("log", &json!({ "level": "warn", "message": message }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const NOW: u64 = 100_000_000;

    fn session() -> Session {
        Session::with_clock(|| NOW)
    }

    fn entry(rel: &str, size: u64, modified: u64) -> TreeEntry {
        TreeEntry {
            path: PathBuf::from("/root").join(rel),
            rel: rel.to_string(),
            is_dir: false,
            size: Some(size),
            modified: Some(modified),
        }
    }

    fn lines(out: &[String]) -> Vec<Value> {
        out.iter()
            .map(|line| serde_json::from_str(line.trim()).unwrap())
            .collect()
    }

    fn open(session: &mut Session) -> (u64, u64, Vec<Value>) {
        let mut out = Vec::new();
        assert!(session.apply(
            Command::Open {
                root: PathBuf::from("/root"),
                show_hidden: true,
            },
            &mut out,
        ));
        let sent = lines(&out);
        let picker = sent[0]["params"]["picker"].as_u64().unwrap();
        let walk = sent[1]["params"]["walk"].as_u64().unwrap();
        (picker, walk, sent)
    }

    fn last_update(session: &Session, settled: bool) -> Value {
        serde_json::from_str(session.update(settled).unwrap().trim()).unwrap()
    }

    fn rels(update: &Value) -> Vec<String> {
        update["params"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["rel"].as_str().unwrap().to_string())
            .collect()
    }

    fn query(session: &mut Session, picker: u64, generation: u64, text: &str) -> Vec<String> {
        let mut out = Vec::new();
        assert!(session.apply(
            Command::Query {
                picker,
                generation,
                text: text.to_string(),
            },
            &mut out,
        ));
        session.advance(usize::MAX);
        out
    }

    #[test]
    fn opening_asks_for_the_picker_then_a_walk_that_carries_sizes_and_times() {
        let mut session = session();
        let (picker, walk, sent) = open(&mut session);
        assert_eq!(sent[0]["method"], "show_picker");
        assert_eq!(sent[0]["params"]["title"], "locate");
        assert_eq!(sent[1]["method"], "walk");
        assert_eq!(sent[1]["params"]["root"], "/root");
        assert_eq!(sent[1]["params"]["show_hidden"], true);
        assert_eq!(sent[1]["params"]["metadata"], true);
        assert_ne!(picker, walk);
    }

    #[test]
    fn filters_pick_from_the_walk_in_the_order_it_arrived() {
        let mut session = session();
        let (picker, walk, _) = open(&mut session);
        let mut out = Vec::new();
        session.apply(
            Command::Batch {
                walk,
                entries: vec![
                    entry("big.rs", 5_000_000, NOW - 10),
                    entry("small.rs", 10, NOW - 10),
                    entry("old.rs", 5_000_000, NOW - 90 * 86_400),
                    entry("big.md", 5_000_000, NOW - 10),
                ],
            },
            &mut out,
        );
        query(&mut session, picker, 3, "*.rs size:>1M mtime:<7d");
        let update = last_update(&session, true);
        assert_eq!(update["params"]["generation"], 3);
        assert_eq!(rels(&update), ["big.rs"]);
        assert_eq!(update["params"]["matches"], 1);
        assert_eq!(update["params"]["indexed"], 4);

        query(&mut session, picker, 4, "*.rs");
        assert_eq!(
            rels(&last_update(&session, true)),
            ["big.rs", "small.rs", "old.rs"]
        );
    }

    #[test]
    fn a_word_that_cannot_be_read_is_reported_once_and_the_rest_still_applies() {
        let mut session = session();
        let (picker, walk, _) = open(&mut session);
        let mut out = Vec::new();
        session.apply(
            Command::Batch {
                walk,
                entries: vec![entry("a.rs", 1, NOW), entry("b.md", 1, NOW)],
            },
            &mut out,
        );
        let first = lines(&query(&mut session, picker, 1, "*.rs size:>"));
        assert_eq!(first.len(), 1);
        assert_eq!(first[0]["method"], "log");
        assert!(
            first[0]["params"]["message"]
                .as_str()
                .unwrap()
                .contains("size:>")
        );
        assert_eq!(rels(&last_update(&session, true)), ["a.rs"]);

        let again = query(&mut session, picker, 2, "*.rs size:>");
        assert!(again.is_empty(), "the same word is not reported twice");
        let fixed = query(&mut session, picker, 3, "*.rs size:>0");
        assert!(fixed.is_empty());
    }

    #[test]
    fn the_walk_ending_is_shown_as_indexing_done_or_truncated() {
        let mut session = session();
        let (_, walk, _) = open(&mut session);
        assert_eq!(last_update(&session, true)["params"]["indexing"], "walking");
        let mut out = Vec::new();
        session.apply(
            Command::Walked {
                walk,
                end: WalkEnd {
                    outcome: WalkOutcome::Done,
                    truncated: true,
                },
            },
            &mut out,
        );
        assert_eq!(
            last_update(&session, true)["params"]["indexing"],
            "truncated"
        );
    }

    #[test]
    fn a_batch_or_query_for_another_walk_or_picker_is_ignored() {
        let mut session = session();
        let (picker, walk, _) = open(&mut session);
        let mut out = Vec::new();
        session.apply(
            Command::Batch {
                walk: walk + 100,
                entries: vec![entry("stale", 1, NOW)],
            },
            &mut out,
        );
        assert!(!session.apply(
            Command::Query {
                picker: picker + 100,
                generation: 9,
                text: "x".to_string(),
            },
            &mut out,
        ));
        let update = last_update(&session, true);
        assert_eq!(update["params"]["indexed"], 0);
        assert_eq!(update["params"]["generation"], 0);
    }

    #[test]
    fn closing_stops_the_walk_and_forgets_the_index() {
        let mut session = session();
        let (picker, walk, _) = open(&mut session);
        let mut out = Vec::new();
        session.apply(Command::Closed { picker }, &mut out);
        let cancel = &lines(&out)[0];
        assert_eq!(cancel["method"], "walk_cancel");
        assert_eq!(cancel["params"]["walk"], walk);
        assert!(session.update(true).is_none());
        assert!(session.advance(1), "nothing to check");
    }

    #[test]
    fn opening_again_stops_the_old_walk_first_and_numbers_never_repeat() {
        let mut session = session();
        let (first_picker, first_walk, _) = open(&mut session);
        let mut out = Vec::new();
        session.apply(
            Command::Open {
                root: PathBuf::from("/other"),
                show_hidden: false,
            },
            &mut out,
        );
        let sent = lines(&out);
        assert_eq!(sent[0]["method"], "walk_cancel");
        assert_eq!(sent[0]["params"]["walk"], first_walk);
        assert!(sent[1]["params"]["picker"].as_u64().unwrap() > first_picker);
        assert!(sent[2]["params"]["walk"].as_u64().unwrap() > first_walk);
    }

    #[test]
    fn a_refused_picker_puts_the_search_away_and_says_why() {
        let mut session = session();
        let (_, walk, sent) = open(&mut session);
        let mut out = Vec::new();
        session.apply(
            Command::Replied {
                id: sent[0]["id"].as_u64().unwrap(),
                error: Some("cannot open the picker: another picker is open".to_string()),
            },
            &mut out,
        );
        let sent = lines(&out);
        assert_eq!(sent[0]["method"], "log");
        assert!(
            sent[0]["params"]["message"]
                .as_str()
                .unwrap()
                .contains("another picker is open")
        );
        assert_eq!(sent[1]["method"], "walk_cancel");
        assert_eq!(sent[1]["params"]["walk"], walk);
        assert!(session.update(true).is_none());
    }

    #[test]
    fn a_refused_walk_leaves_the_picker_open_and_done() {
        let mut session = session();
        let (_, _, sent) = open(&mut session);
        let mut out = Vec::new();
        session.apply(
            Command::Replied {
                id: sent[1]["id"].as_u64().unwrap(),
                error: Some("walk 2 is already running".to_string()),
            },
            &mut out,
        );
        assert_eq!(last_update(&session, true)["params"]["indexing"], "done");
    }

    #[test]
    fn a_successful_reply_changes_nothing() {
        let mut session = session();
        let (_, _, sent) = open(&mut session);
        let mut out = Vec::new();
        session.apply(
            Command::Replied {
                id: sent[0]["id"].as_u64().unwrap(),
                error: None,
            },
            &mut out,
        );
        assert!(out.is_empty());
        assert!(session.update(true).is_some());
    }
}
