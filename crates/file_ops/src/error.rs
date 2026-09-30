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

use std::path::PathBuf;

use shared::VfsError;

#[derive(Debug, thiserror::Error)]
pub enum FileOpsError {
    #[error(transparent)]
    Vfs(#[from] VfsError),
    #[error("source and destination are the same path: {0}")]
    SameLocation(PathBuf),
    #[error("destination '{dst}' is inside source '{src}'")]
    RecursiveDestination { src: PathBuf, dst: PathBuf },
    #[error("path has no parent directory: {0}")]
    NoParent(PathBuf),
    #[error("operation cancelled")]
    Cancelled,
    #[error("not a supported archive (zip, tar, tar.gz): {0}")]
    UnsupportedArchive(PathBuf),
    #[error("archive entry {name:?} would be written outside the destination")]
    UnsafeEntry { name: PathBuf },
    #[error("archive is over the {what} limit of {limit}")]
    ArchiveLimit { what: &'static str, limit: u64 },
    #[error("archive is damaged or unreadable: {0}")]
    Archive(String),
    #[error("nothing to archive")]
    NothingToArchive,
    #[error("name cannot be stored in the archive: {0}")]
    UnarchivableName(PathBuf),
    #[error("a password is needed to open {0}")]
    PasswordRequired(PathBuf),
    #[error("wrong password")]
    WrongPassword,
    #[error("the password is empty")]
    EmptyPassword,
    #[error("only zip archives can be encrypted")]
    EncryptionNeedsZip,
    #[error("could not send to trash: {0}")]
    Trash(#[from] trash::TrashError),
}
