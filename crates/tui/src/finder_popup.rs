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

//! The fuzzy finder's state (`f`): what is typed, the index of every entry below where it was
//! opened, the ranked results and which one is selected. Like `settings_popup` it is pure: no
//! `Frame`, no terminal. `finder_view` draws it and `main` applies what a key asks for, which is
//! only ever to close it or to show a result in the browser.
//!
//! The index fills in while the finder is open (see `finder_job`), and the results re-rank as it
//! does, so typing never waits for the walk. Every key goes to the query, so `j` and `k` are
//! letters here; the arrows and `Ctrl`-`N`/`Ctrl`-`P` move through the results.

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Arc;

use browser::finder::{Candidate, Ranking, SHOWN};
use browser::search::Query;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use shared::{DirEntryInfo, Vfs};

use crate::finder_job::{FinderJob, Progress};
use crate::line_edit::LineEdit;

/// What a keystroke asks the caller to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Close,
    /// Close and put the browser's cursor on this entry.
    Reveal(PathBuf),
}

/// Where the walk that feeds the index stands, for the finder's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indexing {
    Walking,
    Done { truncated: bool },
}

pub struct FinderPopup {
    query: LineEdit,
    index: Vec<Candidate>,
    ranking: Ranking,
    /// Positions in `index` of the ranked results, best first.
    results: Vec<usize>,
    selected: usize,
    job: FinderJob,
    indexing: Indexing,
    /// The first row drawn and how many fit, kept by the renderer between frames the way the
    /// trash browser's are.
    top: Cell<usize>,
    viewport: Cell<usize>,
}

impl FinderPopup {
    /// Opens the finder over everything below `root` and starts indexing it.
    pub fn open(
        handle: &tokio::runtime::Handle,
        vfs: Arc<dyn Vfs>,
        root: PathBuf,
        show_hidden: bool,
    ) -> Self {
        let job = FinderJob::start(handle, vfs, root, show_hidden);
        let mut ranking = Ranking::new();
        ranking.set_query(None);
        Self {
            query: LineEdit::default(),
            index: Vec::new(),
            ranking,
            results: Vec::new(),
            selected: 0,
            job,
            indexing: Indexing::Walking,
            top: Cell::new(0),
            viewport: Cell::new(0),
        }
    }

    /// Takes in whatever the walk has found since the last turn of the loop and re-ranks if there
    /// was something. The selection stays on its row, so results arriving under the cursor do not
    /// move it.
    pub fn poll(&mut self) {
        let before = self.index.len();
        self.indexing = match self.job.poll(&mut self.index) {
            Progress::Walking => Indexing::Walking,
            Progress::Finished { truncated } => Indexing::Done { truncated },
        };
        if self.index.len() != before {
            self.ranking.extend(&self.index);
            self.rank();
        }
    }

    fn rank(&mut self) {
        self.results = self.ranking.top(SHOWN);
        self.selected = self.selected.min(self.results.len().saturating_sub(1));
    }

    fn retype(&mut self) {
        self.ranking.set_query(Query::new(&self.query));
        self.ranking.extend(&self.index);
        self.selected = 0;
        self.top.set(0);
        self.rank();
    }

    pub fn query(&self) -> &LineEdit {
        &self.query
    }

    pub fn indexing(&self) -> Indexing {
        self.indexing
    }

    /// How many entries have been indexed so far.
    pub fn indexed(&self) -> usize {
        self.index.len()
    }

    /// How many entries match what is typed (more than are listed when it is a lot).
    pub fn matches(&self) -> usize {
        self.ranking.matches()
    }

    /// The listed results, best first.
    pub fn results(&self) -> impl ExactSizeIterator<Item = &Candidate> {
        self.results.iter().map(|&position| &self.index[position])
    }

    /// How many results are listed.
    pub fn result_count(&self) -> usize {
        self.results.len()
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
        if self.results.is_empty() {
            return;
        }
        let last = self.results.len() - 1;
        self.selected = self.selected.saturating_add_signed(by).min(last);
    }

    pub fn move_to(&mut self, row: usize) {
        if row < self.results.len() {
            self.selected = row;
        }
    }

    fn selected_candidate(&self) -> Option<&Candidate> {
        self.results
            .get(self.selected)
            .map(|&position| &self.index[position])
    }

    /// The selected result described for the browser's preview pipelines, so the finder shows a
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

    /// `Esc` closes; `Enter` shows the selected result; `Down`/`Ctrl`-`N` and `Up`/`Ctrl`-`P`
    /// (and the page keys) move through the results, stopping at the ends; any other key edits
    /// the query, which re-ranks.
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

    fn type_text(popup: &mut FinderPopup, text: &str) {
        for c in text.chars() {
            popup.key(press(KeyCode::Char(c)));
        }
    }

    /// A finder over a small tree, once its walk has finished.
    fn open(runtime: &tokio::runtime::Runtime) -> FinderPopup {
        let vfs = MemVfs::new();
        vfs.add_file("/root/README.md", b"");
        vfs.add_file("/root/src/main.rs", b"");
        vfs.add_file("/root/src/lib.rs", b"");
        vfs.add_file("/root/docs/guide/main.md", b"");
        let mut popup = FinderPopup::open(runtime.handle(), Arc::new(vfs), "/root".into(), false);
        let deadline = Instant::now() + Duration::from_secs(5);
        while popup.indexing() == Indexing::Walking {
            popup.poll();
            assert!(Instant::now() < deadline, "the walk never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
        popup
    }

    fn rels(popup: &FinderPopup) -> Vec<String> {
        popup.results().map(|c| c.rel.clone()).collect()
    }

    #[test]
    fn with_nothing_typed_everything_indexed_is_listed() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let popup = open(&runtime);
        assert_eq!(popup.indexed(), 7);
        assert_eq!(popup.results().len(), 7);
        assert_eq!(popup.matches(), 7);
    }

    #[test]
    fn typing_narrows_and_ranks_the_results() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        type_text(&mut popup, "main");
        let found = rels(&popup);
        assert_eq!(found.len(), 2);
        assert!(found.contains(&"src/main.rs".to_string()));
        assert!(found.contains(&"docs/guide/main.md".to_string()));
        assert_eq!(popup.matches(), 2);
    }

    #[test]
    fn j_and_k_are_letters_and_the_arrows_move() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        type_text(&mut popup, "jk");
        assert_eq!(popup.query().as_str(), "jk");
        popup.key(press(KeyCode::Backspace));
        popup.key(press(KeyCode::Backspace));
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
    fn enter_asks_to_reveal_the_selected_result_and_esc_closes() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        type_text(&mut popup, "lib");
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
        assert_eq!(popup.results().len(), 0);
        assert_eq!(popup.key(press(KeyCode::Enter)), Outcome::Stay);
        assert!(popup.preview_subject().is_none());
    }

    #[test]
    fn the_preview_subject_follows_the_selection() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut popup = open(&runtime);
        type_text(&mut popup, "guide");
        let subject = popup.preview_subject().expect("guide matches");
        assert_eq!(subject.name, "guide");
        assert!(subject.is_dir);
        assert_eq!(subject.path, PathBuf::from("/root/docs/guide"));
    }
}
