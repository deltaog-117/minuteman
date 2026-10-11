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

//! What a plugin that shows the Inspect panel exchanges with the host. The interface names the
//! entry to inspect (the `inspect` event, sent to the plugin that `provides = ["inspect"]`); the
//! plugin reads its metadata through the host (`stat`, so a browsed ssh folder or an archive is
//! inspected like the local disk is), counts a folder's contents with the host's `walk`, and
//! answers with the rows to show (`set_inspect`), first at once and again when a count lands. The
//! plugin never draws: the interface keeps the panel, its colors and its keys.
//!
//! A plugin may be wrong or hostile and its text ends up in the user's terminal, so
//! [`SetInspect::checked`] cleans every row before the host passes it on, the way a view or a
//! preview document is cleaned.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::view::is_unsafe;

/// The notification that gives the Inspect panel its rows: `set_inspect`.
pub const METHOD_SET_INSPECT: &str = "set_inspect";
/// The request that reads one entry's metadata through the host: `stat`.
pub const METHOD_STAT: &str = "stat";
/// The event name an inspecting plugin gets when the panel opens on an entry, or closes.
pub const EVENT_INSPECT: &str = "inspect";
/// What a plugin lists under `provides` to be the one that fills the Inspect panel.
pub const PROVIDES_INSPECT: &str = "inspect";

/// Most rows a panel holds.
pub const MAX_INSPECT_ROWS: usize = 32;
/// Most characters of a row's label; the panel's label column is narrower still.
pub const MAX_LABEL_CHARS: usize = 32;
/// Most characters of a row's value. Longer text is cut, since a path can be long and the panel
/// fits what it shows to its width anyway.
pub const MAX_VALUE_CHARS: usize = 1024;

/// The request that starts counting what is below a folder: `tally`. It returns at once; the
/// count arrives as a `tally_done` event.
pub const METHOD_TALLY: &str = "tally";
/// The notification that stops a count: `tally_cancel`.
pub const METHOD_TALLY_CANCEL: &str = "tally_cancel";
/// The event name a plugin gets when a count it asked for has ended.
pub const EVENT_TALLY_DONE: &str = "tally_done";

/// The entry the Inspect panel is about, in an `inspect` event: `null` when the panel closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectTarget {
    pub path: Option<PathBuf>,
}

/// The parameters of `stat`: describe the entry at `path` without following a final symlink, so a
/// link is described as a link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stat {
    pub path: PathBuf,
}

/// What kind of thing `stat` found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// What `stat` answers with. Fields a backend does not have are absent, and the panel leaves the
/// rows they would fill out rather than guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatReply {
    pub kind: StatKind,
    /// The entry's own size in bytes (for a folder, the directory node, not its contents).
    pub size: u64,
    /// Where a symlink points, as written in the link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target: Option<PathBuf>,
    /// Whether the target of a symlink is missing.
    #[serde(default)]
    pub link_broken: bool,
    /// Bytes allocated on the backing store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allocated: Option<u64>,
    /// The type and permission bits (`st_mode`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nlink: Option<u64>,
    /// Times as whole seconds since the Unix epoch; absent when unknown or before 1970.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<u64>,
    /// The same entry on this machine's own disk, when the backend is the local disk. The owner
    /// and group ids mean something in this machine's `passwd` and `group` only then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<PathBuf>,
}

/// The parameters of `tally`: count the files, folders and bytes below `root`, through whatever
/// serves the browsed paths. Symlinks are counted as links and never followed, so a link loop
/// cannot hang it, and the host stops at its own entry limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TallyRequest {
    /// Which count this is; the number the plugin's `tally_cancel` and the host's `tally_done`
    /// carry.
    pub tally: u64,
    pub root: PathBuf,
}

/// The parameters of `tally_cancel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TallyCancel {
    pub tally: u64,
}

/// How a count ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TallyEnding {
    Complete,
    /// Stopped at the entry limit: the numbers are a lower bound.
    Truncated,
    Cancelled,
}

/// What the host tells the plugin of a count, in a `tally_done` event under `tally`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TallyDone {
    pub tally: u64,
    pub files: u64,
    pub dirs: u64,
    /// The files' own sizes added up (a symlink counts as the link, not its target).
    pub bytes: u64,
    /// Folders inside that could not be read.
    pub unreadable: u64,
    pub ending: TallyEnding,
}

/// One row of the panel: a label and its value. On the wire it is `["Size","1.2K"]`.
pub type InspectRow = (String, String);

/// The parameters of `set_inspect`: the rows for the entry at `path`. The interface keeps them
/// only while its panel is still about `path`, so a slow answer for an entry the user has closed
/// is dropped, and a later one replaces an earlier one (a folder's count arriving).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetInspect {
    pub path: PathBuf,
    pub rows: Vec<InspectRow>,
}

/// Why a set of rows was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectError {
    TooManyRows,
}

impl fmt::Display for InspectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRows => write!(f, "the panel holds at most {MAX_INSPECT_ROWS} rows"),
        }
    }
}

impl std::error::Error for InspectError {}

fn cleaned(text: &mut String, limit: usize) {
    text.retain(|c| !is_unsafe(c));
    if let Some((cut, _)) = text.char_indices().nth(limit) {
        text.truncate(cut);
    }
}

impl SetInspect {
    /// These rows with control and direction-changing characters removed and over-long text cut,
    /// or why they cannot be shown.
    ///
    /// # Errors
    ///
    /// More than [`MAX_INSPECT_ROWS`] rows.
    pub fn checked(mut self) -> Result<Self, InspectError> {
        if self.rows.len() > MAX_INSPECT_ROWS {
            return Err(InspectError::TooManyRows);
        }
        for (label, value) in &mut self.rows {
            cleaned(label, MAX_LABEL_CHARS);
            cleaned(value, MAX_VALUE_CHARS);
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn set(rows: Vec<InspectRow>) -> SetInspect {
        SetInspect {
            path: PathBuf::from("/a"),
            rows,
        }
    }

    #[test]
    fn rows_are_pairs_on_the_wire() {
        let json = serde_json::to_string(&set(vec![("Size".into(), "1.2K".into())])).unwrap();
        assert_eq!(json, r#"{"path":"/a","rows":[["Size","1.2K"]]}"#);
    }

    #[test]
    fn a_stat_reply_with_only_a_kind_and_size_reads() {
        let reply: StatReply = serde_json::from_str(r#"{"kind":"dir","size":4096}"#).unwrap();
        assert_eq!(
            (reply.kind, reply.size, reply.mode),
            (StatKind::Dir, 4096, None)
        );
        assert!(!reply.link_broken);
    }

    #[test]
    fn too_many_rows_are_refused_and_the_limit_itself_is_not() {
        let rows = |n: usize| vec![("a".to_string(), "b".to_string()); n];
        assert!(set(rows(MAX_INSPECT_ROWS)).checked().is_ok());
        assert_eq!(
            set(rows(MAX_INSPECT_ROWS + 1)).checked(),
            Err(InspectError::TooManyRows)
        );
    }

    proptest! {
        #[test]
        fn nothing_unsafe_or_over_long_survives_checking(
            label in ".{0,80}",
            value in ".{0,1200}",
        ) {
            let checked = set(vec![(label, value)]).checked().unwrap();
            let (label, value) = &checked.rows[0];
            prop_assert!(label.chars().count() <= MAX_LABEL_CHARS);
            prop_assert!(value.chars().count() <= MAX_VALUE_CHARS);
            prop_assert!(!label.chars().chain(value.chars()).any(is_unsafe));
        }

        #[test]
        fn checking_twice_changes_nothing(label in ".{0,40}", value in ".{0,60}") {
            let once = set(vec![(label, value)]).checked().unwrap();
            prop_assert_eq!(once.clone().checked().unwrap(), once);
        }
    }
}
