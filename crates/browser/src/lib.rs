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

pub mod search;

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
    /// Whether dot-prefixed entries appear in the listings. Filtering happens when a listing is
    /// stored, not when it is drawn, so `selected`, `/` search and mouse hit-testing all index
    /// the same list the user sees.
    show_hidden: bool,
}

/// A dot-prefixed name is hidden, the Unix convention every shell and file manager shares.
fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Drops hidden entries unless `show_hidden`, but never `keep`: the parent column must still
/// list the directory being browsed even when that directory is itself hidden, or the column
/// would have nothing to highlight.
fn filter_hidden(
    entries: Vec<DirEntryInfo>,
    show_hidden: bool,
    keep: Option<&Path>,
) -> Vec<DirEntryInfo> {
    if show_hidden {
        return entries;
    }
    entries
        .into_iter()
        .filter(|e| !is_hidden(&e.name) || keep == Some(e.path.as_path()))
        .collect()
}

impl BrowserState {
    /// A browser that shows hidden entries. Use `with_show_hidden` to start with them filtered.
    pub fn new(vfs: &dyn Vfs, start_dir: PathBuf) -> Result<Self, VfsError> {
        Self::with_show_hidden(vfs, start_dir, true)
    }

    pub fn with_show_hidden(
        vfs: &dyn Vfs,
        start_dir: PathBuf,
        show_hidden: bool,
    ) -> Result<Self, VfsError> {
        let mut state = Self {
            current_dir: start_dir,
            parent_entries: Vec::new(),
            current_entries: Vec::new(),
            selected: 0,
            marked: HashSet::new(),
            show_hidden,
        };
        state.refresh(vfs)?;
        Ok(state)
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// Flips whether hidden entries are listed and re-lists, keeping the cursor on the same
    /// entry when it is still visible. Returns the new setting.
    pub fn toggle_hidden(&mut self, vfs: &dyn Vfs) -> Result<bool, VfsError> {
        self.show_hidden = !self.show_hidden;
        self.reload(vfs)?;
        Ok(self.show_hidden)
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

    /// Forgets every mark and returns how many there were.
    pub fn clear_marks(&mut self) -> usize {
        let count = self.marked.len();
        self.marked.clear();
        count
    }

    /// Drops any mark whose path no longer exists — call after a bulk action that may have
    /// deleted marked entries, since `BrowserState` has no other way to learn of that.
    pub fn prune_marks(&mut self, vfs: &dyn Vfs) {
        self.marked.retain(|path| vfs.exists(path));
    }

    /// Entries of the selected item, if it's a directory — used to render the preview pane.
    pub fn preview_entries(&self, vfs: &dyn Vfs) -> Vec<DirEntryInfo> {
        match self.selected_entry() {
            Some(entry) if entry.is_dir => filter_hidden(
                vfs.list_dir(&entry.path).unwrap_or_default(),
                self.show_hidden,
                None,
            ),
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

    /// Opens the directory `path` is in and puts the cursor on it — how a search hit far from
    /// the browsed directory is shown. Stays put in the directory it is already in, so the
    /// listing isn't re-read for a hit right here. A path with no parent (the root) is left
    /// alone.
    pub fn reveal(&mut self, vfs: &dyn Vfs, path: &Path) -> Result<(), VfsError> {
        let Some(parent) = path.parent() else {
            return Ok(());
        };
        if parent != self.current_dir {
            self.goto(vfs, parent)?;
        }
        if let Some(index) = self.current_entries.iter().position(|e| e.path == path) {
            self.selected = index;
        }
        Ok(())
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

    /// Re-lists the current and parent directories. The cursor stays on the same entry by path
    /// (falling back to the same index, clamped, if that entry is gone), so a file appearing
    /// above it doesn't make the selection jump. Call after a filesystem mutation made outside
    /// `BrowserState` (copy/move/delete/create/rename), since those don't otherwise update it.
    pub fn reload(&mut self, vfs: &dyn Vfs) -> Result<(), VfsError> {
        let previous = self.selected_path();
        self.refresh(vfs)?;
        self.reselect(previous);
        Ok(())
    }

    /// Swaps in listings fetched elsewhere (e.g. on a background thread) if they differ from
    /// what is stored, and returns whether anything changed. `dir` is the directory `current`
    /// was read from: a result for a directory the user has since left is dropped, so a slow
    /// listing can never overwrite a newer one. Both listings are unfiltered, as `Vfs` returns
    /// them.
    pub fn apply_listing(
        &mut self,
        dir: &Path,
        current: Vec<DirEntryInfo>,
        parent: Vec<DirEntryInfo>,
    ) -> bool {
        if dir != self.current_dir {
            return false;
        }
        let current = filter_hidden(current, self.show_hidden, None);
        let parent = filter_hidden(parent, self.show_hidden, Some(&self.current_dir));
        if current == self.current_entries && parent == self.parent_entries {
            return false;
        }

        let previous = self.selected_path();
        self.current_entries = current;
        self.parent_entries = parent;
        self.reselect(previous);
        true
    }

    fn selected_path(&self) -> Option<PathBuf> {
        self.selected_entry().map(|e| e.path.clone())
    }

    /// Moves the cursor back onto `previous` if it is still listed, else clamps the old index.
    fn reselect(&mut self, previous: Option<PathBuf>) {
        let found = previous.and_then(|path| {
            self.current_entries
                .iter()
                .position(|entry| entry.path == path)
        });
        self.selected = found.unwrap_or_else(|| {
            self.selected
                .min(self.current_entries.len().saturating_sub(1))
        });
    }

    fn refresh(&mut self, vfs: &dyn Vfs) -> Result<(), VfsError> {
        self.current_entries =
            filter_hidden(vfs.list_dir(&self.current_dir)?, self.show_hidden, None);
        self.selected = self
            .selected
            .min(self.current_entries.len().saturating_sub(1));

        self.parent_entries = match self.current_dir.parent() {
            Some(parent) => filter_hidden(
                vfs.list_dir(parent).unwrap_or_default(),
                self.show_hidden,
                Some(&self.current_dir),
            ),
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
    fn reveal_opens_the_parent_directory_and_selects_the_entry() {
        let root = make_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();

        state
            .reveal(&vfs, &root.join("sub").join("file.txt"))
            .unwrap();
        assert_eq!(state.current_dir(), root.join("sub"));
        assert_eq!(state.selected_entry().unwrap().name, "file.txt");

        // Already in that directory: the cursor just moves.
        state
            .reveal(&vfs, &root.join("sub").join("file.txt"))
            .unwrap();
        assert_eq!(state.current_dir(), root.join("sub"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reveal_fails_cleanly_when_the_directory_is_gone() {
        let root = make_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();

        let result = state.reveal(&vfs, &root.join("vanished").join("x"));
        assert!(result.is_err());
        assert_eq!(
            state.current_dir(),
            root,
            "a failed reveal must not move the browser"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn clear_marks_forgets_every_mark_and_counts_them() {
        let root = make_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();

        assert_eq!(state.clear_marks(), 0);
        state.toggle_mark();
        assert_eq!(state.clear_marks(), 1);
        assert!(state.marked_paths().is_empty());

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

    fn make_hidden_tree() -> PathBuf {
        let root = make_tree();
        std::fs::write(root.join(".secret"), b"hi").unwrap();
        std::fs::create_dir_all(root.join(".dotdir")).unwrap();
        std::fs::write(root.join(".dotdir").join(".inner"), b"hi").unwrap();
        std::fs::write(root.join(".dotdir").join("visible"), b"hi").unwrap();
        root
    }

    fn names(entries: &[DirEntryInfo]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn hidden_entries_are_filtered_from_every_listing_until_toggled_on() {
        let root = make_hidden_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::with_show_hidden(&vfs, root.clone(), false).unwrap();

        assert_eq!(names(state.current_entries()), vec!["sub"]);

        assert!(state.toggle_hidden(&vfs).unwrap());
        assert_eq!(
            names(state.current_entries()),
            vec![".dotdir", "sub", ".secret"]
        );

        assert!(!state.toggle_hidden(&vfs).unwrap());
        assert_eq!(names(state.current_entries()), vec!["sub"]);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn preview_entries_honour_the_hidden_setting() {
        let root = make_hidden_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::with_show_hidden(&vfs, root.clone(), true).unwrap();
        assert_eq!(state.selected_entry().unwrap().name, ".dotdir");
        assert_eq!(
            names(&state.preview_entries(&vfs)),
            vec![".inner", "visible"]
        );

        // Hiding `.dotdir` moves the cursor to `sub`; make `.dotdir` the cursor again by way of
        // `select_index` while hidden entries are shown, then compare its filtered preview.
        state.toggle_hidden(&vfs).unwrap();
        state.toggle_hidden(&vfs).unwrap();
        state.select_index(0);
        assert_eq!(state.selected_entry().unwrap().name, ".dotdir");
        let shown = state.preview_entries(&vfs).len();
        assert_eq!(shown, 2);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_parent_column_still_lists_the_hidden_directory_being_browsed() {
        let root = make_hidden_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::with_show_hidden(&vfs, root.clone(), true).unwrap();
        state.enter(&vfs).unwrap();
        assert_eq!(state.current_dir(), root.join(".dotdir"));

        state.toggle_hidden(&vfs).unwrap();

        assert_eq!(names(state.current_entries()), vec!["visible"]);
        assert!(names(state.parent_entries()).contains(&".dotdir"));
        assert!(!names(state.parent_entries()).contains(&".secret"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn toggling_hidden_keeps_the_cursor_on_the_same_entry() {
        let root = make_hidden_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::with_show_hidden(&vfs, root.clone(), false).unwrap();
        assert_eq!(state.selected_entry().unwrap().name, "sub");

        state.toggle_hidden(&vfs).unwrap();

        assert_eq!(state.selected_entry().unwrap().name, "sub");
        assert_eq!(state.selected_index(), 1);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reload_keeps_the_cursor_on_its_entry_when_a_new_one_sorts_above_it() {
        let root = make_tree();
        std::fs::write(root.join("zeta.txt"), b"hi").unwrap();
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();
        state.select_index(1);
        assert_eq!(state.selected_entry().unwrap().name, "zeta.txt");

        std::fs::write(root.join("alpha.txt"), b"hi").unwrap();
        state.reload(&vfs).unwrap();

        assert_eq!(state.selected_entry().unwrap().name, "zeta.txt");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn apply_listing_reports_a_change_only_when_the_listing_differs() {
        let root = make_tree();
        let dir = root.join("sub");
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, dir.clone()).unwrap();

        // `root` is the parent column here, and it is private to this test — unlike the shared
        // temp dir, nothing else mutates it while the test runs.
        let unchanged = (vfs.list_dir(&dir).unwrap(), vfs.list_dir(&root).unwrap());
        assert!(!state.apply_listing(&dir, unchanged.0, unchanged.1));

        std::fs::write(dir.join("new.txt"), b"hi").unwrap();
        let fresh = (vfs.list_dir(&dir).unwrap(), vfs.list_dir(&root).unwrap());
        assert!(state.apply_listing(&dir, fresh.0, fresh.1));
        assert_eq!(names(state.current_entries()), vec!["file.txt", "new.txt"]);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn apply_listing_notices_a_file_growing_in_place() {
        let root = make_tree();
        let dir = root.join("sub");
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, dir.clone()).unwrap();

        std::fs::write(dir.join("file.txt"), b"a much longer body").unwrap();
        let fresh = (vfs.list_dir(&dir).unwrap(), vfs.list_dir(&root).unwrap());

        assert!(state.apply_listing(&dir, fresh.0, fresh.1));
        assert_eq!(state.selected_entry().unwrap().size, 18);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn apply_listing_drops_a_result_for_a_directory_we_have_left() {
        let root = make_tree();
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();
        let stale = (vfs.list_dir(&root).unwrap(), Vec::new());
        state.enter(&vfs).unwrap();
        let before = state.current_entries().to_vec();

        assert!(!state.apply_listing(&root, stale.0, stale.1));
        assert_eq!(state.current_entries(), before.as_slice());

        std::fs::remove_dir_all(&root).unwrap();
    }
}
