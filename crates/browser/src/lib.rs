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

pub mod finder;
pub mod search;
pub mod sort;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use shared::{DirEntryInfo, Vfs, VfsError};
use sort::Sort;

#[derive(Debug)]
pub struct BrowserState {
    current_dir: PathBuf,
    parent_entries: Vec<DirEntryInfo>,
    current_entries: Vec<DirEntryInfo>,
    selected: usize,
    /// Ranger-style marks: paths toggled via `Select`, persisting across navigation until
    /// explicitly toggled off or consumed by a bulk action (e.g. `Delete`).
    marked: HashSet<PathBuf>,
    /// The entry a range mark (`mark_range_to`) starts from: the last one clicked or toggled by
    /// mouse. Held as a path, not an index, so it survives a re-sort or a refresh; one that is no
    /// longer in the listing is ignored.
    range_anchor: Option<PathBuf>,
    /// Whether dot-prefixed entries appear in the listings. Filtering happens when a listing is
    /// stored, not when it is drawn, so `selected`, `/` search and mouse hit-testing all index
    /// the same list the user sees.
    show_hidden: bool,
    /// How both listings are ordered; applied whenever one is stored.
    sort: Sort,
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

/// Where the cursor lands once a move finishes.
#[derive(Debug, Clone)]
enum Landing {
    First,
    /// The entry with this path, or the first one if it is not listed.
    On(PathBuf),
}

#[derive(Debug, Clone)]
enum MoveKind {
    /// A folder already known to be one (the selected entry, or the parent).
    Into(PathBuf),
    /// Whatever the user typed or a bookmark holds: resolved against the folder moved from, then
    /// checked to be a folder, before anything is listed.
    Typed(PathBuf),
}

/// A move to another folder, described but not yet done. Making the plan is instant; [`load`]
/// does the filesystem work and may be slow (a remote machine), so it can run anywhere, and
/// [`BrowserState::apply_move`] then swaps the result in at once. The browser keeps showing the
/// old folder, with its cursor and marks valid, until that swap.
///
/// [`load`]: MovePlan::load
#[derive(Debug, Clone)]
pub struct MovePlan {
    from: PathBuf,
    kind: MoveKind,
    landing: Landing,
}

/// The listings a [`MovePlan`] read, ready for [`BrowserState::apply_move`].
#[derive(Debug)]
pub struct LoadedMove {
    from: PathBuf,
    dir: PathBuf,
    current: Vec<DirEntryInfo>,
    parent: Vec<DirEntryInfo>,
    landing: Landing,
}

impl MovePlan {
    /// Puts the cursor on `path` instead of the first entry once the move is applied.
    #[must_use]
    pub fn landing_on(mut self, path: PathBuf) -> Self {
        self.landing = Landing::On(path);
        self
    }

    /// A short name for where the move goes, for a status message.
    pub fn target_label(&self) -> String {
        let path = match &self.kind {
            MoveKind::Into(path) | MoveKind::Typed(path) => path,
        };
        path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        )
    }

    /// Reads the target folder and its parent. Leaves the browser alone, so a failure — an
    /// unreachable machine, a folder that cannot be read, a typed path that is not a folder —
    /// strands nothing.
    ///
    /// # Errors
    ///
    /// The [`VfsError`] from resolving, checking or listing the target.
    pub fn load(&self, vfs: &dyn Vfs) -> Result<LoadedMove, VfsError> {
        let dir = match &self.kind {
            MoveKind::Into(dir) => dir.clone(),
            MoveKind::Typed(typed) => {
                let target = vfs.resolve_typed(typed, &self.from);
                // `metadata` rather than `is_dir`, whose plain yes or no would turn "cannot reach
                // that machine" into "not a directory".
                match vfs.metadata(&target) {
                    Ok(meta) if meta.kind == shared::FileKind::Dir => {}
                    Ok(_) | Err(VfsError::NotFound(_)) => {
                        return Err(VfsError::NotADirectory(target));
                    }
                    Err(other) => return Err(other),
                }
                target
            }
        };
        let current = vfs.list_dir(&dir)?;
        let parent = match dir.parent() {
            Some(parent) => vfs.list_dir(parent).unwrap_or_default(),
            None => Vec::new(),
        };
        Ok(LoadedMove {
            from: self.from.clone(),
            dir,
            current,
            parent,
            landing: self.landing.clone(),
        })
    }
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
            range_anchor: None,
            show_hidden,
            sort: Sort::default(),
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

    /// Makes the selected entry the start of the next range mark.
    pub fn anchor_here(&mut self) {
        self.range_anchor = self.selected_entry().map(|entry| entry.path.clone());
    }

    /// Selects `index` and toggles its mark, the way a `Ctrl`-click does; it also becomes the
    /// range anchor. No-op on an empty listing.
    pub fn toggle_mark_at(&mut self, index: usize) {
        self.select_index(index);
        self.toggle_mark();
        self.anchor_here();
    }

    /// Selects `index` and marks every entry from the range anchor to it, inclusive and in
    /// either direction, keeping whatever was already marked. Without a usable anchor the entry
    /// under the cursor before the click stands in for it. The anchor stays put, so a second
    /// range click re-spans from the same start. No-op on an empty listing.
    pub fn mark_range_to(&mut self, index: usize) {
        if self.current_entries.is_empty() {
            return;
        }
        let from = self
            .range_anchor
            .as_ref()
            .and_then(|anchor| self.current_entries.iter().position(|e| &e.path == anchor))
            .unwrap_or(self.selected);
        self.select_index(index);
        let (low, high) = if from <= self.selected {
            (from, self.selected)
        } else {
            (self.selected, from)
        };
        let paths: Vec<PathBuf> = self.current_entries[low..=high]
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        self.marked.extend(paths);
        if self.range_anchor.is_none() {
            self.range_anchor = self.current_entries.get(from).map(|e| e.path.clone());
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
            Some(entry) if entry.is_dir => {
                self.arrange(vfs.list_dir(&entry.path).unwrap_or_default(), None)
            }
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
    /// The index of the best fuzzy match for `query` among the entries on screen: its letters in
    /// order, not necessarily side by side (see `search`). Always ranks the whole listing rather
    /// than searching from the current position, so backspacing a `/` search back to a shorter
    /// query re-finds the same match deterministically.
    pub fn find_match(&self, query: &str) -> Option<usize> {
        let query = search::Query::new(query)?;
        let best = search::rank_entries(&query, &self.current_entries)
            .into_iter()
            .next()?;
        self.current_entries.iter().position(|e| e.path == best)
    }

    /// Jumps directly to `path` (resolved by `Vfs::resolve_typed`, so a relative one is taken from the
    /// current directory),
    /// as if the user had navigated there via repeated `enter`/`leave`. Backs the `:cd` command.
    pub fn goto(&mut self, vfs: &dyn Vfs, path: &Path) -> Result<(), VfsError> {
        let loaded = self.plan_goto(path).load(vfs)?;
        self.apply_move(loaded);
        Ok(())
    }

    /// The move [`goto`](Self::goto) makes, without making it.
    pub fn plan_goto(&self, path: &Path) -> MovePlan {
        MovePlan {
            from: self.current_dir.clone(),
            kind: MoveKind::Typed(path.to_path_buf()),
            landing: Landing::First,
        }
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
        match self.plan_enter() {
            Some(plan) => self.run(vfs, &plan),
            None => Ok(()),
        }
    }

    /// The move [`enter`](Self::enter) makes, without making it; `None` when the selection is
    /// not a folder.
    pub fn plan_enter(&self) -> Option<MovePlan> {
        let entry = self.selected_entry().filter(|entry| entry.is_dir)?;
        Some(MovePlan {
            from: self.current_dir.clone(),
            kind: MoveKind::Into(entry.path.clone()),
            landing: Landing::First,
        })
    }

    /// Move up to the parent directory, restoring the selection to the directory we came from.
    pub fn leave(&mut self, vfs: &dyn Vfs) -> Result<(), VfsError> {
        match self.plan_leave() {
            Some(plan) => self.run(vfs, &plan),
            None => Ok(()),
        }
    }

    /// The move [`leave`](Self::leave) makes, without making it; `None` at the root. The cursor
    /// lands on the folder left — or, stepping out of an archive's root, on the archive file the
    /// root was made from.
    pub fn plan_leave(&self) -> Option<MovePlan> {
        let parent = self.current_dir.parent()?.to_path_buf();
        let came_from =
            shared::archive_of_root(&self.current_dir).unwrap_or_else(|| self.current_dir.clone());
        Some(MovePlan {
            from: self.current_dir.clone(),
            kind: MoveKind::Into(parent),
            landing: Landing::On(came_from),
        })
    }

    /// Loads and applies `plan` on this thread. A folder that cannot be read leaves everything as
    /// it was, so one (or a parent that is not a folder at all, like the step above a remote
    /// machine's root) never strands the browser on it.
    fn run(&mut self, vfs: &dyn Vfs, plan: &MovePlan) -> Result<(), VfsError> {
        let loaded = plan.load(vfs)?;
        self.apply_move(loaded);
        Ok(())
    }

    /// Makes a loaded move the current folder, with the cursor where its plan said. Returns
    /// whether it was applied: a move planned from a folder the browser has since left is
    /// dropped, so a slow one can never override a newer one.
    pub fn apply_move(&mut self, loaded: LoadedMove) -> bool {
        if loaded.from != self.current_dir {
            return false;
        }
        self.current_entries = self.arrange(loaded.current, None);
        self.parent_entries = self.arrange(loaded.parent, Some(&loaded.dir));
        self.current_dir = loaded.dir;
        self.selected = match loaded.landing {
            Landing::First => 0,
            Landing::On(path) => self
                .current_entries
                .iter()
                .position(|entry| entry.path == path)
                .unwrap_or(0),
        };
        true
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
        let current = self.arrange(current, None);
        let parent = self.arrange(parent, Some(&self.current_dir));
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
        self.current_entries = self.arrange(vfs.list_dir(&self.current_dir)?, None);
        self.selected = self
            .selected
            .min(self.current_entries.len().saturating_sub(1));

        self.parent_entries = match self.current_dir.parent() {
            Some(parent) => self.arrange(
                vfs.list_dir(parent).unwrap_or_default(),
                Some(&self.current_dir),
            ),
            None => Vec::new(),
        };

        Ok(())
    }

    /// What a listing from the `Vfs` becomes when stored: hidden entries dropped (unless shown),
    /// then put in the current order.
    fn arrange(&self, entries: Vec<DirEntryInfo>, keep: Option<&Path>) -> Vec<DirEntryInfo> {
        let mut entries = filter_hidden(entries, self.show_hidden, keep);
        self.sort.sort(&mut entries);
        entries
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }

    /// Re-orders both listings in place, keeping the cursor on the same entry. No folder is read.
    pub fn set_sort(&mut self, sort: Sort) {
        if sort == self.sort {
            return;
        }
        let previous = self.selected_path();
        self.sort = sort;
        sort.sort(&mut self.current_entries);
        sort.sort(&mut self.parent_entries);
        self.reselect(previous);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
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

    fn sized_tree(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "minuteman-browser-sort-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("dir")).unwrap();
        std::fs::write(root.join("small"), b"1").unwrap();
        std::fs::write(root.join("medium"), vec![0u8; 100]).unwrap();
        std::fs::write(root.join("large"), vec![0u8; 10_000]).unwrap();
        root
    }

    fn names_of(browser: &BrowserState) -> Vec<String> {
        browser
            .current_entries()
            .iter()
            .map(|e| e.name.clone())
            .collect()
    }

    #[test]
    fn changing_the_sort_reorders_in_place_and_keeps_the_cursor_on_its_entry() {
        let root = sized_tree("keep");
        let mut browser = BrowserState::new(&LocalVfs, root.clone()).unwrap();
        assert_eq!(names_of(&browser), ["dir", "large", "medium", "small"]);
        browser.select_index(
            browser
                .current_entries()
                .iter()
                .position(|e| e.name == "medium")
                .unwrap(),
        );

        browser.set_sort(Sort::default().clicked(sort::SortKey::Size));
        assert_eq!(names_of(&browser), ["dir", "large", "medium", "small"]);
        browser.set_sort(
            Sort::default()
                .clicked(sort::SortKey::Size)
                .clicked(sort::SortKey::Size),
        );
        assert_eq!(names_of(&browser), ["dir", "small", "medium", "large"]);
        assert_eq!(
            browser.selected_entry().map(|e| e.name.as_str()),
            Some("medium"),
            "the cursor follows its entry"
        );
    }

    #[test]
    fn the_sort_survives_a_reload_a_move_and_a_background_listing() {
        let root = sized_tree("survive");
        let mut browser = BrowserState::new(&LocalVfs, root.clone()).unwrap();
        let by_size_up = Sort::default()
            .clicked(sort::SortKey::Size)
            .clicked(sort::SortKey::Size);
        browser.set_sort(by_size_up);

        browser.reload(&LocalVfs).unwrap();
        assert_eq!(names_of(&browser), ["dir", "small", "medium", "large"]);

        std::fs::write(root.join("tiny"), b"").unwrap();
        let fresh = LocalVfs.list_dir(&root).unwrap();
        let parent = LocalVfs.list_dir(root.parent().unwrap()).unwrap();
        assert!(browser.apply_listing(&root, fresh, parent));
        assert_eq!(
            names_of(&browser),
            ["dir", "tiny", "small", "medium", "large"]
        );

        browser.goto(&LocalVfs, &root.join("dir")).unwrap();
        browser.leave(&LocalVfs).unwrap();
        assert_eq!(browser.sort(), by_size_up);
        assert_eq!(names_of(&browser)[1], "tiny");
    }

    #[test]
    fn setting_the_sort_it_already_has_changes_nothing() {
        let root = sized_tree("same");
        let mut browser = BrowserState::new(&LocalVfs, root).unwrap();
        browser.select_index(2);
        browser.set_sort(Sort::default());
        assert_eq!(browser.selected_index(), 2);
    }

    #[test]
    fn a_planned_move_changes_nothing_until_it_is_applied() {
        let root = make_tree();
        let mut browser = BrowserState::new(&LocalVfs, root.clone()).unwrap();
        browser.select_index(
            browser
                .current_entries()
                .iter()
                .position(|e| e.name == "sub")
                .unwrap(),
        );

        let plan = browser.plan_enter().expect("sub is a folder");
        let loaded = plan.load(&LocalVfs).unwrap();
        assert_eq!(
            browser.current_dir(),
            root,
            "loading leaves the browser alone"
        );

        assert!(browser.apply_move(loaded));
        assert_eq!(browser.current_dir(), root.join("sub"));
        assert_eq!(browser.selected_index(), 0);
    }

    #[test]
    fn a_move_planned_from_a_folder_since_left_is_dropped() {
        let root = make_tree();
        let mut browser = BrowserState::new(&LocalVfs, root.clone()).unwrap();
        let stale = browser
            .plan_goto(&root.join("sub"))
            .load(&LocalVfs)
            .unwrap();
        browser.goto(&LocalVfs, &root.join("sub")).unwrap();
        browser.leave(&LocalVfs).unwrap();
        browser.goto(&LocalVfs, &root.join("sub")).unwrap();

        // Planned from `root`, applied while in `sub`.
        assert!(!browser.apply_move(stale));
        assert_eq!(browser.current_dir(), root.join("sub"));
    }

    #[test]
    fn a_move_that_cannot_load_leaves_the_browser_where_it_was() {
        let root = make_tree();
        let mut browser = BrowserState::new(&LocalVfs, root.clone()).unwrap();
        let missing = root.join("nope");
        assert!(browser.plan_goto(&missing).load(&LocalVfs).is_err());
        assert!(browser.goto(&LocalVfs, &missing).is_err());
        assert_eq!(browser.current_dir(), root);

        let file = root.join("sub").join("file.txt");
        assert!(matches!(
            browser.goto(&LocalVfs, &file),
            Err(VfsError::NotADirectory(_))
        ));
        assert_eq!(browser.current_dir(), root);
    }

    #[test]
    fn leaving_lands_on_the_folder_left_and_a_plan_can_choose_another_landing() {
        let root = make_tree();
        std::fs::create_dir_all(root.join("zzz")).unwrap();
        let mut browser = BrowserState::new(&LocalVfs, root.join("sub")).unwrap();

        let plan = browser.plan_leave().expect("sub has a parent");
        let sub_row = {
            let loaded = plan.load(&LocalVfs).unwrap();
            loaded.current.iter().position(|e| e.name == "sub")
        };
        browser.apply_move(plan.load(&LocalVfs).unwrap());
        assert_eq!(Some(browser.selected_index()), sub_row);

        browser.goto(&LocalVfs, &root.join("sub")).unwrap();
        let other = root.join("zzz");
        let plan = browser.plan_leave().unwrap().landing_on(other.clone());
        browser.apply_move(plan.load(&LocalVfs).unwrap());
        assert_eq!(browser.selected_entry().map(|e| &e.path), Some(&other));
    }

    #[test]
    fn nothing_to_plan_on_a_file_or_at_the_root() {
        let root = make_tree();
        let mut browser = BrowserState::new(&LocalVfs, root.join("sub")).unwrap();
        assert!(browser.plan_enter().is_none(), "file.txt is not a folder");
        let mut at_root = BrowserState::new(&LocalVfs, PathBuf::from("/")).unwrap();
        assert!(at_root.plan_leave().is_none());
        at_root.leave(&LocalVfs).unwrap();
        browser.enter(&LocalVfs).unwrap();
        assert_eq!(browser.current_dir(), root.join("sub"));
    }

    #[test]
    #[cfg(unix)]
    fn leaving_into_an_unlistable_parent_stays_where_it_was() {
        use std::os::unix::fs::PermissionsExt;

        let root = make_tree();
        let mut browser = BrowserState::new(&LocalVfs, root.join("sub")).unwrap();
        // Searchable but not listable: the child can be read, the parent cannot.
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o111)).unwrap();
        let listable = LocalVfs.list_dir(&root).is_ok();

        let left = browser.leave(&LocalVfs);

        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        if listable {
            // Permissions do not bind this user (root), so there is nothing to observe.
            return;
        }
        assert!(left.is_err());
        assert_eq!(browser.current_dir(), root.join("sub"));
        assert_eq!(
            browser
                .current_entries()
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["file.txt"]
        );
    }

    #[test]
    #[cfg(unix)]
    fn entering_an_unreadable_folder_stays_where_it_was() {
        use std::os::unix::fs::PermissionsExt;

        let root = make_tree();
        let mut browser = BrowserState::new(&LocalVfs, root.clone()).unwrap();
        let sub = root.join("sub");
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
        let listable = LocalVfs.list_dir(&sub).is_ok();

        let entered = browser.enter(&LocalVfs);

        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
        if listable {
            return;
        }
        assert!(entered.is_err());
        assert_eq!(browser.current_dir(), root);
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

    /// A directory of `count` files named so they list in a known order.
    fn make_files(tag: &str, count: usize) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "minuteman-browser-range-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..count {
            std::fs::write(root.join(format!("f{i:02}")), b"x").unwrap();
        }
        root
    }

    #[test]
    fn toggle_mark_at_twice_restores_the_marks() {
        let root = make_files("toggle", 6);
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();

        state.toggle_mark_at(3);
        assert_eq!(state.marked_paths().len(), 1);
        assert_eq!(state.selected_index(), 3);
        state.toggle_mark_at(3);
        assert!(state.marked_paths().is_empty());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn range_without_an_anchor_starts_at_the_cursor() {
        let root = make_files("noanchor", 6);
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();
        state.select_index(1);

        state.mark_range_to(3);
        let names: Vec<_> = state
            .marked_paths()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["f01", "f02", "f03"]);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn range_ignores_an_anchor_that_left_the_listing() {
        let root = make_files("stale", 4);
        let vfs = LocalVfs;
        let mut state = BrowserState::new(&vfs, root.clone()).unwrap();
        state.select_index(0);
        state.anchor_here();
        std::fs::remove_file(root.join("f00")).unwrap();
        state.refresh(&vfs).unwrap();
        state.select_index(1);

        state.mark_range_to(2);
        assert_eq!(state.marked_paths().len(), 2);

        std::fs::remove_dir_all(&root).unwrap();
    }

    proptest! {
        #[test]
        fn range_marks_exactly_the_span_between_anchor_and_click(
            anchor in 0usize..10,
            click in 0usize..10,
            before in proptest::collection::vec(0usize..10, 0..4),
        ) {
            let root = make_files("prop", 10);
            let vfs = LocalVfs;
            let mut state = BrowserState::new(&vfs, root.clone()).unwrap();
            let paths: Vec<PathBuf> =
                state.current_entries().iter().map(|e| e.path.clone()).collect();
            for i in &before {
                state.select_index(*i);
                if !state.is_marked(&paths[*i]) {
                    state.toggle_mark();
                }
            }
            state.select_index(anchor);
            state.anchor_here();

            state.mark_range_to(click);

            let (low, high) = (anchor.min(click), anchor.max(click));
            for (i, path) in paths.iter().enumerate() {
                let expected = (low..=high).contains(&i) || before.contains(&i);
                prop_assert_eq!(state.is_marked(path), expected, "entry {}", i);
            }
            prop_assert_eq!(state.selected_index(), click);
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    #[test]
    fn find_match_is_case_insensitive_and_searches_from_top() {
        let root = make_tree();
        let vfs = LocalVfs;
        let state = BrowserState::new(&vfs, root.clone()).unwrap();

        assert_eq!(state.find_match("SUB"), Some(0));
        assert_eq!(
            state.find_match("sb"),
            Some(0),
            "letters in order are enough"
        );
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
