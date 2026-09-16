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

use std::path::{Path, PathBuf};

use crate::error::VfsError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntryInfo {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

pub trait Vfs {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, VfsError>;
    fn is_dir(&self, path: &Path) -> bool;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalVfs;

impl Vfs for LocalVfs {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, VfsError> {
        if !path.is_dir() {
            return Err(VfsError::NotADirectory(path.to_path_buf()));
        }

        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path).map_err(|source| VfsError::Io {
            path: path.to_path_buf(),
            source,
        })? {
            let entry = entry.map_err(|source| VfsError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            let entry_path = entry.path();
            let is_dir = entry_path.is_dir();
            let name = entry.file_name().to_string_lossy().into_owned();
            entries.push(DirEntryInfo {
                name,
                path: entry_path,
                is_dir,
            });
        }

        // Directories first, then alphabetical within each group.
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        Ok(entries)
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_directories_before_files_alphabetically() {
        let tmp = std::env::temp_dir().join(format!("minuteman-test-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("b_dir")).unwrap();
        std::fs::create_dir_all(tmp.join("a_dir")).unwrap();
        std::fs::write(tmp.join("a_file.txt"), b"hi").unwrap();

        let vfs = LocalVfs;
        let entries = vfs.list_dir(&tmp).unwrap();

        assert_eq!(entries[0].name, "a_dir");
        assert_eq!(entries[1].name, "b_dir");
        assert_eq!(entries[2].name, "a_file.txt");

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn rejects_non_directory_paths() {
        let vfs = LocalVfs;
        let tmp = std::env::temp_dir().join(format!("minuteman-test-file-{}", std::process::id()));
        std::fs::write(&tmp, b"hi").unwrap();

        let result = vfs.list_dir(&tmp);
        assert!(matches!(result, Err(VfsError::NotADirectory(_))));

        std::fs::remove_file(&tmp).unwrap();
    }
}
