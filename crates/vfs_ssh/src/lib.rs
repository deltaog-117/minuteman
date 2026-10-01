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

//! Native browsing of remote machines over SSH, without a mounted filesystem.
//!
//! A remote location is an ordinary path whose text begins with `ssh://` (see [`target`]), so the
//! rest of the file manager carries it like any other path. [`RoutedVfs`] is the [`shared::Vfs`]
//! the application is built with: it sends a local path to the local disk and a remote one to an
//! SFTP session with that machine.
//!
//! The SFTP session is the system `ssh` program's own (`ssh -s sftp`), so host keys, `~/.ssh/config`,
//! agents and `ProxyJump` behave exactly as they do in a terminal.

mod error;
mod executor;
mod link;
mod routed;
mod stream;
pub mod target;

pub use link::{ConnectOptions, HostKeys};
pub use routed::RoutedVfs;
pub use target::{Host, Location, Remote, Target, TargetError, User, is_remote};
