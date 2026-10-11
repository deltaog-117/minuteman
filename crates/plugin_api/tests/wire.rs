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

//! The shapes `plugin_api` puts on the wire. A plugin written against this crate and the host
//! must agree on the spelling of each, so these pin the JSON rather than the Rust types.

use std::path::PathBuf;

use plugin_api::client::{Incoming, notification_line, parse_incoming, request_line};
use plugin_api::{
    API_VERSION, Change, EventKind, HostEvent, Job, JobOutcome, LogLevel, METHOD_WALK,
    PickerIndexing, PickerUpdate, StateSnapshot, TreeEntry, WalkEnd, WalkOutcome, WalkRequest,
    WireEntry,
};
use proptest::prelude::*;
use serde_json::json;
use shared::DirEntryInfo;

#[test]
fn job_outcomes_are_spelled_in_lowercase() {
    for (outcome, spelled) in [
        (JobOutcome::Completed, "completed"),
        (JobOutcome::Skipped, "skipped"),
        (JobOutcome::Cancelled, "cancelled"),
        (JobOutcome::Failed, "failed"),
    ] {
        assert_eq!(serde_json::to_value(outcome).unwrap(), json!(spelled));
        assert_eq!(
            serde_json::from_value::<JobOutcome>(json!(spelled)).unwrap(),
            outcome
        );
    }
}

#[test]
fn log_levels_are_spelled_in_lowercase_and_unknown_ones_are_refused() {
    for (level, spelled) in [
        (LogLevel::Info, "info"),
        (LogLevel::Warn, "warn"),
        (LogLevel::Error, "error"),
    ] {
        assert_eq!(serde_json::to_value(level).unwrap(), json!(spelled));
        assert_eq!(
            serde_json::from_value::<LogLevel>(json!(spelled)).unwrap(),
            level
        );
    }
    assert!(serde_json::from_value::<LogLevel>(json!("Debug")).is_err());
}

#[test]
fn event_names_round_trip_and_unknown_words_name_nothing() {
    for kind in [
        EventKind::DirChanged,
        EventKind::SelectionChanged,
        EventKind::JobFinished,
    ] {
        assert_eq!(EventKind::parse(kind.name()), Some(kind));
    }
    assert_eq!(EventKind::parse("key"), None);
    assert_eq!(EventKind::parse(""), None);
}

#[test]
fn a_change_with_no_entry_keeps_its_null() {
    let change = Change {
        from: None,
        to: Some(PathBuf::from("/a")),
    };
    let value = serde_json::to_value(&change).unwrap();
    assert_eq!(value, json!({ "from": null, "to": "/a" }));
    assert_eq!(serde_json::from_value::<Change>(value).unwrap(), change);
}

#[test]
fn a_job_carries_its_kind_and_outcome() {
    let job = Job {
        kind: "copy".to_string(),
        outcome: JobOutcome::Cancelled,
    };
    assert_eq!(
        serde_json::to_value(&job).unwrap(),
        json!({ "kind": "copy", "outcome": "cancelled" })
    );
}

#[test]
fn an_entry_without_a_readable_time_reports_null() {
    let entry = DirEntryInfo {
        name: "a".to_string(),
        path: PathBuf::from("/a"),
        is_dir: false,
        size: 3,
        modified: None,
        mode: None,
    };
    let wire = WireEntry::from(&entry);
    let value = serde_json::to_value(&wire).unwrap();
    assert!(value["modified"].is_null());
    assert!(value["mode"].is_null());
    assert_eq!(value["size"], 3);
}

fn event_of(event: &HostEvent) -> Box<plugin_api::EventParams> {
    match parse_incoming(&event.to_line()).unwrap() {
        Incoming::Event(params) => params,
        other => panic!("expected an event, got {other:?}"),
    }
}

#[test]
fn an_action_event_names_the_action_and_whether_dot_files_are_shown() {
    let event = HostEvent::action("find", PathBuf::from("/a"), vec![], true);
    let params = event_of(&event);
    assert_eq!(params.name, "action");
    assert_eq!(params.action.as_deref(), Some("find"));
    assert_eq!(params.show_hidden, Some(true));
    assert_eq!(params.cwd, PathBuf::from("/a"));
    assert_eq!(params.api_version, API_VERSION);
}

#[test]
fn picker_events_carry_the_picker_and_what_is_typed() {
    let query = event_of(&HostEvent::picker_query(3, 9, "ma"));
    let picker = query.picker.unwrap();
    assert_eq!(query.name, "picker_query");
    assert_eq!((picker.picker, picker.generation), (3, Some(9)));
    assert_eq!(picker.text.as_deref(), Some("ma"));

    let closed = event_of(&HostEvent::picker_closed(3));
    let picker = closed.picker.unwrap();
    assert_eq!(closed.name, "picker_closed");
    assert_eq!((picker.generation, picker.text), (None, None));
}

#[test]
fn walk_events_carry_the_entries_or_the_end() {
    let entry = TreeEntry {
        path: PathBuf::from("/r/a/b"),
        rel: "a/b".to_string(),
        is_dir: false,
        size: None,
        modified: None,
    };
    let batch = event_of(&HostEvent::walk_batch(4, vec![entry.clone()]));
    let walk = batch.walk.unwrap();
    assert_eq!(batch.name, "walk_batch");
    assert_eq!((walk.walk, walk.entries, walk.end), (4, vec![entry], None));

    let end = WalkEnd {
        outcome: WalkOutcome::Cancelled,
        truncated: false,
    };
    let done = event_of(&HostEvent::walk_done(4, end));
    let walk = done.walk.unwrap();
    assert_eq!(done.name, "walk_done");
    assert_eq!((walk.entries.len(), walk.end), (0, Some(end)));
}

#[test]
fn metadata_is_optional_on_the_wire_so_older_plugins_and_hosts_still_agree() {
    let request: WalkRequest =
        serde_json::from_value(json!({"walk": 1, "root": "/r", "show_hidden": false})).unwrap();
    assert!(
        !request.metadata,
        "a plugin that never heard of it gets none"
    );

    let entry: TreeEntry =
        serde_json::from_value(json!({"path": "/r/a", "rel": "a", "is_dir": false})).unwrap();
    assert_eq!((entry.size, entry.modified), (None, None));

    let bare = serde_json::to_value(&entry).unwrap();
    assert!(bare.get("size").is_none() && bare.get("modified").is_none());

    let full = TreeEntry {
        size: Some(7),
        modified: Some(9),
        ..entry
    };
    let value = serde_json::to_value(&full).unwrap();
    assert_eq!((&value["size"], &value["modified"]), (&json!(7), &json!(9)));
    assert_eq!(serde_json::from_value::<TreeEntry>(value).unwrap(), full);
}

#[test]
fn an_entry_knows_its_name_and_its_folder() {
    let nested = TreeEntry {
        path: PathBuf::from("/r/src/ui/main.rs"),
        rel: "src/ui/main.rs".to_string(),
        is_dir: false,
        size: None,
        modified: None,
    };
    assert_eq!((nested.name(), nested.parent_rel()), ("main.rs", "src/ui"));
    let top = TreeEntry {
        path: PathBuf::from("/r/main.rs"),
        rel: "main.rs".to_string(),
        is_dir: false,
        size: None,
        modified: None,
    };
    assert_eq!((top.name(), top.parent_rel()), ("main.rs", ""));
}

#[test]
fn indexing_progress_is_spelled_in_lowercase() {
    for (state, spelled) in [
        (PickerIndexing::Walking, "walking"),
        (PickerIndexing::Done, "done"),
        (PickerIndexing::Truncated, "truncated"),
    ] {
        assert_eq!(serde_json::to_value(state).unwrap(), json!(spelled));
    }
}

#[test]
fn a_reply_is_read_with_its_id_and_the_reason_when_it_failed() {
    let ok = parse_incoming(r#"{"jsonrpc":"2.0","id":3,"result":{"walk":1}}"#).unwrap();
    assert!(matches!(ok, Incoming::Reply { error: None, .. }));
    let failed = parse_incoming(r#"{"jsonrpc":"2.0","id":4,"error":{"message":"no"}}"#).unwrap();
    match failed {
        Incoming::Reply { id, error } => {
            assert_eq!(id, json!(4));
            assert_eq!(error.as_deref(), Some("no"));
        }
        other => panic!("expected a reply, got {other:?}"),
    }
    assert!(parse_incoming("{}").is_err());
    assert!(parse_incoming("not json").is_err());
}

#[test]
fn request_and_notification_lines_have_the_shape_the_host_reads() {
    let request = WalkRequest {
        walk: 2,
        root: PathBuf::from("/r"),
        show_hidden: false,
        metadata: false,
    };
    let line = request_line(7, METHOD_WALK, &request);
    assert!(line.ends_with('\n'));
    let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(value["id"], 7);
    assert_eq!(value["method"], "walk");
    assert_eq!(value["params"]["root"], "/r");

    let update = PickerUpdate {
        picker: 1,
        generation: 2,
        items: vec![],
        matches: 0,
        indexed: 0,
        indexing: PickerIndexing::Walking,
        settled: false,
    };
    let value: serde_json::Value =
        serde_json::from_str(notification_line("picker_items", &update).trim()).unwrap();
    assert!(value.get("id").is_none());
    assert_eq!(value["params"]["indexing"], "walking");
}

proptest! {
    #[test]
    fn a_state_snapshot_survives_a_round_trip(
        cwd in "/[a-z/]{0,12}",
        cursor in proptest::option::of("/[a-z/]{1,12}"),
        marks in proptest::collection::vec("/[a-z/]{1,12}", 0..5),
        show_hidden: bool,
        sort in "[a-z]{0,8}",
        sort_reversed: bool,
        view in "[a-z]{0,8}",
    ) {
        let snapshot = StateSnapshot {
            cwd: PathBuf::from(cwd),
            cursor: cursor.map(PathBuf::from),
            marks: marks.into_iter().map(PathBuf::from).collect(),
            show_hidden,
            sort,
            sort_reversed,
            view,
        };
        let text = serde_json::to_string(&snapshot).unwrap();
        prop_assert_eq!(serde_json::from_str::<StateSnapshot>(&text).unwrap(), snapshot);
    }
}
