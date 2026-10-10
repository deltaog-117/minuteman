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

//! The file chooser backend of the XDG desktop portal, answered by `mman`.
//!
//! A browser's "Save as" dialog asks the portal (`xdg-desktop-portal`) for a file chooser; the
//! portal forwards the call to the backend named in `portals.conf`. This crate is that backend:
//! it opens `mman --chooser-file` in a terminal window, waits for the user to pick, and replies
//! with the chosen `file://` URIs.

pub mod request;
pub mod service;
pub mod session;
pub mod terminal;
pub mod uri;

/// The well-known bus name the portal looks for, from the `DBusName` of `minuteman.portal`.
pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.minuteman";
/// Where every portal backend exports its interfaces.
pub const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
/// The window class and app id of the picker, for a window-manager rule that floats it.
pub const WINDOW_CLASS: &str = "mman-chooser";
