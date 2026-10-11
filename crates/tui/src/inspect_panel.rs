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

//! The Inspect panel's state. The rows are not made here: the plugin that provides `inspect`
//! describes the entry and counts a folder's contents, and sends the rows (see
//! `plugin_api::inspect`). The panel only remembers which entry it was opened on, shows a
//! placeholder until the first rows arrive, and replaces them when later ones do (a folder's
//! count landing). Rows for any other entry are dropped, so a slow answer cannot show up on a
//! panel that has since been opened on something else.

use std::path::{Path, PathBuf};

use plugin_host::protocol::SetInspect;

/// An open Inspect panel.
pub struct InspectPanel {
    path: PathBuf,
    title: String,
    /// What the plugin last sent, or `None` while nothing has arrived.
    rows: Option<Vec<(String, String)>>,
}

impl InspectPanel {
    /// A panel about `path` that is waiting for its rows.
    pub fn open(path: &Path) -> Self {
        let title = path.file_name().map_or_else(
            || path.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        Self {
            path: path.to_path_buf(),
            title,
            rows: None,
        }
    }

    /// Takes the plugin's rows if they are about this panel's entry. Returns whether they were.
    pub fn accept(&mut self, set: SetInspect) -> bool {
        if set.path != self.path {
            return false;
        }
        self.rows = Some(set.rows);
        true
    }

    /// Whether no rows have arrived yet.
    pub fn waiting(&self) -> bool {
        self.rows.is_none()
    }

    /// Says so on the panel when the plugin ended without answering, so it does not claim to be
    /// loading for ever.
    pub fn mark_stopped(&mut self) {
        self.rows = Some(vec![
            ("Name".to_string(), self.title.clone()),
            (
                "Error".to_string(),
                "the Inspect plugin stopped before it answered".to_string(),
            ),
        ]);
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn rows(&self) -> Vec<(String, String)> {
        match &self.rows {
            Some(rows) => rows.clone(),
            None => vec![
                ("Name".to_string(), self.title.clone()),
                ("Status".to_string(), "loading…".to_string()),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(l, v)| ((*l).to_string(), (*v).to_string()))
            .collect()
    }

    fn set(path: &str, pairs: &[(&str, &str)]) -> SetInspect {
        SetInspect {
            path: PathBuf::from(path),
            rows: rows(pairs),
        }
    }

    #[test]
    fn a_new_panel_is_titled_by_the_entry_and_says_it_is_loading() {
        let panel = InspectPanel::open(Path::new("/d/note.txt"));
        assert_eq!(panel.title(), "note.txt");
        assert!(panel.waiting());
        assert_eq!(
            panel.rows(),
            rows(&[("Name", "note.txt"), ("Status", "loading…")])
        );
    }

    #[test]
    fn rows_for_its_entry_replace_the_placeholder_and_later_rows_replace_those() {
        let mut panel = InspectPanel::open(Path::new("/d"));
        assert!(panel.accept(set("/d", &[("Contents", "counting…")])));
        assert!(!panel.waiting());
        assert!(panel.accept(set("/d", &[("Contents", "3 files")])));
        assert_eq!(panel.rows(), rows(&[("Contents", "3 files")]));
    }

    #[test]
    fn rows_for_another_entry_are_dropped() {
        let mut panel = InspectPanel::open(Path::new("/d"));
        assert!(!panel.accept(set("/other", &[("Name", "x")])));
        assert!(panel.waiting());
    }

    #[test]
    fn a_plugin_that_stopped_before_answering_is_reported_on_the_panel() {
        let mut panel = InspectPanel::open(Path::new("/d"));
        panel.mark_stopped();
        assert!(!panel.waiting());
        assert!(panel.rows().iter().any(|(l, _)| l == "Error"));
    }
}
