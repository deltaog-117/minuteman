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

//! `mman-atlas`: the map of the machine's storage, as a plugin. The host tells it when the atlas
//! view opens and closes, how many cells it has and what the user did in it (a key it claimed, a
//! click); it answers with a canvas to draw and, now and then, a note for the browser.

pub mod canvas;
pub mod layout;
pub mod run;
pub mod state;

pub use run::run;
