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

//! The extract form's state: which row has focus, what has been typed, and what a key means.
//! Like `compress_popup` this is pure, with no `Frame` and no filesystem, so `overlay_view` is
//! the only place that draws it and `main` the only place that acts on what it submits.
//!
//! The form never starts the job. [`Key::Submit`] hands back a [`Request`] whose destination is
//! either this directory, one plain folder name, or a folder per archive, and
//! `App::start_extract` decides whether it can run (a folder already in the way, an operation
//! already running) and reports that back through [`ExtractPopup::set_error`].

use std::path::{Path, PathBuf};

use crossterm::event::KeyCode;
use file_ops::ConflictPolicy;
use file_ops::archive::Format;

use crate::compress_popup::check_name;

/// One row of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Folder,
    Existing,
    DeleteArchives,
}

impl Row {
    pub fn label(self) -> &'static str {
        match self {
            Row::Folder => "Folder",
            Row::Existing => "Existing files",
            Row::DeleteArchives => "Delete archives",
        }
    }
}

/// Where the contents land, relative to the browsed directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// Straight into the browsed directory.
    Here,
    /// Into one new folder with this name (a single plain path component).
    Folder(String),
    /// Into a folder per archive, named after it.
    PerArchive,
}

/// A job the form asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub archives: Vec<PathBuf>,
    /// The directory `destination` is relative to.
    pub dir: PathBuf,
    pub destination: Destination,
    pub policy: ConflictPolicy,
    /// Send each archive to the trash once it has been extracted.
    pub delete_archives: bool,
}

/// What a keystroke asks the caller to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Stay,
    Close,
    Submit(Request),
}

pub struct ExtractPopup {
    archives: Vec<PathBuf>,
    dir: PathBuf,
    cursor: usize,
    /// The typed folder name; empty means extract into the browsed directory itself.
    folder: String,
    policy: ConflictPolicy,
    delete_archives: bool,
    error: Option<String>,
    /// A heads-up that does not stop the job from being asked for, such as a folder already there.
    warning: Option<String>,
}

const ROWS: [Row; 3] = [Row::Folder, Row::Existing, Row::DeleteArchives];

impl ExtractPopup {
    /// A form for extracting `archives` from `dir`. `None` when there is nothing to extract.
    pub fn new(archives: Vec<PathBuf>, dir: PathBuf) -> Option<Self> {
        if archives.is_empty() {
            return None;
        }
        Some(Self {
            folder: default_folder(&archives),
            archives,
            dir,
            cursor: 0,
            policy: ConflictPolicy::Abort,
            delete_archives: false,
            error: None,
            warning: None,
        })
    }

    pub fn rows(&self) -> &'static [Row] {
        &ROWS
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn folder(&self) -> &str {
        &self.folder
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Shows `message` under the form until the next keystroke.
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }

    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    /// Sets (or clears) the heads-up shown under the form while there is no error. The caller
    /// works it out from the disk, which this form never touches, after each keystroke.
    pub fn set_warning(&mut self, warning: Option<String>) {
        self.warning = warning;
    }

    /// The panel's title.
    pub fn title(&self) -> String {
        match self.archives.as_slice() {
            [only] => format!("extract: {}", display_name(only)),
            many => format!("extract: {} archives", many.len()),
        }
    }

    /// Whether the Folder row is used: with several archives each is extracted into a folder
    /// named after it, so there is no single name to type.
    pub fn folder_applies(&self) -> bool {
        self.archives.len() == 1
    }

    /// The row's value as the form displays it. The Folder row is drawn by the caller (it has a
    /// cursor), so its value here is only the typed text.
    pub fn value(&self, row: Row) -> String {
        match row {
            Row::Folder if !self.folder_applies() => {
                "(each goes to a folder named after it)".into()
            }
            Row::Folder => self.folder.clone(),
            Row::Existing => match self.policy {
                ConflictPolicy::Abort => "stop",
                ConflictPolicy::Skip => "skip them",
                ConflictPolicy::Overwrite => "replace them",
            }
            .into(),
            Row::DeleteArchives => {
                if self.delete_archives {
                    "yes — archives go to the trash".into()
                } else {
                    "no".into()
                }
            }
        }
    }

    /// Moves focus to `index` (a click on that row).
    pub fn focus(&mut self, index: usize) {
        if index < ROWS.len() {
            self.cursor = index;
            self.error = None;
        }
    }

    /// Changes the focused row's value one step forward, unless it is the Folder row, which is
    /// typed into rather than cycled.
    pub fn cycle(&mut self, forward: bool) {
        self.error = None;
        match ROWS[self.cursor] {
            Row::Folder => {}
            Row::Existing => {
                self.policy = match (self.policy, forward) {
                    (ConflictPolicy::Abort, true) | (ConflictPolicy::Overwrite, false) => {
                        ConflictPolicy::Skip
                    }
                    (ConflictPolicy::Skip, true) | (ConflictPolicy::Abort, false) => {
                        ConflictPolicy::Overwrite
                    }
                    (ConflictPolicy::Overwrite, true) | (ConflictPolicy::Skip, false) => {
                        ConflictPolicy::Abort
                    }
                };
            }
            Row::DeleteArchives => self.delete_archives = !self.delete_archives,
        }
    }

    /// Up, down, `Tab` and `BackTab` move focus, wrapping. On the Folder row characters are typed
    /// and `Backspace` deletes; on any other row `j`/`k` also move, `h`/`l`/`Space`/the side
    /// arrows change the value, and `Enter` starts the job from any row. `Esc` closes.
    pub fn key(&mut self, code: KeyCode) -> Key {
        let on_folder = ROWS[self.cursor] == Row::Folder;
        match code {
            KeyCode::Esc => return Key::Close,
            KeyCode::Enter => return self.submit(),
            KeyCode::Down | KeyCode::Tab => self.step(1),
            KeyCode::Up | KeyCode::BackTab => self.step(ROWS.len() - 1),
            KeyCode::Char(c) if on_folder => {
                if self.folder_applies() && !c.is_control() && c != '/' {
                    self.folder.push(c);
                }
                self.error = None;
            }
            KeyCode::Backspace if on_folder => {
                self.folder.pop();
                self.error = None;
            }
            KeyCode::Char('j') => self.step(1),
            KeyCode::Char('k') => self.step(ROWS.len() - 1),
            KeyCode::Left | KeyCode::Char('h') => self.cycle(false),
            KeyCode::Right | KeyCode::Char('l' | ' ') => self.cycle(true),
            _ => {}
        }
        Key::Stay
    }

    fn step(&mut self, by: usize) {
        self.cursor = (self.cursor + by) % ROWS.len();
        self.error = None;
    }

    fn submit(&mut self) -> Key {
        match self.build() {
            Ok(request) => Key::Submit(request),
            Err(message) => {
                self.error = Some(message);
                Key::Stay
            }
        }
    }

    /// The job the form currently describes, or why it cannot be asked for yet. Changes nothing,
    /// so the caller can also use it to look ahead (for a folder that is already there).
    pub fn build(&self) -> Result<Request, String> {
        let destination = if self.folder_applies() {
            // An empty name is a deliberate "extract here", not a mistake, so it skips the
            // check that would reject it as a folder name.
            let typed = self.folder.trim();
            if typed.is_empty() {
                Destination::Here
            } else {
                check_name(typed).map_err(str::to_owned)?;
                Destination::Folder(typed.to_owned())
            }
        } else {
            Destination::PerArchive
        };
        Ok(Request {
            archives: self.archives.clone(),
            dir: self.dir.clone(),
            destination,
            policy: self.policy,
            delete_archives: self.delete_archives,
        })
    }
}

/// The folder offered first: the archive's name without its extension (`photos.tar.gz` becomes
/// `photos`), so a tarball full of loose files cannot litter the browsed directory. Empty for
/// several archives, where the Folder row is unused.
fn default_folder(archives: &[PathBuf]) -> String {
    let [only] = archives else {
        return String::new();
    };
    Format::stem_of(only)
        .filter(|name| check_name(name).is_ok())
        .unwrap_or_else(|| "extracted".into())
}

fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn popup(names: &[&str]) -> ExtractPopup {
        let archives = names.iter().map(|n| PathBuf::from("/w").join(n)).collect();
        ExtractPopup::new(archives, PathBuf::from("/w")).unwrap()
    }

    fn clear_folder(popup: &mut ExtractPopup) {
        while !popup.folder().is_empty() {
            popup.key(KeyCode::Backspace);
        }
    }

    fn submitted(key: Key) -> Request {
        match key {
            Key::Submit(request) => request,
            other => panic!("expected a submit, got {other:?}"),
        }
    }

    #[test]
    fn nothing_selected_means_no_form() {
        assert!(ExtractPopup::new(vec![], PathBuf::from("/w")).is_none());
    }

    #[test]
    fn the_folder_starts_from_the_archive_name_without_its_extension() {
        assert_eq!(popup(&["photos.tar.gz"]).folder(), "photos");
        assert_eq!(popup(&["notes.zip"]).folder(), "notes");
        assert_eq!(popup(&["a.zip", "b.zip"]).folder(), "");
    }

    #[test]
    fn enter_submits_a_new_folder_named_after_the_archive() {
        let mut p = popup(&["notes.zip"]);
        let request = submitted(p.key(KeyCode::Enter));
        assert_eq!(request.destination, Destination::Folder("notes".into()));
        assert_eq!(request.policy, ConflictPolicy::Abort);
        assert!(!request.delete_archives);
        assert_eq!(request.dir, PathBuf::from("/w"));
    }

    #[test]
    fn an_emptied_folder_means_extract_here() {
        let mut p = popup(&["notes.zip"]);
        clear_folder(&mut p);
        assert_eq!(
            submitted(p.key(KeyCode::Enter)).destination,
            Destination::Here
        );
    }

    #[test]
    fn several_archives_each_go_to_their_own_folder_and_the_name_is_not_typed() {
        let mut p = popup(&["a.zip", "b.tar"]);
        assert!(!p.folder_applies());
        p.key(KeyCode::Char('x'));
        assert_eq!(p.folder(), "");
        assert_eq!(
            submitted(p.key(KeyCode::Enter)).destination,
            Destination::PerArchive
        );
    }

    #[test]
    fn a_reserved_folder_name_is_an_error_not_a_job() {
        for bad in [".", ".."] {
            let mut p = popup(&["x.zip"]);
            clear_folder(&mut p);
            for c in bad.chars() {
                p.key(KeyCode::Char(c));
            }
            assert_eq!(p.key(KeyCode::Enter), Key::Stay, "{bad:?}");
            assert!(p.error().is_some(), "{bad:?}");
        }
    }

    #[test]
    fn a_slash_or_control_character_is_not_typed() {
        let mut p = popup(&["x.zip"]);
        clear_folder(&mut p);
        for c in "a/b\u{1b}c".chars() {
            p.key(KeyCode::Char(c));
        }
        assert_eq!(p.folder(), "abc");
    }

    #[test]
    fn existing_files_cycle_through_stop_skip_and_replace() {
        let mut p = popup(&["x.zip"]);
        p.focus(1);
        assert_eq!(p.value(Row::Existing), "stop");
        p.key(KeyCode::Right);
        assert_eq!(
            submitted(p.key(KeyCode::Enter)).policy,
            ConflictPolicy::Skip
        );
        p.key(KeyCode::Right);
        assert_eq!(
            submitted(p.key(KeyCode::Enter)).policy,
            ConflictPolicy::Overwrite
        );
        p.key(KeyCode::Right);
        assert_eq!(
            submitted(p.key(KeyCode::Enter)).policy,
            ConflictPolicy::Abort
        );
        p.key(KeyCode::Left);
        assert_eq!(
            submitted(p.key(KeyCode::Enter)).policy,
            ConflictPolicy::Overwrite
        );
    }

    #[test]
    fn delete_archives_toggles() {
        let mut p = popup(&["x.zip"]);
        p.focus(2);
        p.key(KeyCode::Char(' '));
        assert!(submitted(p.key(KeyCode::Enter)).delete_archives);
        p.key(KeyCode::Char(' '));
        assert!(!submitted(p.key(KeyCode::Enter)).delete_archives);
    }

    #[test]
    fn off_the_folder_row_j_and_k_move_and_focus_wraps() {
        let mut p = popup(&["x.zip"]);
        p.key(KeyCode::Char('x'));
        assert_eq!(p.cursor(), 0);
        p.key(KeyCode::Down);
        p.key(KeyCode::Char('j'));
        assert_eq!(p.rows()[p.cursor()], Row::DeleteArchives);
        p.key(KeyCode::Tab);
        assert_eq!(p.rows()[p.cursor()], Row::Folder);
        p.key(KeyCode::BackTab);
        assert_eq!(p.rows()[p.cursor()], Row::DeleteArchives);
        p.key(KeyCode::Char('k'));
        assert_eq!(p.rows()[p.cursor()], Row::Existing);
    }

    #[test]
    fn the_error_clears_on_the_next_key_and_escape_closes() {
        let mut p = popup(&["x.zip"]);
        p.set_error("boom");
        p.key(KeyCode::Down);
        assert!(p.error().is_none());
        assert_eq!(p.key(KeyCode::Esc), Key::Close);
    }

    proptest! {
        /// Whatever is typed, a submitted destination can only land inside the chosen directory.
        #[test]
        fn a_submitted_folder_is_always_a_single_safe_component(
            typed in prop::collection::vec(any::<char>(), 0..40),
        ) {
            let mut p = popup(&["seed.zip"]);
            for c in typed {
                p.key(KeyCode::Char(c));
            }
            if let Key::Submit(request) = p.key(KeyCode::Enter)
                && let Destination::Folder(name) = request.destination
            {
                let path = Path::new(&name);
                prop_assert_eq!(path.components().count(), 1);
                prop_assert!(matches!(
                    path.components().next(),
                    Some(std::path::Component::Normal(_))
                ));
                prop_assert!(!name.chars().any(char::is_control));
            }
        }
    }
}
