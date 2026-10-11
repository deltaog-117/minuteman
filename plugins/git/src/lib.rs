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

//! The git status, as a plugin. It follows the browsed folder and the cursor through the host's
//! `dir_changed` and `selection_changed` events, asks git about the folder's repository, and
//! fills three slots with `set_view`: the repository (branch, distance from its upstream, counts)
//! and the state of the entry under the cursor in the status bar, and a mark beside each changed
//! entry of the browsed folder in the list. The host decides the colors and
//! symbols and what to drop on a narrow bar; nothing here touches the terminal.
//!
//! `status` is git itself (running it and reading what it says), `view` turns an answer into what
//! the slots show, `session` is the state and how it reacts to each event, and `run` drives it
//! from the lines on stdin.

pub mod run;
pub mod session;
pub mod status;
pub mod view;

pub use run::run;
