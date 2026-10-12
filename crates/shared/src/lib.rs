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

//! Core, read-only infrastructure shared by every Minuteman feature crate.
//!
//! This crate must never depend on a feature crate (`browser`, `file_ops`, `theming`, ...) —
//! dependencies always point inward, toward `shared`.

pub mod archive_path;
pub mod chooser;
pub mod conformance;
pub mod devices;
pub mod error;
pub mod format;
pub mod fuzzy;
pub mod memory;
pub mod mount;
pub mod names;
pub mod ssh_path;
pub mod tally;
pub mod usage;
pub mod usage_view;
pub mod vfs;

pub use archive_path::{Format, Mount, Route, is_archive, is_inside, route};
pub use error::VfsError;
pub use fuzzy::Query;
pub use memory::MemVfs;
pub use mount::{MOUNT_MARK, archive_of_root, mount_root};
pub use names::{clean_name, is_image, is_text};
pub use ssh_path::{Host, Location, Remote, Target, TargetError, User, is_remote};
pub use vfs::{
    DirEntryInfo, FileKind, LocalVfs, Metadata, ReadSeek, ScanEntry, Vfs, WriteSeek, create_dir_all,
};
