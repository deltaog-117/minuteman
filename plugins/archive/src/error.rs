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

//! What can go wrong while packing or unpacking, and the small pieces every archive function
//! shares: the way a job reports an entry and is told to stop, and the guards on its paths.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use shared::VfsError;

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
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
}

/// What a compression did with its archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    /// The archive was already there and the policy was to leave it.
    Skipped,
}

/// Called after each entry is read or written. Returning `ControlFlow::Break(())` stops the job
/// with [`ArchiveError::Cancelled`] (whatever was already written stays on disk).
pub type ProgressFn<'a> = dyn FnMut(&Path) -> ControlFlow<()> + 'a;

pub(crate) fn report(on_progress: &mut ProgressFn<'_>, path: &Path) -> Result<(), ArchiveError> {
    match on_progress(path) {
        ControlFlow::Continue(()) => Ok(()),
        ControlFlow::Break(()) => Err(ArchiveError::Cancelled),
    }
}

pub(crate) fn guard_distinct(src: &Path, dst: &Path) -> Result<(), ArchiveError> {
    if src == dst {
        return Err(ArchiveError::SameLocation(src.to_path_buf()));
    }
    Ok(())
}

pub(crate) fn guard_not_recursive(src: &Path, dst: &Path) -> Result<(), ArchiveError> {
    if dst.starts_with(src) {
        return Err(ArchiveError::RecursiveDestination {
            src: src.to_path_buf(),
            dst: dst.to_path_buf(),
        });
    }
    Ok(())
}
