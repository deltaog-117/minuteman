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

//! Packing and unpacking archives, as a plugin. The host names the work (the `archive` event);
//! the plugin does it on the local disk with the code in `archive`, tells the host which entry
//! it has reached and how the whole request ended, and stops when told to. Nothing here touches the
//! terminal, and the forms, prompts and trash stay in the interface.
//!
//! `work` runs one request's jobs, `session` starts each request on its own thread and keeps the
//! flags that stop them, and `run` only moves lines between the host's pipes and the session.

pub mod archive;
pub mod error;
pub mod run;
pub mod session;
pub mod work;

pub use run::run;
