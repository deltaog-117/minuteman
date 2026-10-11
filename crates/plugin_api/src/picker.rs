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

//! What a plugin and the host exchange to run a picker (the list the fuzzy finder is drawn in) and
//! to walk a folder tree. The host draws the picker and owns the disk; the plugin owns what is
//! listed in it and how it is ranked. A plugin chooses the `picker` and `walk` numbers itself, so
//! it never has to wait for a reply to know which one an event is about.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The request that opens the picker: `show_picker`. The host answers with an error if one is
/// already open, since a picker is modal.
pub const METHOD_SHOW_PICKER: &str = "show_picker";
/// The notification that replaces what the picker lists: `picker_items`.
pub const METHOD_PICKER_ITEMS: &str = "picker_items";
/// The notification that closes the picker from the plugin's side: `picker_close`.
pub const METHOD_PICKER_CLOSE: &str = "picker_close";
/// The request that starts a walk of a folder tree: `walk`.
pub const METHOD_WALK: &str = "walk";
/// The notification that stops a walk: `walk_cancel`.
pub const METHOD_WALK_CANCEL: &str = "walk_cancel";

/// One entry of a folder tree, as a walk reports it and a picker lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeEntry {
    pub path: PathBuf,
    /// The path below the folder the walk began at, with `/` separators: what is matched and shown.
    pub rel: String,
    pub is_dir: bool,
}

impl TreeEntry {
    /// The entry's own name: the last piece of `rel`.
    pub fn name(&self) -> &str {
        self.rel.rsplit('/').next().unwrap_or(&self.rel)
    }

    /// The folders above the entry, relative to the walk's root, or an empty string at the top.
    pub fn parent_rel(&self) -> &str {
        self.rel.rfind('/').map_or("", |slash| &self.rel[..slash])
    }
}

/// The parameters of `show_picker`. Choosing a result shows it in the browser; closing the picker
/// from the keyboard tells the plugin with a `picker_closed` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickerSpec {
    /// Which picker this is; the number the plugin's later `picker_items` and the host's events carry.
    pub picker: u64,
    /// Shown on the panel's border.
    pub title: String,
    /// Whether the selected result is previewed beside the list, on a screen wide enough.
    pub preview: bool,
}

/// Where the walk behind a picker stands, for the header's note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PickerIndexing {
    Walking,
    Done,
    /// Done, but a limit cut the walk short, so entries may exist beyond what was indexed.
    Truncated,
}

/// The parameters of `picker_items`: the best results for the newest query the plugin has ranked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickerUpdate {
    pub picker: u64,
    /// The `generation` of the `picker_query` these results answer. The host keeps the previous
    /// results on screen until it gets ones for the query it last sent.
    pub generation: u64,
    /// The best matches, best first.
    pub items: Vec<TreeEntry>,
    /// How many entries match so far (more than `items` holds when it is a lot).
    pub matches: usize,
    /// How many entries have been indexed so far.
    pub indexed: usize,
    pub indexing: PickerIndexing,
    /// Whether every indexed entry has been ranked against the query. False while the plugin is
    /// still catching up, when `items` may yet change.
    pub settled: bool,
}

/// The parameters of `picker_close`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickerClose {
    pub picker: u64,
}

/// The parameters of `walk`: list everything below `root`, nearest level first, through whatever
/// serves the browsed paths (the local disk, a machine over ssh, the inside of an archive).
/// Entries arrive as `walk_batch` events, one per folder listed, and a `walk_done` ends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalkRequest {
    pub walk: u64,
    pub root: PathBuf,
    /// Whether dot-files are listed and entered; the host's own limits on depth and count apply.
    pub show_hidden: bool,
}

/// The parameters of `walk_cancel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalkCancel {
    pub walk: u64,
}

/// How a walk ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WalkOutcome {
    /// Every entry within the limits was sent.
    Done,
    /// The plugin cancelled it, or the picker it served was closed.
    Cancelled,
}

/// The end of a `walk_done` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalkEnd {
    pub outcome: WalkOutcome,
    /// Whether a limit cut the walk short, so entries may exist beyond what was sent.
    pub truncated: bool,
}

/// What the host tells the plugin about its picker, nested under `picker` in the event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickerEvent {
    pub picker: u64,
    /// On `picker_query`: rises with every change of what is typed, so a plugin can tell a stale
    /// ranking from a current one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    /// On `picker_query`: what is typed, empty for nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// What the host sends of a walk, nested under `walk` in the event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalkEvent {
    pub walk: u64,
    /// On `walk_batch`: the entries of one folder (or a slice of a large one).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<TreeEntry>,
    /// On `walk_done`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<WalkEnd>,
}
