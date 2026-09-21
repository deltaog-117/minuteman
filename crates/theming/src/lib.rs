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

//! Config loading, keybinding resolution, and appearance — keys from
//! `~/.config/minuteman/config.toml`, the whole look (colors, glyphs, text styles, font) from
//! `~/.config/minuteman/appearance.toml`, each with built-in defaults when absent or invalid.

pub mod appearance;
pub mod config;
pub mod keymap;
pub mod theme;
pub mod ui;

pub use appearance::{Font, Mods, Styles};
pub use config::{Config, OpenWith};
pub use keymap::{Action, KeyMap};
pub use theme::Theme;
pub use ui::{GlyphSet, Ui};
