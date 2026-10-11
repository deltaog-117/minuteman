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

//! The fuzzy finder (`f`), as a plugin. When the user triggers the `find` action the host sends an
//! `action` event; this plugin opens a picker (`show_picker`), asks the host to walk the tree
//! below the browsed folder (`walk`) and ranks the entries that come back against what is typed,
//! sending the best of them back with `picker_items`. The host draws the list and the preview and
//! shows the chosen entry in the browser; nothing here touches the disk or the terminal.
//!
//! `rank` scores and orders entries, `session` is the finder's state and how it answers each event,
//! and `run` drives it from the lines on stdin.

pub mod rank;
pub mod run;
pub mod session;

pub use run::run;
