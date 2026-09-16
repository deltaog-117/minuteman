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

pub mod error;
pub mod vfs;

pub use error::VfsError;
pub use vfs::{DirEntryInfo, LocalVfs, Vfs};
