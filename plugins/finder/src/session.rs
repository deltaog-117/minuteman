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

//! The finder's state and how it answers each thing the host tells it, with no input or output of
//! its own: `apply` takes a `Command` and pushes the lines it wants written, which is what lets
//! it be tested without a process on the other end.
//!
//! One finder is open at a time. Opening it picks fresh numbers for the picker and the walk, asks
//! the host for both, and from then on only listens to events that carry those numbers, so a late
//! batch from an earlier walk can never end up in this one's index.

use std::collections::HashMap;
use std::path::PathBuf;

use plugin_api::client::{notification_line, request_line};
use plugin_api::{
    METHOD_PICKER_ITEMS, METHOD_SHOW_PICKER, METHOD_WALK, METHOD_WALK_CANCEL, PickerIndexing,
    PickerSpec, PickerUpdate, TreeEntry, WalkCancel, WalkEnd, WalkOutcome, WalkRequest,
};
use serde_json::json;
use shared::Query;

use crate::rank::{Ranking, SHOWN};

/// The name of the action this plugin provides, as the host's keymap spells it.
pub const ACTION: &str = "find";

/// What the host tells the finder, already read from its line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// The user triggered `find` in `root`.
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

/// The finder while it is open: what the walk has found and how it ranks against what is typed.
struct Open {
    picker: u64,
    walk: u64,
    index: Vec<TreeEntry>,
    ranking: Ranking,
    /// Which query the ranking is for; the host keeps older results on screen until it sees it.
    generation: u64,
    walked: Option<WalkEnd>,
}

#[derive(Default)]
pub struct Session {
    /// Numbers for pickers and walks, which never repeat within a run.
    numbers: u64,
    requests: u64,
    pending: HashMap<u64, Pending>,
    open: Option<Open>,
}

impl Session {
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
    /// query (or opened the finder), so the caller sends the first answer to it at once.
    pub fn apply(&mut self, command: Command, out: &mut Vec<String>) -> bool {
        match command {
            Command::Open { root, show_hidden } => {
                self.close(out);
                let (picker, walk) = (self.number(), self.number());
                let spec = PickerSpec {
                    picker,
                    title: "find".to_string(),
                    preview: true,
                };
                let id = self.request(Pending::Picker(picker));
                out.push(request_line(id, METHOD_SHOW_PICKER, &spec));
                let request = WalkRequest {
                    walk,
                    root,
                    show_hidden,
                    metadata: false,
                };
                let id = self.request(Pending::Walk(walk));
                out.push(request_line(id, METHOD_WALK, &request));
                self.open = Some(Open {
                    picker,
                    walk,
                    index: Vec::new(),
                    ranking: Ranking::new(),
                    generation: 0,
                    walked: None,
                });
                true
            }
            Command::Query {
                picker,
                generation,
                text,
            } => match self.open.as_mut() {
                Some(open) if open.picker == picker => {
                    open.generation = generation;
                    open.ranking.set_query(Query::new(&text));
                    true
                }
                _ => false,
            },
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

    /// Stops the walk behind the open finder and forgets its index.
    fn close(&mut self, out: &mut Vec<String>) {
        if let Some(open) = self.open.take() {
            out.push(notification_line(
                METHOD_WALK_CANCEL,
                &WalkCancel { walk: open.walk },
            ));
        }
    }

    /// The host said no to a request: say so in the status bar, and put the finder away if the
    /// picker was what it refused.
    fn refused(&mut self, what: Pending, reason: &str, out: &mut Vec<String>) {
        out.push(log_line(&format!("find: {reason}")));
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

    /// Ranks at most `budget` entries toward the current query and says whether it has caught up
    /// with the whole index (trivially so while the finder is closed).
    pub fn advance(&mut self, budget: usize) -> bool {
        match self.open.as_mut() {
            Some(open) => open.ranking.advance(&open.index, budget),
            None => true,
        }
    }

    /// The `picker_items` line for the best results so far, or `None` with the finder closed.
    /// `settled` says whether every indexed entry has been ranked.
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
                .ranking
                .top(open.index.len(), SHOWN)
                .into_iter()
                .map(|position| open.index[position].clone())
                .collect(),
            matches: open.ranking.matches(open.index.len()),
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

    fn entry(rel: &str) -> TreeEntry {
        TreeEntry {
            path: PathBuf::from("/root").join(rel),
            rel: rel.to_string(),
            is_dir: false,
            size: None,
            modified: None,
        }
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
        let sent: Vec<Value> = out
            .iter()
            .map(|line| serde_json::from_str(line.trim()).unwrap())
            .collect();
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

    #[test]
    fn opening_asks_for_the_picker_then_the_walk_below_the_browsed_folder() {
        let mut session = Session::default();
        let (picker, walk, sent) = open(&mut session);
        assert_eq!(sent[0]["method"], "show_picker");
        assert_eq!(sent[0]["params"]["title"], "find");
        assert_eq!(sent[0]["params"]["preview"], true);
        assert_eq!(sent[1]["method"], "walk");
        assert_eq!(sent[1]["params"]["root"], "/root");
        assert_eq!(sent[1]["params"]["show_hidden"], true);
        assert_ne!(picker, walk);
        assert_ne!(sent[0]["id"], sent[1]["id"]);
    }

    #[test]
    fn entries_of_the_walk_are_ranked_against_what_is_typed() {
        let mut session = Session::default();
        let (picker, walk, _) = open(&mut session);
        let mut out = Vec::new();
        session.apply(
            Command::Batch {
                walk,
                entries: vec![entry("src"), entry("src/main.rs"), entry("README.md")],
            },
            &mut out,
        );
        assert!(session.advance(usize::MAX));
        let update = last_update(&session, true);
        assert_eq!(update["params"]["indexed"], 3);
        assert_eq!(update["params"]["indexing"], "walking");

        assert!(session.apply(
            Command::Query {
                picker,
                generation: 4,
                text: "main".to_string(),
            },
            &mut out,
        ));
        session.advance(usize::MAX);
        let update = last_update(&session, true);
        assert_eq!(update["params"]["generation"], 4);
        assert_eq!(rels(&update), ["src/main.rs"]);
        assert_eq!(update["params"]["matches"], 1);

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
        let mut session = Session::default();
        let (picker, walk, _) = open(&mut session);
        let mut out = Vec::new();
        session.apply(
            Command::Batch {
                walk: walk + 100,
                entries: vec![entry("stale")],
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
        let mut session = Session::default();
        let (picker, walk, _) = open(&mut session);
        let mut out = Vec::new();
        session.apply(Command::Closed { picker }, &mut out);
        let cancel: Value = serde_json::from_str(out[0].trim()).unwrap();
        assert_eq!(cancel["method"], "walk_cancel");
        assert_eq!(cancel["params"]["walk"], walk);
        assert!(session.update(true).is_none());
        assert!(session.advance(1), "nothing to rank");
    }

    #[test]
    fn opening_again_stops_the_old_walk_first_and_numbers_never_repeat() {
        let mut session = Session::default();
        let (first_picker, first_walk, _) = open(&mut session);
        let mut out = Vec::new();
        session.apply(
            Command::Open {
                root: PathBuf::from("/other"),
                show_hidden: false,
            },
            &mut out,
        );
        let sent: Vec<Value> = out
            .iter()
            .map(|line| serde_json::from_str(line.trim()).unwrap())
            .collect();
        assert_eq!(sent[0]["method"], "walk_cancel");
        assert_eq!(sent[0]["params"]["walk"], first_walk);
        assert!(sent[1]["params"]["picker"].as_u64().unwrap() > first_picker);
        assert!(sent[2]["params"]["walk"].as_u64().unwrap() > first_walk);
    }

    #[test]
    fn a_refused_picker_puts_the_finder_away_and_says_why() {
        let mut session = Session::default();
        let (_, walk, sent) = open(&mut session);
        let mut out = Vec::new();
        session.apply(
            Command::Replied {
                id: sent[0]["id"].as_u64().unwrap(),
                error: Some("cannot open the picker: another picker is open".to_string()),
            },
            &mut out,
        );
        let lines: Vec<Value> = out
            .iter()
            .map(|line| serde_json::from_str(line.trim()).unwrap())
            .collect();
        assert_eq!(lines[0]["method"], "log");
        assert!(
            lines[0]["params"]["message"]
                .as_str()
                .unwrap()
                .contains("another picker is open")
        );
        assert_eq!(lines[1]["method"], "walk_cancel");
        assert_eq!(lines[1]["params"]["walk"], walk);
        assert!(session.update(true).is_none());
    }

    #[test]
    fn a_refused_walk_leaves_the_picker_open_and_done() {
        let mut session = Session::default();
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
        let mut session = Session::default();
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
