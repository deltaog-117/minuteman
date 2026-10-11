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

use plugin_api::{Change, EventKind, Job, JobOutcome, LogLevel, StateSnapshot, WireEntry};
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
