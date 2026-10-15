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

//! Serves the paths on remote machines (`ssh://host/dir`) to the host: each `vfs` request names a
//! full address, and the plugin answers it over an SFTP session it keeps with that machine.
//!
//! The session is the system `ssh` program's own (`ssh -s sftp`), so host keys, `~/.ssh/config`,
//! agents and `ProxyJump` behave exactly as they do in a terminal. The host never sees any of it:
//! it sends a [`plugin_api::VfsRequest`] and reads the reply.

mod connecting;
mod error;
mod executor;
mod link;
mod machines;
mod stream;

pub mod run;
pub mod serve;

pub use link::{ConnectOptions, HostKeys};
pub use run::run;
pub use serve::Server;
