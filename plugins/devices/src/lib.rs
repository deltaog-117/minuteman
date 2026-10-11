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

//! The rail of devices, as a plugin. The host says when the rail is showing (the `devices` event
//! with a `watch` command) and which device the user chose to open, mount or eject; the plugin
//! looks at this machine's disks, sends the list about once a second while it is watched, does
//! what was asked with `udisksctl` and reports what came of it. Nothing here touches the
//! terminal: the rail, the selection, the keys and the check of whether carried files will fit
//! stay in the interface.
//!
//! `scanner` looks at the machine on a thread of its own, `session` turns each event into a
//! command for it or an action on a thread, and `run` only moves lines between the host's pipes
//! and the session.

pub mod run;
pub mod scanner;
pub mod session;

pub use run::run;
