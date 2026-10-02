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

//! How an archive that is being browsed as a folder is written as a path: the archive's own path
//! with a `!` added, so `/x/a.zip` opens as `/x/a.zip!` and a file in it is `/x/a.zip!/dir/f`.
//!
//! The spelling lives here, in the contract every feature may read, because both the backend that
//! serves such paths and the browser that steps back out of one need it, and neither may depend on
//! the other.

use std::path::{Path, PathBuf};

/// The character that marks an archive's root.
pub const MOUNT_MARK: char = '!';

/// The path that browses `archive` as a folder.
pub fn mount_root(archive: &Path) -> PathBuf {
    let mut text = archive.as_os_str().to_os_string();
    text.push(MOUNT_MARK.to_string());
    PathBuf::from(text)
}

/// The archive that `root` browses, if `root` is exactly an archive's root (`/x/a.zip!`), not
/// something inside it.
pub fn archive_of_root(root: &Path) -> Option<PathBuf> {
    let text = root.to_str()?.strip_suffix(MOUNT_MARK)?;
    (!text.is_empty() && !text.ends_with('/')).then(|| PathBuf::from(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Whatever the archive is called, opening it and stepping back out lands on the same path.
        #[test]
        fn mounting_then_unmounting_returns_the_archive(name in "[a-zA-Z0-9 ._-]{1,16}") {
            let archive = Path::new("/some/dir").join(name);
            prop_assert_eq!(archive_of_root(&mount_root(&archive)), Some(archive));
        }
    }

    #[test]
    fn only_a_root_unmounts() {
        assert_eq!(
            archive_of_root(Path::new("/x/a.zip!")),
            Some("/x/a.zip".into())
        );
        assert_eq!(archive_of_root(Path::new("/x/a.zip!/dir")), None);
        assert_eq!(archive_of_root(Path::new("/x/a.zip")), None);
        assert_eq!(archive_of_root(Path::new("/x/!")), None);
        assert_eq!(archive_of_root(Path::new("!")), None);
    }
}
