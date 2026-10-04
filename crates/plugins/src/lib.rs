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

//! Runs Minuteman's plugins: user-configured external processes that speak a small, versioned
//! JSON-RPC-over-stdio protocol (see `protocol`), so a plugin can be written in any language that
//! can read a line and print one — no compile step, no host-side bindings per language. Each
//! plugin fires on its configured keys and `:` commands (or `:plugin <name>`) and can call back into `read_dir`/`copy`/`mv`/`delete`/
//! `create_dir`/`create_file`/`touch`/`rename` — the same `file_ops` orchestration the built-in
//! keys use, so a plugin reaches real core operations, not a bolted-on extras API.

pub mod error;
pub mod host;
mod mailbox;
pub mod protocol;

pub use error::PluginError;
pub use host::{LogMessage, PluginManager, Triggers};
pub use protocol::{API_VERSION, EventKind, JobOutcome, LogLevel, StateSnapshot};
