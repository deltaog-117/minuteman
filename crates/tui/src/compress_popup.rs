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

//! The compress form's state: which row has focus, what has been typed, and what a key means.
//! Like `settings_popup` this is pure, with no `Frame` and no filesystem, so `overlay_view` is
//! the only place that draws it and `main` the only place that acts on what it submits.
//!
//! The form never starts the job. [`Key::Submit`] hands back a [`Request`] that can only describe
//! a well-formed job (a file name that is a single, plain path component, or one archive per
//! item), and `App::start_compress` decides whether it can run (an existing file in the way, an
//! operation already running) and reports that back through [`CompressPopup::set_error`].

use std::path::{Path, PathBuf};

use crossterm::event::KeyCode;
use file_ops::archive::{Format, Level};

/// The longest archive file name the form accepts, in characters. Well under the 255 bytes most
/// filesystems allow, so the extension and a `.partial` sibling still fit.
const MAX_NAME_CHARS: usize = 100;

/// One row of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Name,
    Format,
    Level,
    /// Only offered when more than one item is being compressed.
    PerItem,
    DeleteOriginals,
}

impl Row {
    pub fn label(self) -> &'static str {
        match self {
            Row::Name => "Name",
            Row::Format => "Format",
            Row::Level => "Level",
            Row::PerItem => "One per item",
            Row::DeleteOriginals => "Delete originals",
        }
    }
}

/// How the items are packed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layout {
    /// Everything into one archive with this file name (extension included).
    One { file_name: String },
    /// A separate archive for each item, named after it.
    PerItem,
}

/// A job the form asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub sources: Vec<PathBuf>,
    /// Where the archives are written.
    pub dir: PathBuf,
    pub format: Format,
    pub level: Level,
    pub layout: Layout,
    /// Send the originals to the trash once their archive is complete.
    pub delete_originals: bool,
}

/// What a keystroke asks the caller to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Stay,
    Close,
    Submit(Request),
}

pub struct CompressPopup {
    sources: Vec<PathBuf>,
    dir: PathBuf,
    rows: Vec<Row>,
    cursor: usize,
    /// The typed name, without the extension the format adds.
    name: String,
    format: Format,
    level: Level,
    per_item: bool,
    delete_originals: bool,
    error: Option<String>,
}

impl CompressPopup {
    /// A form for compressing `sources` into `dir`. `None` when there is nothing to compress.
    pub fn new(sources: Vec<PathBuf>, dir: PathBuf) -> Option<Self> {
        if sources.is_empty() {
            return None;
        }
        let mut rows = vec![Row::Name, Row::Format, Row::Level];
        if sources.len() > 1 {
            rows.push(Row::PerItem);
        }
        rows.push(Row::DeleteOriginals);
        Some(Self {
            name: default_name(&sources),
            sources,
            dir,
            rows,
            cursor: 0,
            format: Format::Zip,
            level: Level::Normal,
            per_item: false,
            delete_originals: false,
            error: None,
        })
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn format(&self) -> Format {
        self.format
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Shows `message` under the form until the next keystroke.
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }

    /// The panel's title.
    pub fn title(&self) -> String {
        match self.sources.as_slice() {
            [only] => format!("compress: {}", display_name(only)),
            many => format!("compress: {} items", many.len()),
        }
    }

    /// Whether the Name row is used: with one archive per item each is named after its item.
    pub fn name_applies(&self) -> bool {
        !self.per_item
    }

    /// Whether the Level row is used: a plain tar is not compressed.
    pub fn level_applies(&self) -> bool {
        self.format != Format::Tar
    }

    /// The row's value as the form displays it. The Name row is drawn by the caller (it has a
    /// cursor and a fixed extension), so its value here is only the typed text.
    pub fn value(&self, row: Row) -> String {
        match row {
            Row::Name if !self.name_applies() => "(each is named after its item)".into(),
            Row::Name => self.name.clone(),
            Row::Format => match self.format {
                Format::Zip => "zip",
                Format::TarGz => "tar.gz",
                Format::Tar => "tar",
            }
            .into(),
            Row::Level if !self.level_applies() => "(a plain tar is not compressed)".into(),
            Row::Level => match self.level {
                Level::Fast => "fast",
                Level::Normal => "normal",
                Level::Best => "smallest",
            }
            .into(),
            Row::PerItem => yes_no(self.per_item, "one archive for each item"),
            Row::DeleteOriginals => yes_no(self.delete_originals, "originals go to the trash"),
        }
    }

    /// Moves focus to `index` (a click on that row).
    pub fn focus(&mut self, index: usize) {
        if index < self.rows.len() {
            self.cursor = index;
            self.error = None;
        }
    }

    /// Changes the focused row's value one step forward, unless it is the Name row, which is
    /// typed into rather than cycled.
    pub fn cycle(&mut self, forward: bool) {
        self.error = None;
        match self.rows[self.cursor] {
            Row::Name => {}
            Row::Format => {
                self.format = match (self.format, forward) {
                    (Format::Zip, true) | (Format::Tar, false) => Format::TarGz,
                    (Format::TarGz, true) | (Format::Zip, false) => Format::Tar,
                    (Format::Tar, true) | (Format::TarGz, false) => Format::Zip,
                };
            }
            Row::Level if self.level_applies() => {
                self.level = match (self.level, forward) {
                    (Level::Fast, true) | (Level::Best, false) => Level::Normal,
                    (Level::Normal, true) | (Level::Fast, false) => Level::Best,
                    (Level::Best, true) | (Level::Normal, false) => Level::Fast,
                };
            }
            Row::Level => {}
            Row::PerItem => self.per_item = !self.per_item,
            Row::DeleteOriginals => self.delete_originals = !self.delete_originals,
        }
    }

    /// Up, down, `Tab` and `BackTab` move focus, wrapping. On the Name row characters are typed
    /// and `Backspace` deletes; on any other row `j`/`k` also move, `h`/`l`/`Space`/the side
    /// arrows change the value, and `Enter` starts the job from any row. `Esc` closes.
    pub fn key(&mut self, code: KeyCode) -> Key {
        let on_name = self.rows[self.cursor] == Row::Name;
        match code {
            KeyCode::Esc => return Key::Close,
            KeyCode::Enter => return self.submit(),
            KeyCode::Down | KeyCode::Tab => self.step(1),
            KeyCode::Up | KeyCode::BackTab => self.step(self.rows.len() - 1),
            KeyCode::Char(c) if on_name => {
                if self.name_applies() && !c.is_control() && c != '/' {
                    self.name.push(c);
                }
                self.error = None;
            }
            KeyCode::Backspace if on_name => {
                self.name.pop();
                self.error = None;
            }
            KeyCode::Char('j') => self.step(1),
            KeyCode::Char('k') => self.step(self.rows.len() - 1),
            KeyCode::Left | KeyCode::Char('h') => self.cycle(false),
            KeyCode::Right | KeyCode::Char('l' | ' ') => self.cycle(true),
            _ => {}
        }
        Key::Stay
    }

    fn step(&mut self, by: usize) {
        self.cursor = (self.cursor + by) % self.rows.len();
        self.error = None;
    }

    fn submit(&mut self) -> Key {
        let mut format = self.format;
        let layout = if self.per_item {
            Layout::PerItem
        } else {
            // Typing the extension is a natural thing to do, so `photos.tar.gz` means the
            // format as well as the name rather than becoming `photos.tar.gz.zip`.
            let typed = self.name.trim();
            let base = match Format::of(Path::new(typed)) {
                Some(typed_format) => {
                    format = typed_format;
                    Format::stem_of(Path::new(typed)).unwrap_or_default()
                }
                None => typed.to_owned(),
            };
            if let Err(message) = check_name(&base) {
                self.error = Some(message.into());
                return Key::Stay;
            }
            Layout::One {
                file_name: format!("{base}.{}", format.extension()),
            }
        };
        Key::Submit(Request {
            sources: self.sources.clone(),
            dir: self.dir.clone(),
            format,
            level: self.level,
            layout,
            delete_originals: self.delete_originals,
        })
    }
}

/// Why `base` cannot be an archive's name, or `Ok` if it can: a single plain component that
/// cannot climb out of the directory, and short enough to store.
pub(crate) fn check_name(base: &str) -> Result<(), &'static str> {
    if base.is_empty() {
        Err("the name is empty")
    } else if base == "." || base == ".." {
        Err("that name is reserved")
    } else if base.chars().any(|c| c == '/' || c.is_control()) {
        Err("the name cannot contain / or control characters")
    } else if base.chars().count() > MAX_NAME_CHARS {
        Err("the name is too long")
    } else {
        Ok(())
    }
}

/// The name offered first: a folder's own name, a file's without its extension (`report.pdf`
/// becomes `report`), or `archive` for several items.
fn default_name(sources: &[PathBuf]) -> String {
    let [only] = sources else {
        return "archive".into();
    };
    let name = if only.is_dir() {
        only.file_name()
    } else {
        only.file_stem()
    };
    name.map(|n| n.to_string_lossy().into_owned())
        .filter(|n| check_name(n).is_ok())
        .unwrap_or_else(|| "archive".into())
}

fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into(),
    )
}

fn yes_no(on: bool, detail: &str) -> String {
    if on {
        format!("yes — {detail}")
    } else {
        "no".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn popup(names: &[&str]) -> CompressPopup {
        let sources = names.iter().map(|n| PathBuf::from("/w").join(n)).collect();
        CompressPopup::new(sources, PathBuf::from("/w")).unwrap()
    }

    fn type_text(popup: &mut CompressPopup, text: &str) {
        for c in text.chars() {
            assert_eq!(popup.key(KeyCode::Char(c)), Key::Stay);
        }
    }

    fn clear_name(popup: &mut CompressPopup) {
        while !popup.name().is_empty() {
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
        assert!(CompressPopup::new(vec![], PathBuf::from("/w")).is_none());
    }

    #[test]
    fn the_name_starts_from_the_selection() {
        assert_eq!(popup(&["report.pdf"]).name(), "report");
        assert_eq!(popup(&["a.txt", "b.txt"]).name(), "archive");
        let dir = std::env::temp_dir();
        let folder = CompressPopup::new(vec![dir.clone()], dir.clone()).unwrap();
        assert_eq!(
            folder.name(),
            dir.file_name().unwrap().to_string_lossy().as_ref()
        );
    }

    #[test]
    fn the_per_item_row_only_exists_for_several_items() {
        assert!(!popup(&["a"]).rows().contains(&Row::PerItem));
        assert!(popup(&["a", "b"]).rows().contains(&Row::PerItem));
    }

    #[test]
    fn enter_submits_one_archive_named_with_the_format_extension() {
        let mut p = popup(&["report.pdf"]);
        let request = submitted(p.key(KeyCode::Enter));
        assert_eq!(
            request.layout,
            Layout::One {
                file_name: "report.zip".into()
            }
        );
        assert_eq!(request.format, Format::Zip);
        assert_eq!(request.level, Level::Normal);
        assert!(!request.delete_originals);
        assert_eq!(request.dir, PathBuf::from("/w"));
    }

    #[test]
    fn on_the_name_row_letters_are_typed_and_j_and_k_are_letters() {
        let mut p = popup(&["x"]);
        clear_name(&mut p);
        type_text(&mut p, "jk h l");
        assert_eq!(p.name(), "jk h l");
        assert_eq!(p.cursor(), 0);
        p.key(KeyCode::Backspace);
        assert_eq!(p.name(), "jk h ");
    }

    #[test]
    fn a_slash_or_control_character_is_not_typed() {
        let mut p = popup(&["x"]);
        clear_name(&mut p);
        type_text(&mut p, "a/b\u{1b}c");
        assert_eq!(p.name(), "abc");
    }

    #[test]
    fn off_the_name_row_j_and_k_move_and_h_and_l_change_the_value() {
        let mut p = popup(&["x"]);
        p.key(KeyCode::Down);
        assert_eq!(p.rows()[p.cursor()], Row::Format);
        p.key(KeyCode::Char('l'));
        assert_eq!(p.format(), Format::TarGz);
        p.key(KeyCode::Char('l'));
        assert_eq!(p.format(), Format::Tar);
        p.key(KeyCode::Char('l'));
        assert_eq!(p.format(), Format::Zip);
        p.key(KeyCode::Char('h'));
        assert_eq!(p.format(), Format::Tar);
        p.key(KeyCode::Char('j'));
        assert_eq!(p.rows()[p.cursor()], Row::Level);
        p.key(KeyCode::Char('k'));
        assert_eq!(p.rows()[p.cursor()], Row::Format);
    }

    #[test]
    fn focus_wraps_both_ways() {
        let mut p = popup(&["x"]);
        p.key(KeyCode::Up);
        assert_eq!(p.rows()[p.cursor()], Row::DeleteOriginals);
        p.key(KeyCode::Tab);
        assert_eq!(p.rows()[p.cursor()], Row::Name);
        p.key(KeyCode::BackTab);
        assert_eq!(p.rows()[p.cursor()], Row::DeleteOriginals);
    }

    #[test]
    fn the_level_is_chosen_and_ignored_for_a_plain_tar() {
        let mut p = popup(&["x"]);
        p.focus(2);
        p.key(KeyCode::Right);
        assert_eq!(submitted(p.key(KeyCode::Enter)).level, Level::Best);

        p.focus(1);
        p.key(KeyCode::Left);
        assert_eq!(p.format(), Format::Tar);
        assert!(!p.level_applies());
        p.focus(2);
        p.key(KeyCode::Right);
        assert_eq!(submitted(p.key(KeyCode::Enter)).level, Level::Best);
        assert_eq!(p.value(Row::Level), "(a plain tar is not compressed)");
    }

    #[test]
    fn an_empty_or_reserved_name_is_an_error_not_a_job() {
        for bad in ["", "   ", ".", ".."] {
            let mut p = popup(&["x"]);
            clear_name(&mut p);
            type_text(&mut p, bad);
            assert_eq!(p.key(KeyCode::Enter), Key::Stay, "{bad:?}");
            assert!(p.error().is_some(), "{bad:?}");
        }
    }

    #[test]
    fn the_error_clears_on_the_next_key() {
        let mut p = popup(&["x"]);
        clear_name(&mut p);
        p.key(KeyCode::Enter);
        assert!(p.error().is_some());
        p.key(KeyCode::Char('a'));
        assert!(p.error().is_none());
    }

    #[test]
    fn a_typed_extension_picks_the_format_instead_of_doubling() {
        let mut p = popup(&["x"]);
        clear_name(&mut p);
        type_text(&mut p, "backup.tar.gz");
        let request = submitted(p.key(KeyCode::Enter));
        assert_eq!(request.format, Format::TarGz);
        assert_eq!(
            request.layout,
            Layout::One {
                file_name: "backup.tar.gz".into()
            }
        );
    }

    #[test]
    fn one_archive_per_item_ignores_the_name() {
        let mut p = popup(&["a", "b"]);
        p.focus(3);
        assert_eq!(p.rows()[3], Row::PerItem);
        p.key(KeyCode::Char(' '));
        assert!(!p.name_applies());
        p.focus(0);
        type_text(&mut p, "zzz");
        assert_eq!(p.name(), "archive");
        let request = submitted(p.key(KeyCode::Enter));
        assert_eq!(request.layout, Layout::PerItem);
        assert_eq!(request.sources.len(), 2);
    }

    #[test]
    fn delete_originals_toggles() {
        let mut p = popup(&["x"]);
        p.focus(3);
        assert_eq!(p.rows()[3], Row::DeleteOriginals);
        p.key(KeyCode::Right);
        assert!(submitted(p.key(KeyCode::Enter)).delete_originals);
        p.key(KeyCode::Right);
        assert!(!submitted(p.key(KeyCode::Enter)).delete_originals);
    }

    #[test]
    fn escape_closes() {
        assert_eq!(popup(&["x"]).key(KeyCode::Esc), Key::Close);
    }

    proptest! {
        /// Whatever is typed, a submitted file name is one plain path component that can only
        /// land inside the chosen directory.
        #[test]
        fn a_submitted_name_is_always_a_single_safe_component(
            typed in prop::collection::vec(any::<char>(), 0..40),
            format_steps in 0usize..3,
        ) {
            let mut p = popup(&["seed"]);
            for c in typed {
                p.key(KeyCode::Char(c));
            }
            p.focus(1);
            for _ in 0..format_steps {
                p.key(KeyCode::Right);
            }
            if let Key::Submit(request) = p.key(KeyCode::Enter) {
                let Layout::One { file_name } = request.layout else {
                    return Err(TestCaseError::fail("expected one archive"));
                };
                let path = Path::new(&file_name);
                prop_assert_eq!(path.components().count(), 1);
                prop_assert!(matches!(
                    path.components().next(),
                    Some(std::path::Component::Normal(_))
                ));
                prop_assert!(!file_name.chars().any(char::is_control));
                prop_assert_eq!(Format::of(path), Some(request.format));
            }
        }
    }
}
