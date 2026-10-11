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

//! What the plugin does with each line the host writes: which entry the panel is about, what is
//! still being waited for, and the lines to answer with. It never reads or writes a stream, so a
//! whole conversation with the host can be played through it in a test.
//!
//! Only the newest entry matters. Opening another one, or closing the panel, stops the count under
//! way, and an answer to anything older is dropped on arrival (a `stat` reply by its request id, a
//! `tally_done` by its count number).

use std::path::PathBuf;

use plugin_api::client::{notification_line, request_line};
use plugin_api::{
    EVENT_INSPECT, EVENT_TALLY_DONE, EventParams, METHOD_SET_INSPECT, METHOD_STAT, METHOD_TALLY,
    METHOD_TALLY_CANCEL, SetInspect, Stat, StatKind, StatReply, TallyCancel, TallyRequest,
};
use serde_json::Value;

use crate::rows::{self, Contents};

/// A count that has been asked for and not yet answered.
#[derive(Debug, Clone, Copy)]
struct Counting {
    /// The id of the `tally` request, whose reply only says whether the count started.
    request: u64,
    /// The count's number, which its `tally_done` carries.
    number: u64,
}

#[derive(Debug)]
struct Current {
    path: PathBuf,
    /// The id of the `stat` request this entry is waiting on.
    stat_request: u64,
    stat: Option<StatReply>,
    counting: Option<Counting>,
    contents: Contents,
}

#[derive(Debug, Default)]
pub struct Session {
    /// Request ids and count numbers share one counter, so none is ever reused.
    last: u64,
    current: Option<Current>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    fn next(&mut self) -> u64 {
        self.last += 1;
        self.last
    }

    /// The lines to write for an event the host sent.
    pub fn on_event(&mut self, event: &EventParams) -> Vec<String> {
        match event.name.as_str() {
            EVENT_INSPECT => {
                let mut out = self.stop_counting();
                self.current = None;
                if let Some(path) = event.inspect.as_ref().and_then(|t| t.path.clone()) {
                    let id = self.next();
                    out.push(request_line(id, METHOD_STAT, &Stat { path: path.clone() }));
                    self.current = Some(Current {
                        path,
                        stat_request: id,
                        stat: None,
                        counting: None,
                        contents: Contents::NotApplicable,
                    });
                }
                out
            }
            EVENT_TALLY_DONE => {
                let (Some(done), Some(current)) = (event.tally, self.current.as_mut()) else {
                    return Vec::new();
                };
                if current.counting.is_none_or(|c| c.number != done.tally) {
                    return Vec::new();
                }
                current.counting = None;
                current.contents = Contents::Counted(done);
                self.answer()
            }
            _ => Vec::new(),
        }
    }

    /// The lines to write for the reply to one of this plugin's requests.
    pub fn on_reply(
        &mut self,
        id: u64,
        error: Option<String>,
        result: Option<Value>,
    ) -> Vec<String> {
        let Some(current) = self.current.as_mut() else {
            return Vec::new();
        };
        if id == current.stat_request {
            return self.stat_replied(error, result);
        }
        // A count that could not start will never report, so stop saying it is counting.
        if current.counting.is_some_and(|c| c.request == id) && error.is_some() {
            current.counting = None;
            current.contents = Contents::Unavailable;
            return self.answer();
        }
        Vec::new()
    }

    fn stat_replied(&mut self, error: Option<String>, result: Option<Value>) -> Vec<String> {
        let parsed = match (error, result) {
            (Some(message), _) => Err(message),
            (None, Some(value)) => serde_json::from_value::<StatReply>(value)
                .map_err(|e| format!("unreadable reply to stat: {e}")),
            (None, None) => Err("an empty reply to stat".to_string()),
        };
        let number = self.next();
        let request = self.next();
        let Some(current) = self.current.as_mut() else {
            return Vec::new();
        };
        match parsed {
            Err(message) => {
                let set = SetInspect {
                    path: current.path.clone(),
                    rows: rows::failure(&current.path, &message),
                };
                vec![notification_line(METHOD_SET_INSPECT, &set)]
            }
            Ok(stat) => {
                let mut out = Vec::new();
                if stat.kind == StatKind::Dir {
                    current.contents = Contents::Counting;
                    current.counting = Some(Counting { request, number });
                    out.push(request_line(
                        request,
                        METHOD_TALLY,
                        &TallyRequest {
                            tally: number,
                            root: current.path.clone(),
                        },
                    ));
                }
                current.stat = Some(stat);
                // The rows go first, with "counting…", so the panel never waits on the count.
                let mut lines = self.answer();
                lines.append(&mut out);
                lines
            }
        }
    }

    /// The `set_inspect` line for what is known of the current entry now.
    fn answer(&self) -> Vec<String> {
        let Some(current) = self.current.as_ref() else {
            return Vec::new();
        };
        let Some(stat) = current.stat.as_ref() else {
            return Vec::new();
        };
        let set = SetInspect {
            path: current.path.clone(),
            rows: rows::rows(&current.path, stat, &current.contents),
        };
        vec![notification_line(METHOD_SET_INSPECT, &set)]
    }

    /// The line that stops the count under way, if there is one.
    fn stop_counting(&mut self) -> Vec<String> {
        self.current
            .as_mut()
            .and_then(|current| current.counting.take())
            .map(|counting| {
                vec![notification_line(
                    METHOD_TALLY_CANCEL,
                    &TallyCancel {
                        tally: counting.number,
                    },
                )]
            })
            .unwrap_or_default()
    }

    /// The lines to write when the host has gone: nothing is left to answer.
    pub fn on_gone(&mut self) -> Vec<String> {
        self.current = None;
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use plugin_api::{HostEvent, TallyDone, TallyEnding};
    use serde_json::json;

    use super::*;

    fn event(host: HostEvent) -> EventParams {
        let line = host.to_line();
        let value: Value = serde_json::from_str(line.trim()).unwrap();
        serde_json::from_value(value["params"].clone()).unwrap()
    }

    fn inspect(path: Option<&str>) -> EventParams {
        event(HostEvent::inspect(path.map(PathBuf::from)))
    }

    fn parsed(lines: &[String]) -> Vec<Value> {
        lines
            .iter()
            .map(|line| serde_json::from_str(line.trim()).unwrap())
            .collect()
    }

    fn methods(lines: &[String]) -> Vec<String> {
        parsed(lines)
            .iter()
            .map(|v| v["method"].as_str().unwrap().to_string())
            .collect()
    }

    fn dir_reply() -> Option<Value> {
        Some(json!({ "kind": "dir", "size": 4096 }))
    }

    fn id_of(lines: &[String], method: &str) -> u64 {
        parsed(lines)
            .iter()
            .find(|v| v["method"] == method)
            .and_then(|v| v["id"].as_u64())
            .expect("a request with an id")
    }

    fn done(tally: u64) -> EventParams {
        event(HostEvent::tally_done(TallyDone {
            tally,
            files: 3,
            dirs: 1,
            bytes: 10,
            unreadable: 0,
            ending: TallyEnding::Complete,
        }))
    }

    #[test]
    fn an_entry_is_stat_ed_then_described_and_a_file_is_not_counted() {
        let mut session = Session::new();
        let asked = session.on_event(&inspect(Some("/d/f")));
        assert_eq!(methods(&asked), ["stat"]);
        let id = id_of(&asked, "stat");

        let sent = session.on_reply(id, None, Some(json!({ "kind": "file", "size": 5 })));
        assert_eq!(methods(&sent), ["set_inspect"], "no count for a file");
        let set = &parsed(&sent)[0]["params"];
        assert_eq!(set["path"], "/d/f");
        assert_eq!(set["rows"][0], json!(["Name", "f"]));
    }

    #[test]
    fn a_folder_is_described_at_once_and_again_when_its_count_lands() {
        let mut session = Session::new();
        let id = id_of(&session.on_event(&inspect(Some("/d"))), "stat");

        let sent = session.on_reply(id, None, dir_reply());
        assert_eq!(methods(&sent), ["set_inspect", "tally"], "rows first");
        let counting = parsed(&sent)[0]["params"]["rows"].to_string();
        assert!(counting.contains("counting…"), "{counting}");
        let number = parsed(&sent)[1]["params"]["tally"].as_u64().unwrap();

        let finished = session.on_event(&done(number));
        assert_eq!(methods(&finished), ["set_inspect"]);
        let rows = parsed(&finished)[0]["params"]["rows"].to_string();
        assert!(rows.contains("3 files, 1 folders, 10B"), "{rows}");
    }

    #[test]
    fn opening_another_entry_or_closing_the_panel_stops_the_count_under_way() {
        let mut session = Session::new();
        let id = id_of(&session.on_event(&inspect(Some("/d"))), "stat");
        let sent = session.on_reply(id, None, dir_reply());
        let number = parsed(&sent)[1]["params"]["tally"].as_u64().unwrap();

        let next = session.on_event(&inspect(Some("/e")));
        assert_eq!(methods(&next), ["tally_cancel", "stat"]);
        assert_eq!(parsed(&next)[0]["params"]["tally"], number);
        assert!(
            session.on_event(&done(number)).is_empty(),
            "the stopped count's end is not about /e"
        );

        let id = id_of(&next, "stat");
        let _ = session.on_reply(id, None, dir_reply());
        let closed = session.on_event(&inspect(None));
        assert_eq!(methods(&closed), ["tally_cancel"]);
        assert!(
            session.on_event(&inspect(None)).is_empty(),
            "nothing left to stop"
        );
    }

    #[test]
    fn a_stat_reply_for_an_entry_the_panel_has_left_is_dropped() {
        let mut session = Session::new();
        let old = id_of(&session.on_event(&inspect(Some("/old"))), "stat");
        let new = id_of(&session.on_event(&inspect(Some("/new"))), "stat");
        assert!(session.on_reply(old, None, dir_reply()).is_empty());
        assert!(!session.on_reply(new, None, dir_reply()).is_empty());
    }

    #[test]
    fn an_entry_that_cannot_be_described_says_why_instead_of_leaving_the_panel_waiting() {
        let mut session = Session::new();
        let id = id_of(&session.on_event(&inspect(Some("/gone"))), "stat");
        let sent = session.on_reply(id, Some("No such file".into()), None);
        let rows = parsed(&sent)[0]["params"]["rows"].to_string();
        assert!(
            rows.contains("Error") && rows.contains("No such file"),
            "{rows}"
        );

        let id = id_of(&session.on_event(&inspect(Some("/odd"))), "stat");
        let garbled = session.on_reply(id, None, Some(json!({ "kind": "nope" })));
        assert!(
            parsed(&garbled)[0]["params"]["rows"]
                .to_string()
                .contains("Error")
        );
    }

    #[test]
    fn a_count_that_cannot_start_stops_claiming_to_count() {
        let mut session = Session::new();
        let id = id_of(&session.on_event(&inspect(Some("/d"))), "stat");
        let sent = session.on_reply(id, None, dir_reply());
        let request = id_of(&sent, "tally");

        let failed = session.on_reply(request, Some("busy".into()), None);
        let rows = parsed(&failed)[0]["params"]["rows"].to_string();
        assert!(rows.contains("unavailable"), "{rows}");
        // The reply that says it started is not an event worth a line.
        let mut again = Session::new();
        let id = id_of(&again.on_event(&inspect(Some("/d"))), "stat");
        let request = id_of(&again.on_reply(id, None, dir_reply()), "tally");
        assert!(
            again
                .on_reply(request, None, Some(json!({ "tally": 2 })))
                .is_empty()
        );
    }

    #[test]
    fn other_events_and_replies_with_nothing_open_say_nothing() {
        let mut session = Session::new();
        let other = event(HostEvent::new(
            "dir_changed",
            PathBuf::from("/"),
            Vec::new(),
        ));
        assert!(session.on_event(&other).is_empty());
        assert!(session.on_reply(1, None, dir_reply()).is_empty());
        assert!(session.on_event(&done(1)).is_empty());
        assert!(session.on_gone().is_empty());
    }
}
