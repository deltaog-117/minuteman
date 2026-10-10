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

//! The contract between `mman --chooser-file` and whatever asked it to pick something (the portal
//! backend): what kind of pick it is, and how the answer travels back through the file.
//!
//! The answer is the chosen absolute paths, each followed by a NUL byte. NUL is the one byte a
//! Unix path can never contain, so no file name — newline or invalid UTF-8 included — can be
//! misread. An empty file means the user backed out.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// What the caller wants picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickKind {
    /// One existing file.
    #[default]
    File,
    /// One or more existing files.
    Files,
    /// One folder.
    Directory,
    /// A path that may not exist yet, to write a file to.
    Save,
}

impl PickKind {
    /// The spelling used on the command line (`--pick <name>`).
    pub fn name(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Files => "files",
            Self::Directory => "directory",
            Self::Save => "save",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        [Self::File, Self::Files, Self::Directory, Self::Save]
            .into_iter()
            .find(|kind| kind.name() == name)
    }

    /// Whether more than one path is a valid answer.
    pub fn allows_many(self) -> bool {
        self == Self::Files
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectionError {
    #[error("nothing was chosen")]
    Empty,
    #[error("not an absolute path: {0}")]
    NotAbsolute(PathBuf),
}

/// One or more absolute paths the user chose. Never empty, and every path is absolute: the only
/// way to hold one is [`Selection::try_new`] or [`Selection::decode`], so a consumer never has to
/// ask whether "chose nothing" or "chose a relative path" is possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection(Vec<PathBuf>);

impl Selection {
    pub fn try_new(paths: Vec<PathBuf>) -> Result<Self, SelectionError> {
        if paths.is_empty() {
            return Err(SelectionError::Empty);
        }
        if let Some(relative) = paths.iter().find(|path| !path.is_absolute()) {
            return Err(SelectionError::NotAbsolute(relative.clone()));
        }
        Ok(Self(paths))
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.0
    }

    pub fn into_paths(self) -> Vec<PathBuf> {
        self.0
    }

    /// The bytes to write to the chooser file.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for path in &self.0 {
            out.extend_from_slice(path.as_os_str().as_bytes());
            out.push(0);
        }
        out
    }

    /// Reads a chooser file back. `Ok(None)` is the user backing out (an empty file); an `Err`
    /// is a file that is not an answer at all, which the caller should treat as a failure rather
    /// than as a cancel.
    pub fn decode(bytes: &[u8]) -> Result<Option<Self>, SelectionError> {
        if bytes.is_empty() {
            return Ok(None);
        }
        // Every path ends in NUL, so splitting leaves one empty tail that is not a path.
        let mut parts: Vec<&[u8]> = bytes.split(|&byte| byte == 0).collect();
        if parts.pop().is_some_and(|tail| !tail.is_empty()) {
            // The last path has no terminator: a write cut short, not an answer.
            return Err(SelectionError::Empty);
        }
        let paths = parts
            .into_iter()
            .map(|part| Path::new(OsStr::from_bytes(part)).to_path_buf())
            .collect();
        Self::try_new(paths).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn kinds_round_trip_through_their_names() {
        for kind in [
            PickKind::File,
            PickKind::Files,
            PickKind::Directory,
            PickKind::Save,
        ] {
            assert_eq!(PickKind::parse(kind.name()), Some(kind));
        }
        assert_eq!(PickKind::parse("folder"), None);
    }

    #[test]
    fn only_files_allows_many() {
        assert!(PickKind::Files.allows_many());
        assert!(!PickKind::File.allows_many());
        assert!(!PickKind::Directory.allows_many());
        assert!(!PickKind::Save.allows_many());
    }

    #[test]
    fn an_empty_selection_is_refused() {
        assert_eq!(Selection::try_new(vec![]), Err(SelectionError::Empty));
    }

    #[test]
    fn a_relative_path_is_refused() {
        assert_eq!(
            Selection::try_new(vec!["/a".into(), "b/c".into()]),
            Err(SelectionError::NotAbsolute("b/c".into()))
        );
    }

    #[test]
    fn an_empty_file_is_a_cancel() {
        assert_eq!(Selection::decode(b""), Ok(None));
    }

    #[test]
    fn a_missing_terminator_is_not_an_answer() {
        assert!(Selection::decode(b"/a\0/b").is_err());
    }

    #[test]
    fn a_relative_path_in_the_file_is_not_an_answer() {
        assert_eq!(
            Selection::decode(b"a/b\0"),
            Err(SelectionError::NotAbsolute("a/b".into()))
        );
    }

    #[test]
    fn newlines_and_invalid_utf8_survive() {
        let selection = Selection::try_new(vec![
            PathBuf::from("/a\nb"),
            PathBuf::from(OsStr::from_bytes(b"/\xff")),
        ])
        .expect("absolute");
        assert_eq!(Selection::decode(&selection.encode()), Ok(Some(selection)));
    }

    proptest! {
        /// Whatever bytes the names hold, what is written is exactly what is read back.
        #[test]
        fn encode_then_decode_is_the_identity(
            names in proptest::collection::vec(
                proptest::collection::vec(1u8..=255, 1..24)
                    .prop_filter("no slash inside a name", |n| !n.contains(&b'/')),
                1..6,
            )
        ) {
            let paths: Vec<PathBuf> = names
                .iter()
                .map(|name| Path::new("/").join(OsStr::from_bytes(name)))
                .collect();
            let selection = Selection::try_new(paths).expect("absolute and non-empty");
            prop_assert_eq!(Selection::decode(&selection.encode()), Ok(Some(selection)));
        }

        /// Bytes that are not an answer never decode into something that looks like one.
        #[test]
        fn decoded_selections_are_always_valid(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
            if let Ok(Some(selection)) = Selection::decode(&bytes) {
                prop_assert!(!selection.paths().is_empty());
                prop_assert!(selection.paths().iter().all(|path| path.is_absolute()));
            }
        }
    }
}
