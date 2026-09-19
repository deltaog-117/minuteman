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

//! Config loading, keybinding resolution, and theme colors — read from
//! `~/.config/minuteman/config.toml`, with built-in defaults when absent or invalid.

pub mod config;
pub mod keymap;
pub mod theme;
pub mod ui;

pub use config::Config;
pub use keymap::{Action, KeyMap};
pub use theme::Theme;
pub use ui::{GlyphSet, Ui};
