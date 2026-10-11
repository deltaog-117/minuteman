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

//! A plugin's picker (the fuzzy finder's `f` is the first): what is typed, the newest list of
//! results the plugin sent and which result is selected. Like `settings_popup` it is pure: no
//! `Frame`, no terminal. `picker_view` draws it and `main` applies what a key asks for, which is
//! only ever to close it or to show a result in the browser, and passes what is typed on to the
//! plugin (see `take_query`).
//!
//! Nothing here scores or sorts anything: the plugin does. A key that edits the query records it
//! for sending and carries on; `apply` takes in whatever the plugin has answered. Until the answer
//! to a new query arrives the previous results stay on screen, so typing never flashes an empty
//! list. A folder's contents for the preview are read on the blocking pool for the same reason,
//! once per selection, never in the middle of drawing.
//!
//! Every key goes to the query, so `j` and `k` are letters here; the arrows and `Ctrl`-`N`/
//! `Ctrl`-`P` move through the results.

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use plugin_host::protocol::{PickerIndexing, PickerSpec, PickerUpdate, TreeEntry};
use shared::{DirEntryInfo, Vfs};

use crate::line_edit::LineEdit;

/// A folder's preview lists no more than this many entries: the pane cannot show more than a
/// screenful, and building rows for a folder of tens of thousands every frame would be the
/// stutter this exists to avoid.
const PREVIEW_ENTRIES: usize = 300;

/// What a keystroke asks the caller to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Close,
    /// Close and put the browser's cursor on this entry.
    Reveal(PathBuf),
}

pub struct PickerPopup {
    /// The plugin whose picker this is, and the number it gave it.
    plugin: String,
    picker: u64,
    title: String,
    preview: bool,
    query: LineEdit,
    /// Rises with every change of the query; the plugin's updates say which one they answer.
    generation: u64,
    /// The newest query not yet passed on to the plugin (see `take_query`).
    unsent: Option<(u64, String)>,
    update: Update,
    selected: usize,
    /// The path the selection is on, to keep it there when results arrive and reorder. `None`
    /// right after the query changes, when the old selection means nothing.
    anchor: Option<PathBuf>,
    /// The first row drawn and how many fit, kept by the renderer between frames the way the
    /// trash browser's are.
    top: Cell<usize>,
    viewport: Cell<usize>,
    handle: tokio::runtime::Handle,
    vfs: Arc<dyn Vfs>,
    /// The folder whose contents have been asked for, and the newest contents that came back.
    asked: Option<PathBuf>,
    children: Option<(PathBuf, Vec<DirEntryInfo>)>,
    children_tx: Sender<(PathBuf, Vec<DirEntryInfo>)>,
    children_rx: Receiver<(PathBuf, Vec<DirEntryInfo>)>,
}

/// What the plugin has said so far: the best results for the query it last ranked.
struct Update {
    generation: u64,
    results: Vec<TreeEntry>,
    matches: usize,
    indexed: usize,
    indexing: PickerIndexing,
    settled: bool,
}

impl Update {
    /// What the picker shows before the plugin has said anything.
    fn empty() -> Self {
        Self {
            generation: 0,
            results: Vec::new(),
            matches: 0,
            indexed: 0,
            indexing: PickerIndexing::Walking,
            settled: false,
        }
    }
}

impl PickerPopup {
    /// Opens the picker `plugin` asked for with `spec`.
    pub fn open(
        handle: &tokio::runtime::Handle,
        vfs: Arc<dyn Vfs>,
        plugin: &str,
        spec: &PickerSpec,
    ) -> Self {
        let (children_tx, children_rx) = channel();
        Self {
            plugin: plugin.to_string(),
            picker: spec.picker,
            title: spec.title.clone(),
            preview: spec.preview,
            query: LineEdit::default(),
            generation: 0,
            unsent: None,
            update: Update::empty(),
            selected: 0,
            anchor: None,
            top: Cell::new(0),
            viewport: Cell::new(0),
            handle: handle.clone(),
            vfs,
            asked: None,
            children: None,
            children_tx,
            children_rx,
        }
    }

    /// The plugin that owns this picker.
    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    /// The number the plugin gave this picker.
    pub fn picker(&self) -> u64 {
        self.picker
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Whether the selected result is previewed beside the list.
    pub fn wants_preview(&self) -> bool {
        self.preview
    }

    /// What was typed since the last call, as `(generation, text)`, to pass on to the plugin.
    pub fn take_query(&mut self) -> Option<(u64, String)> {
        self.unsent.take()
    }

    /// Takes in the plugin's newest answer, if it is for the current query. The selection stays on
    /// its entry when results reorder under it, and moves to the nearest row only if that entry is
    /// no longer listed. An answer to an earlier query, or to another picker, is dropped.
    pub fn apply(&mut self, update: PickerUpdate) {
        if update.picker != self.picker || update.generation != self.generation {
            return;
        }
        let on = self.anchor.take();
        self.update = Update {
            generation: update.generation,
            results: update.items,
            matches: update.matches,
            indexed: update.indexed,
            indexing: update.indexing,
            settled: update.settled,
        };
        if let Some(path) = on
            && let Some(row) = self.update.results.iter().position(|c| c.path == path)
        {
            self.selected = row;
        }
        self.selected = self
            .selected
            .min(self.update.results.len().saturating_sub(1));
        self.anchor = self.selected_candidate().map(|c| c.path.clone());
    }

    /// Takes in a folder's contents read in the background, and starts reading the selected
    /// folder's if it is not already.
    pub fn poll(&mut self) {
        while let Ok(loaded) = self.children_rx.try_recv() {
            self.children = Some(loaded);
        }
        self.ask_for_children();
    }

    fn ask_for_children(&mut self) {
        let wanted = self
            .selected_candidate()
            .filter(|c| c.is_dir)
            .map(|c| c.path.clone());
        if wanted == self.asked {
            return;
        }
        self.asked.clone_from(&wanted);
        let Some(path) = wanted else { return };
        let (vfs, tx) = (Arc::clone(&self.vfs), self.children_tx.clone());
        self.handle.spawn_blocking(move || {
            let mut entries = vfs.list_dir(&path).unwrap_or_default();
            entries.truncate(PREVIEW_ENTRIES);
            let _ = tx.send((path, entries));
        });
    }

    fn retype(&mut self) {
        self.generation += 1;
        self.unsent = Some((self.generation, self.query.to_string()));
        self.selected = 0;
        self.anchor = None;
        self.top.set(0);
    }

    pub fn query(&self) -> &LineEdit {
        &self.query
    }

    pub fn indexing(&self) -> PickerIndexing {
        self.update.indexing
    }

    /// Whether every indexed entry has been ranked against what is typed. False for a moment
    /// after typing into a large index, while the results below may still change.
    pub fn settled(&self) -> bool {
        self.update.settled && self.update.generation == self.generation
    }

    /// How many entries have been indexed so far.
    pub fn indexed(&self) -> usize {
        self.update.indexed
    }

    /// How many entries match what is typed (more than are listed when it is a lot).
    pub fn matches(&self) -> usize {
        self.update.matches
    }

    /// The listed results, best first.
    pub fn results(&self) -> impl ExactSizeIterator<Item = &TreeEntry> {
        self.update.results.iter()
    }

    /// How many results are listed.
    pub fn result_count(&self) -> usize {
        self.update.results.len()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn top(&self) -> &Cell<usize> {
        &self.top
    }

    pub fn viewport(&self) -> &Cell<usize> {
        &self.viewport
    }

    pub fn move_by(&mut self, by: isize) {
        if self.update.results.is_empty() {
            return;
        }
        let last = self.update.results.len() - 1;
        self.selected = self.selected.saturating_add_signed(by).min(last);
        self.anchor = self.selected_candidate().map(|c| c.path.clone());
    }

    pub fn move_to(&mut self, row: usize) {
        if row < self.update.results.len() {
            self.selected = row;
            self.anchor = self.selected_candidate().map(|c| c.path.clone());
        }
    }

    fn selected_candidate(&self) -> Option<&TreeEntry> {
        self.update.results.get(self.selected)
    }

    /// The selected result described for the browser's preview pipelines, so the picker shows a
    /// file the way the browser does.
    pub fn preview_subject(&self) -> Option<DirEntryInfo> {
        let candidate = self.selected_candidate()?;
        Some(DirEntryInfo {
            name: candidate.name().to_string(),
            path: candidate.path.clone(),
            is_dir: candidate.is_dir,
            size: 0,
            modified: None,
            mode: None,
        })
    }

    /// The selected folder's contents once they have been read (the first `PREVIEW_ENTRIES`),
    /// or `None` for a file, or while they are still being read.
    pub fn children(&self) -> Option<&[DirEntryInfo]> {
        let candidate = self.selected_candidate().filter(|c| c.is_dir)?;
        self.children
            .as_ref()
            .filter(|(path, _)| *path == candidate.path)
            .map(|(_, entries)| entries.as_slice())
    }

    /// `Esc` closes; `Enter` shows the selected result; `Down`/`Ctrl`-`N` and `Up`/`Ctrl`-`P`
    /// (and the page keys) move through the results, stopping at the ends; any other key edits
    /// the query, which the plugin re-ranks.
    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = isize::try_from(self.viewport.get().max(1)).unwrap_or(isize::MAX);
        match key.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Enter => {
                return match self.selected_candidate() {
                    Some(candidate) => Outcome::Reveal(candidate.path.clone()),
                    None => Outcome::Stay,
                };
            }
            KeyCode::Down => self.move_by(1),
            KeyCode::Up => self.move_by(-1),
            KeyCode::Char('n') if ctrl => self.move_by(1),
            KeyCode::Char('p') if ctrl => self.move_by(-1),
            KeyCode::PageDown => self.move_by(page),
            KeyCode::PageUp => self.move_by(-page),
            _ => {
                if self.query.edit(key) {
                    self.retype();
                }
            }
        }
        Outcome::Stay
    }
}

#[cfg(test)]
mod tests {
    use shared::MemVfs;
    use std::time::{Duration, Instant};

    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn entry(rel: &str, is_dir: bool) -> TreeEntry {
        TreeEntry {
            path: PathBuf::from("/root").join(rel),
            rel: rel.to_string(),
            is_dir,
            size: None,
            modified: None,
        }
    }

    /// What the plugin knows: seven entries, as the old finder tests had.
    fn universe() -> Vec<TreeEntry> {
        vec![
            entry("README.md", false),
            entry("docs", true),
            entry("docs/guide", true),
            entry("docs/guide/main.md", false),
            entry("src", true),
            entry("src/lib.rs", false),
            entry("src/main.rs", false),
        ]
    }

    fn spec() -> PickerSpec {
        PickerSpec {
            picker: 11,
            title: "find".to_string(),
            preview: true,
        }
    }

    fn update(generation: u64, items: Vec<TreeEntry>, settled: bool) -> PickerUpdate {
        PickerUpdate {
            picker: 11,
            generation,
            matches: items.len(),
            indexed: 7,
            items,
            indexing: PickerIndexing::Done,
            settled,
        }
    }

    /// A stand-in for the plugin: answers the newest query by keeping the entries whose path
    /// holds what was typed, or all of them for nothing typed.
    fn answer(popup: &mut PickerPopup) {
        let (generation, text) = popup
            .take_query()
            .unwrap_or((popup.generation, popup.query.to_string()));
        let items = universe()
            .into_iter()
            .filter(|e| e.rel.to_lowercase().contains(&text.to_lowercase()))
            .collect();
        popup.apply(update(generation, items, true));
    }

    fn type_text(popup: &mut PickerPopup, text: &str) {
        for c in text.chars() {
            popup.key(press(KeyCode::Char(c)));
        }
    }

    fn open(runtime: &tokio::runtime::Runtime) -> PickerPopup {
        let vfs = MemVfs::new();
        vfs.add_file("/root/README.md", b"");
        vfs.add_file("/root/src/main.rs", b"");
        vfs.add_file("/root/src/lib.rs", b"");
        vfs.add_file("/root/docs/guide/main.md", b"");
        let mut popup = PickerPopup::open(runtime.handle(), Arc::new(vfs), "finder", &spec());
        answer(&mut popup);
        popup
    }

    fn rels(popup: &PickerPopup) -> Vec<String> {
        popup.results().map(|c| c.rel.clone()).collect()
    }

    #[test]
    fn it_remembers_whose_picker_it_is() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let popup = open(&runtime);
        assert_eq!((popup.plugin(), popup.picker()), ("finder", 11));
        assert_eq!(popup.title(), "find");
        assert!(popup.wants_preview());
    }

    #[test]
    fn with_nothing_typed_everything_indexed_is_listed() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let popup = open(&runtime);
        assert_eq!(popup.indexed(), 7);
        assert_eq!(popup.result_count(), 7);
        assert_eq!(popup.matches(), 7);
        assert!(popup.settled());
    }

    #[test]
    fn each_edit_of_the_query_is_recorded_once_for_the_plugin() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        assert_eq!(popup.take_query(), None);
        type_text(&mut popup, "ma");
        assert_eq!(popup.take_query(), Some((2, "ma".to_string())));
        assert_eq!(popup.take_query(), None, "taking it consumes it");
        popup.key(press(KeyCode::Backspace));
        popup.key(press(KeyCode::Backspace));
        assert_eq!(popup.take_query(), Some((4, String::new())));
        popup.key(press(KeyCode::Left));
        assert_eq!(
            popup.take_query(),
            None,
            "moving the cursor changes nothing"
        );
    }

    #[test]
    fn typing_narrows_the_results_when_the_plugin_answers() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        type_text(&mut popup, "main");
        answer(&mut popup);
        assert_eq!(rels(&popup), ["docs/guide/main.md", "src/main.rs"]);
        assert_eq!(popup.matches(), 2);
    }

    #[test]
    fn the_old_results_stay_until_the_new_ones_arrive() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        type_text(&mut popup, "m");
        // No answer yet: nothing has been said, and the list has not gone blank.
        assert_eq!(popup.result_count(), 7);
        assert!(!popup.settled());
    }

    #[test]
    fn an_answer_to_an_earlier_query_or_another_picker_is_dropped() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        type_text(&mut popup, "ma");
        popup.apply(update(1, vec![entry("stale", false)], true));
        assert_eq!(popup.result_count(), 7, "generation 1 is not the query");
        let mut other = update(2, vec![entry("other", false)], true);
        other.picker = 99;
        popup.apply(other);
        assert_eq!(popup.result_count(), 7, "another picker's answer");
        popup.apply(update(2, vec![entry("fresh", false)], true));
        assert_eq!(rels(&popup), ["fresh"]);
    }

    #[test]
    fn j_and_k_are_letters_and_the_arrows_move() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        type_text(&mut popup, "jk");
        assert_eq!(popup.query().as_str(), "jk");
        popup.key(press(KeyCode::Backspace));
        popup.key(press(KeyCode::Backspace));
        answer(&mut popup);
        assert_eq!(popup.selected(), 0);
        popup.key(press(KeyCode::Down));
        popup.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(popup.selected(), 2);
        popup.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        popup.key(press(KeyCode::Up));
        assert_eq!(popup.selected(), 0);
    }

    #[test]
    fn the_selection_stops_at_both_ends_and_a_new_query_returns_it_to_the_top() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        popup.key(press(KeyCode::Up));
        assert_eq!(popup.selected(), 0);
        for _ in 0..20 {
            popup.key(press(KeyCode::Down));
        }
        assert_eq!(popup.selected(), 6);
        type_text(&mut popup, "s");
        assert_eq!(popup.selected(), 0);
    }

    #[test]
    fn the_selection_stays_on_its_entry_when_results_change_under_it() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        popup.key(press(KeyCode::Down));
        popup.key(press(KeyCode::Down));
        let on = popup.selected_candidate().unwrap().path.clone();
        // The same results arrive again in another order, with a new entry ahead of them.
        let mut items = universe();
        items.reverse();
        items.insert(0, entry("new", false));
        popup.apply(update(0, items, true));
        assert_eq!(popup.selected_candidate().unwrap().path, on);
    }

    #[test]
    fn enter_asks_to_reveal_the_selected_result_and_esc_closes() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        type_text(&mut popup, "lib");
        answer(&mut popup);
        assert_eq!(
            popup.key(press(KeyCode::Enter)),
            Outcome::Reveal(PathBuf::from("/root/src/lib.rs"))
        );
        assert_eq!(popup.key(press(KeyCode::Esc)), Outcome::Close);
    }

    #[test]
    fn enter_with_no_match_does_nothing() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        type_text(&mut popup, "zzzzz");
        answer(&mut popup);
        assert_eq!(popup.result_count(), 0);
        assert_eq!(popup.key(press(KeyCode::Enter)), Outcome::Stay);
        assert!(popup.preview_subject().is_none());
    }

    #[test]
    fn the_preview_subject_follows_the_selection() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        type_text(&mut popup, "guide");
        answer(&mut popup);
        let subject = popup.preview_subject().expect("guide matches");
        assert_eq!(subject.name, "guide");
        assert!(subject.is_dir);
        assert_eq!(subject.path, PathBuf::from("/root/docs/guide"));
    }

    #[test]
    fn a_selected_folders_contents_arrive_in_the_background_and_a_file_has_none() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        type_text(&mut popup, "guide");
        answer(&mut popup);
        assert!(popup.children().is_none(), "not read yet");
        let deadline = Instant::now() + Duration::from_secs(5);
        while popup.children().is_none() {
            popup.poll();
            assert!(Instant::now() < deadline, "the folder was never read");
            std::thread::sleep(Duration::from_millis(5));
        }
        let names: Vec<&str> = popup
            .children()
            .unwrap()
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(names, ["main.md"]);
        for _ in 0.."guide".len() {
            popup.key(press(KeyCode::Backspace));
        }
        type_text(&mut popup, "lib");
        answer(&mut popup);
        popup.poll();
        assert!(popup.children().is_none(), "a file has no folder contents");
    }
}
