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

//! Miller-column navigation state and logic — no rendering, no I/O beyond the `Vfs` trait.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use shared::{DirEntryInfo, Vfs, VfsError};

#[derive(Debug)]
pub struct BrowserState {
    current_dir: PathBuf,
    parent_entries: Vec<DirEntryInfo>,
    current_entries: Vec<DirEntryInfo>,
    selected: usize,
    /// Ranger-style marks: paths toggled via `Select`, persisting across navigation until
    /// explicitly toggled off or consumed by a bulk action (e.g. `Delete`).
    marked: HashSet<PathBuf>,
}

impl BrowserState {
    pub fn new(vfs: &dyn Vfs, start_dir: PathBuf) -> Result<Self, VfsError> {
        let mut state = Self {
            current_dir: start_dir,
            parent_entries: Vec::new(),
            current_entries: Vec::new(),
            selected: 0,
            marked: HashSet::new(),
        };
        state.refresh(vfs)?;
        Ok(state)
    }

    pub fn current_dir(&self) -> &std::path::Path {
        &self.current_dir
    }

    pub fn current_entries(&self) -> &[DirEntryInfo] {
        &self.current_entries
    }

    pub fn parent_entries(&self) -> &[DirEntryInfo] {
        &self.parent_entries
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected_entry(&self) -> Option<&DirEntryInfo> {
        self.current_entries.get(self.selected)
    }

    /// Toggles the current entry's mark on/off. No-op on an empty listing.
    pub fn toggle_mark(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let path = entry.path.clone();
        if !self.marked.remove(&path) {
            self.marked.insert(path);
        }
    }

    pub fn is_marked(&self, path: &Path) -> bool {
        self.marked.contains(path)
    }

    /// All currently marked paths, sorted for deterministic ordering (a `HashSet` has none).
    pub fn marked_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self.marked.iter().cloned().collect();
        paths.sort();
        paths
    }

    /// Drops any mark whose path no longer exists — call after a bulk action that may have
    /// deleted marked entries, since `BrowserState` has no other way to learn of that.
    pub fn prune_marks(&mut self, vfs: &dyn Vfs) {
        self.marked.retain(|path| vfs.exists(path));
    }

    /// Entries of the selected item, if it's a directory — used to render the preview pane.
    pub fn preview_entries(&self, vfs: &dyn Vfs) -> Vec<DirEntryInfo> {
        match self.selected_entry() {
            Some(entry) if entry.is_dir => vfs.list_dir(&entry.path).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Selects `index` directly, clamped to the current entry count. No-op on an empty listing.
    /// Backs both the `/` search jump and `Esc`-cancel-restores-original-position.
    pub fn select_index(&mut self, index: usize) {
        if !self.current_entries.is_empty() {
            self.selected = index.min(self.current_entries.len() - 1);
        }
    }

    /// Index of the first entry (top to bottom) whose name contains `query`, case-insensitively.
    /// Always searches from the top rather than from the current position, so backspacing a `/`
    /// search back to a shorter query re-finds the same match deterministically.
    pub fn find_match(&self, query: &str) -> Option<usize> {
        if query.is_empty() {
            return None;
        }
        let needle = query.to_lowercase();
        self.current_entries
            .iter()
            .position(|e| e.name.to_lowercase().contains(&needle))
    }

    /// Jumps directly to `path` (resolved relative to the current directory if not absolute),
    /// as if the user had navigated there via repeated `enter`/`leave`. Backs the `:cd` command.
    pub fn goto(&mut self, vfs: &dyn Vfs, path: &Path) -> Result<(), VfsError> {
        let target = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.current_dir.join(path)
        };
        if !vfs.is_dir(&target) {
            return Err(VfsError::NotADirectory(target));
        }

        self.current_dir = target;
        self.selected = 0;
        self.refresh(vfs)
    }

    pub fn move_down(&mut self) {
        if !self.current_entries.is_empty() {
            self.selected = (self.selected + 1).min(self.current_entries.len() - 1);
        }
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Descend into the selected entry, if it's a directory.
    pub fn enter(&mut self, vfs: &dyn Vfs) -> Result<(), VfsError> {
        let Some(entry) = self.selected_entry() else {
            return Ok(());
        };
        if !entry.is_dir {
            return Ok(());
        }

        self.current_dir = entry.path.clone();
        self.selected = 0;
        self.refresh(vfs)
    }

    /// Move up to the parent directory, restoring the selection to the directory we came from.
    pub fn leave(&mut self, vfs: &dyn Vfs) -> Result<(), VfsError> {
        let Some(parent) = self.current_dir.parent().map(|p| p.to_path_buf()) else {
            return Ok(());
        };
        let came_from = self.current_dir.clone();

        self.current_dir = parent;
        self.refresh(vfs)?;

        self.selected = self
            .current_entries
            .iter()
            .position(|e| e.path == came_from)
            .unwrap_or(0);

        Ok(())
    }

    /// Re-lists the current and parent directories, clamping the selection if the entry count
    /// shrank. Call after a filesystem mutation made outside `BrowserState` (copy/move/delete/
    /// create/rename), since those don't otherwise update this state.
    pub fn reload(&mut self, vfs: &dyn Vfs) -> Result<(), VfsError> {
        self.refresh(vfs)
    }

    fn refresh(&mut self, vfs: &dyn Vfs) -> Result<(), VfsError> {
        self.current_entries = vfs.list_dir(&self.current_dir)?;
        self.selected = self
            .selected
            .min(self.current_entries.len().saturating_sub(1));

        self.parent_entries = match self.current_dir.parent() {
            Some(parent) => vfs.list_dir(parent).unwrap_or_default(),
            None => Vec::new(),
        };

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::LocalVfs;

    fn make_tree() -> PathBuf {
        // `cargo test` runs tests on separate threads within the same process, so
        // `std::process::id()` alone is identical across tests and would race on one path.
        let root = std::env::temp_dir().join(format!(
            "minuteman-browser-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("file.txt"), b"hi").unwrap();
        root
    }

    #[test]
    fn enter_and_leave_round_trip_restores_selection() {
        let root = make_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();

        assert_eq!(state.selected_entry().unwrap().name, "sub");

        state.enter(&vfs).unwrap();
        assert_eq!(state.current_dir(), root.join("sub"));
        assert_eq!(state.selected_entry().unwrap().name, "file.txt");

        state.leave(&vfs).unwrap();
        assert_eq!(state.current_dir(), root);
        assert_eq!(state.selected_entry().unwrap().name, "sub");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn move_down_does_not_go_past_last_entry() {
        let root = make_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();

        for _ in 0..10 {
            state.move_down();
        }
        assert_eq!(state.selected_index(), state.current_entries().len() - 1);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn select_index_clamps_to_last_entry() {
        let root = make_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();

        state.select_index(50);
        assert_eq!(state.selected_index(), state.current_entries().len() - 1);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_match_is_case_insensitive_and_searches_from_top() {
        let root = make_tree();
        let vfs = LocalVfs;
        let state = BrowserState::new(&vfs, root.clone()).unwrap();

        assert_eq!(state.find_match("SUB"), Some(0));
        assert_eq!(state.find_match("nope"), None);
        assert_eq!(state.find_match(""), None);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn goto_jumps_to_an_arbitrary_directory() {
        let root = make_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();

        state.goto(&vfs, &root.join("sub")).unwrap();
        assert_eq!(state.current_dir(), root.join("sub"));
        assert_eq!(state.selected_entry().unwrap().name, "file.txt");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn toggle_mark_adds_then_removes_the_selected_path() {
        let root = make_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();
        let sub_path = state.selected_entry().unwrap().path.clone();

        assert!(!state.is_marked(&sub_path));
        state.toggle_mark();
        assert!(state.is_marked(&sub_path));
        assert_eq!(state.marked_paths(), vec![sub_path.clone()]);

        state.toggle_mark();
        assert!(!state.is_marked(&sub_path));
        assert!(state.marked_paths().is_empty());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn marks_persist_across_navigation_and_prune_drops_missing_paths() {
        let root = make_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();
        let sub_path = state.selected_entry().unwrap().path.clone();
        state.toggle_mark();

        state.enter(&vfs).unwrap();
        assert!(state.is_marked(&sub_path));

        std::fs::remove_dir_all(&sub_path).unwrap();
        state.leave(&vfs).unwrap();
        state.prune_marks(&vfs);
        assert!(!state.is_marked(&sub_path));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn goto_rejects_a_non_directory() {
        let root = make_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();

        let result = state.goto(&vfs, &root.join("sub").join("file.txt"));
        assert!(matches!(result, Err(VfsError::NotADirectory(_))));
        assert_eq!(state.current_dir(), root);

        std::fs::remove_dir_all(&root).unwrap();
    }
}
