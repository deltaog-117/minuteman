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

//! Sends a path to the real desktop trash — the freedesktop.org trash on Linux, the Recycle Bin
//! on Windows, the Trash on macOS — the same one Nautilus, Dolphin, Explorer and Finder use, via
//! the `trash` crate (imported here as `os_trash`; this crate is itself named `trash`, which
//! `cargo` won't let depend on the identically-named crates.io package under its own name).
//!
//! This is a local-filesystem concept with no equivalent in the `Vfs` trait `shared`/`file_ops`
//! build everything else on — there is no "SSH trash" — so, unlike the rest of `file_ops`, sending
//! something here is not backend-agnostic.

use std::path::Path;

/// Wrapped as plain text rather than depending on `os_trash::Error`'s own trait shape, which
/// this crate has no control over.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct TrashError(String);

/// Moves `path` to the desktop trash.
pub fn send(path: &Path) -> Result<(), TrashError> {
    os_trash::delete(path).map_err(|e| TrashError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sending_a_file_removes_it_from_its_original_location() {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-trash-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.txt");
        std::fs::write(&file, b"hi").unwrap();

        send(&file).unwrap();

        assert!(!file.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sending_a_missing_path_fails() {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-trash-test-missing-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("nope.txt");

        assert!(send(&missing).is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
