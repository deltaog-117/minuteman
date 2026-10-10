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

//! What a portal call asks for, reduced to what `mman` can be told.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use shared::chooser::PickKind;

/// The longest title handed to a terminal; a longer one is a mistake or an attack on the bar.
const TITLE_LIMIT: usize = 120;

/// The options of an `OpenFile`, `SaveFile` or `SaveFiles` call that change what is asked, as
/// plain values so they can be built and tested without D-Bus. Paths arrive as the portal sends
/// them: byte strings ending in a NUL.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RawOptions {
    pub multiple: bool,
    pub directory: bool,
    pub current_name: Option<String>,
    pub current_folder: Option<Vec<u8>>,
    pub current_file: Option<Vec<u8>>,
    /// `SaveFiles` only: the names of the files to be saved.
    pub files: Vec<Vec<u8>>,
}

/// One pick to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickRequest {
    pub kind: PickKind,
    pub title: String,
    /// Where to open the browser; `None` leaves it to `mman`.
    pub start_dir: Option<PathBuf>,
    /// The file name to offer when saving.
    pub name: Option<String>,
}

impl PickRequest {
    /// `OpenFile`. A folder pick takes one folder even when the caller allows several, because
    /// `mman` answers a folder pick with the folder under the cursor.
    pub fn open(title: &str, options: &RawOptions) -> Self {
        let kind = match (options.directory, options.multiple) {
            (true, _) => PickKind::Directory,
            (false, true) => PickKind::Files,
            (false, false) => PickKind::File,
        };
        Self {
            kind,
            title: clean_title(title),
            start_dir: options.current_folder.as_deref().and_then(path_from_bytes),
            name: None,
        }
    }

    /// `SaveFile`: the folder comes from `current_folder`, or failing that from where
    /// `current_file` lives, and the name from `current_name`, or failing that from `current_file`.
    pub fn save(title: &str, options: &RawOptions) -> Self {
        let current_file = options.current_file.as_deref().and_then(path_from_bytes);
        let start_dir = options
            .current_folder
            .as_deref()
            .and_then(path_from_bytes)
            .or_else(|| {
                current_file
                    .as_deref()
                    .and_then(Path::parent)
                    .map(Path::to_path_buf)
            });
        let name = options
            .current_name
            .clone()
            .filter(|name| !name.is_empty())
            .or_else(|| {
                current_file
                    .as_deref()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
            });
        Self {
            kind: PickKind::Save,
            title: clean_title(title),
            start_dir,
            name,
        }
    }

    /// `SaveFiles`: the user picks the folder the files go into.
    pub fn save_into_folder(title: &str, options: &RawOptions) -> Self {
        Self {
            kind: PickKind::Directory,
            title: clean_title(title),
            start_dir: options.current_folder.as_deref().and_then(path_from_bytes),
            name: None,
        }
    }
}

/// A path as the portal sends it (`ay`): the bytes with a terminating NUL. Anything that is not
/// one absolute path — empty, relative, with a NUL inside — is dropped, so what reaches `mman` is
/// always something it can open.
pub fn path_from_bytes(bytes: &[u8]) -> Option<PathBuf> {
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    if bytes.is_empty() || bytes.contains(&0) {
        return None;
    }
    let path = Path::new(OsStr::from_bytes(bytes));
    path.is_absolute().then(|| path.to_path_buf())
}

/// The file names of a `SaveFiles` call. A name that is not one plain component is dropped: the
/// reply is `folder/name`, and a name with a slash or `..` would point outside the folder.
pub fn file_names(files: &[Vec<u8>]) -> Vec<PathBuf> {
    files
        .iter()
        .filter_map(|bytes| {
            let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
            let name = Path::new(OsStr::from_bytes(bytes))
                .file_name()?
                .as_bytes()
                .to_vec();
            (name == bytes).then(|| PathBuf::from(OsStr::from_bytes(&name)))
        })
        .collect()
}

fn clean_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| !c.is_control())
        .take(TITLE_LIMIT)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn bytes(path: &str) -> Vec<u8> {
        let mut bytes = path.as_bytes().to_vec();
        bytes.push(0);
        bytes
    }

    #[test]
    fn open_picks_the_kind_from_the_flags() {
        let kind = |directory, multiple| {
            PickRequest::open(
                "Open",
                &RawOptions {
                    directory,
                    multiple,
                    ..RawOptions::default()
                },
            )
            .kind
        };
        assert_eq!(kind(false, false), PickKind::File);
        assert_eq!(kind(false, true), PickKind::Files);
        assert_eq!(kind(true, false), PickKind::Directory);
        assert_eq!(kind(true, true), PickKind::Directory);
    }

    #[test]
    fn save_takes_the_folder_and_name_from_the_current_file_when_nothing_else_is_given() {
        let request = PickRequest::save(
            "Save",
            &RawOptions {
                current_file: Some(bytes("/home/u/docs/report.pdf")),
                ..RawOptions::default()
            },
        );
        assert_eq!(request.start_dir, Some("/home/u/docs".into()));
        assert_eq!(request.name.as_deref(), Some("report.pdf"));
    }

    #[test]
    fn save_prefers_the_explicit_folder_and_name() {
        let request = PickRequest::save(
            "Save",
            &RawOptions {
                current_folder: Some(bytes("/tmp")),
                current_name: Some("a.txt".into()),
                current_file: Some(bytes("/home/u/docs/report.pdf")),
                ..RawOptions::default()
            },
        );
        assert_eq!(request.start_dir, Some("/tmp".into()));
        assert_eq!(request.name.as_deref(), Some("a.txt"));
    }

    #[test]
    fn a_relative_or_empty_folder_is_ignored() {
        for folder in [bytes("docs"), bytes(""), vec![], b"/a\0b\0".to_vec()] {
            let request = PickRequest::open(
                "Open",
                &RawOptions {
                    current_folder: Some(folder),
                    ..RawOptions::default()
                },
            );
            assert_eq!(request.start_dir, None);
        }
    }

    #[test]
    fn titles_lose_control_characters_and_are_cut() {
        let request = PickRequest::open(
            &format!("a\x1b[31mb\n{}", "x".repeat(500)),
            &RawOptions::default(),
        );
        assert!(request.title.starts_with("a[31mb"));
        assert!(request.title.chars().all(|c| !c.is_control()));
        assert_eq!(request.title.chars().count(), TITLE_LIMIT);
    }

    #[test]
    fn file_names_keep_only_plain_names() {
        let files = vec![
            bytes("a.txt"),
            bytes("../x"),
            bytes("d/e"),
            bytes(".."),
            bytes("b c"),
        ];
        assert_eq!(
            file_names(&files),
            [PathBuf::from("a.txt"), PathBuf::from("b c")]
        );
    }

    proptest! {
        /// Whatever bytes arrive, a start folder is absolute and has no NUL, or is dropped.
        #[test]
        fn a_start_folder_is_always_an_absolute_path(raw in proptest::collection::vec(any::<u8>(), 0..40)) {
            if let Some(path) = path_from_bytes(&raw) {
                prop_assert!(path.is_absolute());
                prop_assert!(!path.as_os_str().as_bytes().contains(&0));
            }
        }

        /// A saved file always lands directly in the chosen folder.
        #[test]
        fn a_listed_file_never_leaves_the_folder(raw in proptest::collection::vec(any::<u8>(), 0..24)) {
            for name in file_names(&[raw]) {
                prop_assert_eq!(name.components().count(), 1);
                prop_assert!(name != Path::new(".."));
            }
        }
    }
}
