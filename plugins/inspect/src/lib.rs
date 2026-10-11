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

//! The Inspect panel, as a plugin: everything a properties dialog shows for one file, folder or
//! link. The host names the entry (the `inspect` event); the plugin describes it with `stat`,
//! counts a folder's contents with `tally` (which takes seconds on a big tree, so the rows are sent
//! at once with "counting…" and again when the count lands), and answers with `set_inspect`.
//! Nothing here touches the terminal.
//!
//! `rows` words the facts as the panel's rows, and `session` turns the lines the host writes into
//! the lines to answer with. `run` only moves lines between them and stdin and stdout.

pub mod rows;
pub mod run;
pub mod session;

pub use run::run;
