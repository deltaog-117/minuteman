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

//! Telling a path inside an archive (`/x/a.zip!/dir/f`) from an ordinary one, and splitting it
//! into the archive and the place within it.
//!
//! The split is done on the path's text and never by rebuilding it from components, because a
//! component walk would collapse the `//` of `ssh://host/...` and change which machine the archive
//! is on.

use std::path::{Path, PathBuf};

use shared::MOUNT_MARK;

/// The archive formats that can be browsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    TarGz,
}

impl Format {
    /// Which format a file name says it is (`.zip`, `.jar`, `.tar`, `.tar.gz`, `.tgz`), ignoring
    /// case. A name that is nothing but the extension is not an archive.
    pub fn of(name: &str) -> Option<Self> {
        const SUFFIXES: [(&str, Format); 5] = [
            (".zip", Format::Zip),
            (".jar", Format::Zip),
            (".tar.gz", Format::TarGz),
            (".tgz", Format::TarGz),
            (".tar", Format::Tar),
        ];
        let lower = name.to_lowercase();
        SUFFIXES.iter().find_map(|&(suffix, format)| {
            let stem = lower.strip_suffix(suffix)?;
            (!stem.is_empty()).then_some(format)
        })
    }
}

/// A path that is inside an archive, split into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    /// The archive file itself, as the backend underneath knows it.
    pub archive: PathBuf,
    /// The archive's root as a folder (`archive` plus the mark); every path in it starts here.
    pub root: PathBuf,
    pub format: Format,
    /// Where in the archive, as `/`-joined names with no `.`, `..` or empty parts; empty for the
    /// root.
    pub entry: String,
}

/// What a path is, as far as archives go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Not in an archive: pass it to the backend underneath untouched.
    Outside,
    Inside(Mount),
    /// In an archive but climbing out of it with `..`, which there is nothing to climb out to.
    Escapes,
}

/// Decides where `path` goes. The outermost archive wins, so an archive inside an archive is an
/// entry of the first (browsing it as a folder in turn is not done).
pub fn route(path: &Path) -> Route {
    let Some(text) = path.to_str() else {
        return Route::Outside;
    };
    for (at, _) in text.match_indices(MOUNT_MARK) {
        let after = &text[at + MOUNT_MARK.len_utf8()..];
        if !(after.is_empty() || after.starts_with('/')) {
            continue;
        }
        let name_start = text[..at].rfind('/').map_or(0, |slash| slash + 1);
        let Some(format) = Format::of(&text[name_start..at]) else {
            continue;
        };
        let mut parts: Vec<&str> = Vec::new();
        for part in after.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Route::Escapes;
                    }
                }
                name => parts.push(name),
            }
        }
        return Route::Inside(Mount {
            archive: PathBuf::from(&text[..at]),
            root: PathBuf::from(&text[..at + MOUNT_MARK.len_utf8()]),
            format,
            entry: parts.join("/"),
        });
    }
    Route::Outside
}

/// Whether `path` is an archive's root or something in an archive.
pub fn is_inside(path: &Path) -> bool {
    !matches!(route(path), Route::Outside)
}

/// Whether `path` names a file that can be browsed as a folder.
pub fn is_archive(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| Format::of(name).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn inside(path: &str) -> Mount {
        match route(Path::new(path)) {
            Route::Inside(mount) => mount,
            other => panic!("{path} routed to {other:?}"),
        }
    }

    #[test]
    fn formats_come_from_the_name_and_a_bare_extension_is_not_one() {
        assert_eq!(Format::of("A.ZIP"), Some(Format::Zip));
        assert_eq!(Format::of("a.jar"), Some(Format::Zip));
        assert_eq!(Format::of("a.tar"), Some(Format::Tar));
        assert_eq!(Format::of("a.tar.gz"), Some(Format::TarGz));
        assert_eq!(Format::of("a.TGZ"), Some(Format::TarGz));
        for other in [
            ".zip", "zip", "a.gz", "a.txt", "a.tar.xz", "a.7z", ".tar.gz",
        ] {
            assert_eq!(Format::of(other), None, "{other}");
        }
    }

    #[test]
    fn a_path_splits_into_archive_and_entry() {
        let root = inside("/x/a.zip!");
        assert_eq!(root.archive, Path::new("/x/a.zip"));
        assert_eq!(root.root, Path::new("/x/a.zip!"));
        assert_eq!(root.entry, "");

        let deep = inside("/x/a.zip!/dir//./f.txt");
        assert_eq!(deep.entry, "dir/f.txt");
        assert_eq!(deep.format, Format::Zip);
        assert_eq!(inside("/x/a.zip!/").entry, "");
    }

    #[test]
    fn the_scheme_of_a_remote_path_is_left_alone() {
        let mount = inside("ssh://me@host:22/srv/a.tar.gz!/d/f");
        assert_eq!(mount.archive, Path::new("ssh://me@host:22/srv/a.tar.gz"));
        assert_eq!(mount.entry, "d/f");
    }

    #[test]
    fn dot_dot_stays_inside_or_escapes() {
        assert_eq!(inside("/x/a.zip!/d/../e").entry, "e");
        assert_eq!(route(Path::new("/x/a.zip!/..")), Route::Escapes);
        assert_eq!(route(Path::new("/x/a.zip!/d/../../e")), Route::Escapes);
    }

    #[test]
    fn only_a_mark_after_an_archive_name_counts() {
        for plain in [
            "/x/a.zip",
            "/x/plain!",
            "/x/a.zip!x/y",
            "/x/!/y",
            "/x/.zip!/y",
            "/x/notes!.txt",
        ] {
            assert_eq!(route(Path::new(plain)), Route::Outside, "{plain}");
        }
    }

    #[test]
    fn an_archive_in_an_archive_is_an_entry_of_the_outer_one() {
        let mount = inside("/x/a.zip!/b.zip!/c");
        assert_eq!(mount.archive, Path::new("/x/a.zip"));
        assert_eq!(mount.entry, "b.zip!/c");
    }

    proptest! {
        /// Any entry path written under a root comes back out of `route` exactly.
        #[test]
        fn entries_round_trip(parts in proptest::collection::vec("[a-zA-Z0-9 _-]{1,8}", 0..5)) {
            let path = format!("/x/a.zip!/{}", parts.join("/"));
            prop_assert_eq!(inside(&path).entry, parts.join("/"));
        }

        /// Routing never panics, whatever the text.
        #[test]
        fn routing_accepts_any_text(text in "\\PC{0,40}") {
            let _ = route(Path::new(&text));
        }
    }
}
