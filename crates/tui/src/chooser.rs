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

//! Pick mode, for `--chooser-file`: another program (a browser's save dialog, through the portal
//! backend) borrows the file manager to have the user choose a path. Browsing is untouched; what
//! changes is what opening a file and `Q` mean, and the answer goes to a file instead of the
//! screen (see `shared::chooser` for its format).
//!
//! Two keys decide: opening a file (`Enter`, a double-click) picks it, and `Q` — which already
//! means "finish here" — confirms where the cursor is. Backing out with `q` writes an empty answer.

use std::io;
use std::path::{Path, PathBuf};

use browser::BrowserState;
use shared::chooser::{PickKind, Selection};
use shared::{FileKind, Vfs};

use crate::app::{App, Prompt};
use crate::cli::{self, ChooserArgs};
use crate::line_edit::LineEdit;

const NOT_LOCAL: &str = "that is on another machine or inside an archive: pick one on this disk";

/// The pick in progress: where the answer goes, and what is being asked for.
#[derive(Debug)]
pub struct Chooser {
    file: PathBuf,
    kind: PickKind,
    suggested_name: Option<String>,
}

/// What a key means, given the pick being made.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The question is answered.
    Pick(Selection),
    /// A file is to be saved: ask for its name, starting from `name`.
    AskName { dir: PathBuf, name: String },
    /// This key cannot answer it; say why.
    Refuse(&'static str),
}

/// Where the user is when they confirm.
pub struct Here<'a> {
    pub dir: &'a Path,
    /// The row under the cursor and whether it is a folder.
    pub cursor: Option<(&'a Path, bool)>,
    /// The marked files.
    pub marks: &'a [PathBuf],
}

#[derive(Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    /// A name is one path component: no slash, not `.` or `..`.
    NotAName,
}

/// The path to save to: `name` directly inside `dir`. Whatever is typed, the result never leaves
/// `dir`, so a slash or `..` cannot aim a save somewhere the folder on screen is not.
pub fn save_target(dir: &Path, name: &str) -> Result<PathBuf, NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name == "." || name == ".." || name.contains(['/', '\0']) {
        return Err(NameError::NotAName);
    }
    Ok(dir.join(name))
}

/// Whether a program on this machine can be handed `path`: not on another machine, not inside an
/// archive that exists only as a view in this process.
fn is_local(path: &Path) -> bool {
    !shared::ssh_path::is_remote(path) && !vfs_archive::is_inside(path)
}

fn pick(paths: Vec<PathBuf>) -> Verdict {
    if !paths.iter().all(|path| is_local(path)) {
        return Verdict::Refuse(NOT_LOCAL);
    }
    match Selection::try_new(paths) {
        Ok(selection) => Verdict::Pick(selection),
        Err(_) => Verdict::Refuse("nothing to pick here"),
    }
}

impl Chooser {
    pub fn new(args: ChooserArgs) -> Self {
        Self {
            file: args.file,
            kind: args.kind,
            suggested_name: args.suggested_name,
        }
    }

    /// The first line of help, shown until something else needs the status bar.
    pub fn hint(&self) -> &'static str {
        match self.kind {
            PickKind::File => "pick a file: enter chooses it · q cancels",
            PickKind::Files => {
                "pick files: v marks, enter or Q chooses the marked ones · q cancels"
            }
            PickKind::Directory => "pick a folder: Q chooses it · q cancels",
            PickKind::Save => {
                "save: Q names the file here, enter on a file replaces it · q cancels"
            }
        }
    }

    /// A file was opened (`Enter`, a double-click, the menu's Open).
    pub fn on_open_file(&self, path: &Path, dir: &Path, marks: &[PathBuf]) -> Verdict {
        match self.kind {
            PickKind::File => pick(vec![path.to_path_buf()]),
            PickKind::Files if marks.is_empty() => pick(vec![path.to_path_buf()]),
            PickKind::Files => pick(marks.to_vec()),
            PickKind::Directory => Verdict::Refuse("pick a folder, not a file"),
            PickKind::Save => Verdict::AskName {
                dir: dir.to_path_buf(),
                name: path
                    .file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
            },
        }
    }

    /// `Q`: choose where the cursor is.
    pub fn on_confirm(&self, here: &Here<'_>) -> Verdict {
        match self.kind {
            PickKind::File => match here.cursor {
                Some((path, false)) => pick(vec![path.to_path_buf()]),
                _ => Verdict::Refuse("move onto a file, then press enter"),
            },
            PickKind::Files => match (here.marks, here.cursor) {
                ([], Some((path, false))) => pick(vec![path.to_path_buf()]),
                ([], _) => Verdict::Refuse("mark some files with v first"),
                (marks, _) => pick(marks.to_vec()),
            },
            PickKind::Directory => match here.cursor {
                Some((path, true)) => pick(vec![path.to_path_buf()]),
                _ => pick(vec![here.dir.to_path_buf()]),
            },
            PickKind::Save => Verdict::AskName {
                dir: here.dir.to_path_buf(),
                name: self.suggested_name.clone().unwrap_or_default(),
            },
        }
    }

    /// Writes the answer; `None` writes the empty file that means "backed out", so an answer left
    /// there by an earlier run can never be mistaken for this one.
    pub fn write_answer(&self, picked: Option<&Selection>) -> io::Result<()> {
        cli::write_chooser_file(
            &self.file,
            &picked.map(Selection::encode).unwrap_or_default(),
        )
    }
}

impl App {
    pub fn begin_choosing(&mut self, args: ChooserArgs) {
        let chooser = Chooser::new(args);
        self.status = Some(chooser.hint().into());
        self.chooser = Some(chooser);
    }

    /// What `main` writes once the loop is over: `None` while the user has not chosen yet.
    pub fn take_chooser(&mut self) -> (Option<Chooser>, Option<Selection>) {
        (self.chooser.take(), self.picked.take())
    }

    /// Whether a pick is in progress and the loop should end because it has been answered.
    pub fn is_answered(&self) -> bool {
        self.picked.is_some()
    }

    /// A file was opened while picking. Returns `false` when there is no pick, so the caller
    /// opens it the usual way.
    pub fn choose_file(&mut self, browser: &BrowserState, vfs: &dyn Vfs, path: &Path) -> bool {
        let Some(chooser) = &self.chooser else {
            return false;
        };
        let marks = marked_files(vfs, browser);
        let verdict = chooser.on_open_file(path, browser.current_dir(), &marks);
        self.apply_verdict(verdict);
        true
    }

    /// `Q` while picking. Returns `false` when there is no pick, so it quits as usual.
    pub fn choose_here(&mut self, browser: &BrowserState, vfs: &dyn Vfs) -> bool {
        let Some(chooser) = &self.chooser else {
            return false;
        };
        let marks = marked_files(vfs, browser);
        let cursor = browser
            .selected_entry()
            .map(|entry| (entry.path.as_path(), entry.is_dir));
        let verdict = chooser.on_confirm(&Here {
            dir: browser.current_dir(),
            cursor,
            marks: &marks,
        });
        self.apply_verdict(verdict);
        true
    }

    fn apply_verdict(&mut self, verdict: Verdict) {
        match verdict {
            Verdict::Pick(selection) => self.picked = Some(selection),
            Verdict::AskName { dir, name } => {
                self.prompt = Some(Prompt::SaveName {
                    dir,
                    buffer: LineEdit::new(name),
                });
            }
            Verdict::Refuse(why) => self.status = Some(why.into()),
        }
    }

    /// `Enter` on the save-name prompt: the name becomes a path, and an existing file is only
    /// replaced once the user says so.
    pub fn finish_save_name(&mut self, vfs: &dyn Vfs, dir: PathBuf, buffer: LineEdit) {
        let target = match save_target(&dir, &buffer.to_string()) {
            Ok(target) => target,
            Err(error) => {
                self.status = Some(
                    match error {
                        NameError::Empty => "a file needs a name",
                        NameError::NotAName => "a name cannot hold a slash, and cannot be . or ..",
                    }
                    .into(),
                );
                self.prompt = Some(Prompt::SaveName { dir, buffer });
                return;
            }
        };
        if !is_local(&target) {
            self.status = Some(NOT_LOCAL.into());
            self.prompt = Some(Prompt::SaveName { dir, buffer });
            return;
        }
        match vfs.metadata(&target).map(|meta| meta.kind) {
            Ok(FileKind::Dir) => {
                self.status = Some("a folder with that name is already there".into());
                self.prompt = Some(Prompt::SaveName { dir, buffer });
            }
            Ok(_) => self.prompt = Some(Prompt::ConfirmReplace { target }),
            Err(_) => self.accept_save(target),
        }
    }

    /// The user agreed to replace `target`, or it did not exist.
    pub fn accept_save(&mut self, target: PathBuf) {
        if let Ok(selection) = Selection::try_new(vec![target]) {
            self.picked = Some(selection);
        } else {
            self.status = Some("that path is not absolute".into());
        }
    }
}

/// The marked entries that are not folders: a file pick has no use for a folder.
fn marked_files(vfs: &dyn Vfs, browser: &BrowserState) -> Vec<PathBuf> {
    browser
        .marked_paths()
        .into_iter()
        .filter(|path| {
            vfs.metadata(path)
                .is_ok_and(|meta| meta.kind != FileKind::Dir)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::fs;

    fn chooser(kind: PickKind, name: Option<&str>) -> Chooser {
        Chooser::new(ChooserArgs {
            file: "/out".into(),
            kind,
            suggested_name: name.map(str::to_owned),
        })
    }

    fn picked(verdict: Verdict) -> Vec<PathBuf> {
        match verdict {
            Verdict::Pick(selection) => selection.into_paths(),
            other => panic!("expected a pick, got {other:?}"),
        }
    }

    #[test]
    fn opening_a_file_picks_it_for_a_file_pick() {
        let c = chooser(PickKind::File, None);
        assert_eq!(
            picked(c.on_open_file(Path::new("/d/a.txt"), Path::new("/d"), &[])),
            [PathBuf::from("/d/a.txt")]
        );
    }

    #[test]
    fn marks_win_over_the_cursor_for_many_files() {
        let c = chooser(PickKind::Files, None);
        let marks = [PathBuf::from("/d/x"), PathBuf::from("/d/y")];
        assert_eq!(
            picked(c.on_open_file(Path::new("/d/a"), Path::new("/d"), &marks)),
            marks
        );
        assert_eq!(
            picked(c.on_open_file(Path::new("/d/a"), Path::new("/d"), &[])),
            [PathBuf::from("/d/a")]
        );
    }

    #[test]
    fn a_folder_pick_refuses_a_file_and_takes_the_cursor_folder_or_the_current_one() {
        let c = chooser(PickKind::Directory, None);
        assert!(matches!(
            c.on_open_file(Path::new("/d/a"), Path::new("/d"), &[]),
            Verdict::Refuse(_)
        ));
        let on_folder = Here {
            dir: Path::new("/d"),
            cursor: Some((Path::new("/d/sub"), true)),
            marks: &[],
        };
        assert_eq!(picked(c.on_confirm(&on_folder)), [PathBuf::from("/d/sub")]);
        let on_file = Here {
            dir: Path::new("/d"),
            cursor: Some((Path::new("/d/a"), false)),
            marks: &[],
        };
        assert_eq!(picked(c.on_confirm(&on_file)), [PathBuf::from("/d")]);
    }

    #[test]
    fn a_file_pick_will_not_confirm_a_folder() {
        let c = chooser(PickKind::File, None);
        let here = Here {
            dir: Path::new("/d"),
            cursor: Some((Path::new("/d/sub"), true)),
            marks: &[],
        };
        assert!(matches!(c.on_confirm(&here), Verdict::Refuse(_)));
    }

    #[test]
    fn saving_asks_for_a_name_starting_from_the_suggestion_or_the_opened_file() {
        let c = chooser(PickKind::Save, Some("report.pdf"));
        let here = Here {
            dir: Path::new("/d"),
            cursor: None,
            marks: &[],
        };
        assert_eq!(
            c.on_confirm(&here),
            Verdict::AskName {
                dir: "/d".into(),
                name: "report.pdf".into()
            }
        );
        assert_eq!(
            c.on_open_file(Path::new("/d/old.pdf"), Path::new("/d"), &[]),
            Verdict::AskName {
                dir: "/d".into(),
                name: "old.pdf".into()
            }
        );
    }

    #[test]
    fn a_path_on_another_machine_is_refused() {
        let c = chooser(PickKind::File, None);
        assert_eq!(
            c.on_open_file(Path::new("ssh://host/a"), Path::new("ssh://host"), &[]),
            Verdict::Refuse(NOT_LOCAL)
        );
    }

    #[test]
    fn the_answer_file_holds_the_pick_or_nothing() {
        let file = std::env::temp_dir().join(format!("minuteman-pick-{}", std::process::id()));
        let c = Chooser::new(ChooserArgs {
            file: file.clone(),
            kind: PickKind::File,
            suggested_name: None,
        });
        let selection = Selection::try_new(vec!["/d/a".into()]).expect("absolute");
        c.write_answer(Some(&selection))
            .expect("temp dir is writable");
        let written = fs::read(&file).expect("just written");
        assert_eq!(Selection::decode(&written), Ok(Some(selection)));
        c.write_answer(None).expect("temp dir is writable");
        let emptied = fs::read(&file).expect("just written");
        let _ = fs::remove_file(&file);
        assert_eq!(Selection::decode(&emptied), Ok(None));
    }

    #[test]
    fn q_picks_every_marked_file_and_skips_marked_folders() {
        let dir = std::env::temp_dir().join(format!("minuteman-marks-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).expect("temp dir is writable");
        for name in ["a.txt", "b.txt"] {
            fs::write(dir.join(name), b"").expect("temp dir is writable");
        }
        let vfs = shared::LocalVfs;
        let mut browser = BrowserState::with_show_hidden(&vfs, dir.clone(), false).expect("lists");
        for _ in 0..browser.current_entries().len() {
            browser.toggle_mark();
            browser.move_down();
        }
        let (runtime, mut app) = {
            let runtime = tokio::runtime::Runtime::new().expect("runtime");
            let app = App::new(runtime.handle().clone());
            (runtime, app)
        };
        app.begin_choosing(ChooserArgs {
            file: dir.join("answer"),
            kind: PickKind::Files,
            suggested_name: None,
        });
        assert!(app.choose_here(&browser, &vfs));
        let picked = app.picked.take().expect("answered").into_paths();
        drop(runtime);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(picked, [dir.join("a.txt"), dir.join("b.txt")]);
    }

    #[test]
    fn names_that_are_not_one_component_are_refused() {
        let dir = Path::new("/d");
        assert_eq!(save_target(dir, ""), Err(NameError::Empty));
        for bad in [".", "..", "a/b", "/etc/passwd", "../x", "a\0b"] {
            assert_eq!(save_target(dir, bad), Err(NameError::NotAName), "{bad:?}");
        }
        assert_eq!(save_target(dir, "a b.txt"), Ok(PathBuf::from("/d/a b.txt")));
    }

    proptest! {
        /// Whatever is typed, a saved path is directly inside the folder on screen.
        #[test]
        fn a_save_target_never_leaves_its_folder(name in ".{0,24}") {
            let dir = Path::new("/d/here");
            if let Ok(target) = save_target(dir, &name) {
                prop_assert_eq!(target.parent(), Some(dir));
            }
        }
    }
}
