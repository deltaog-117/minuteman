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
//! keys use, so a plugin reaches real core operations, not a bolted-on extras API — and run the
//! interface's own actions by name with `run_action`. A plugin can also provide one of those
//! actions itself (the fuzzy finder is such a plugin), opening a list the interface draws and
//! filling it, with the tree below a folder walked for it (see `picker` and `walk`).

mod actions;
pub mod error;
pub mod host;
mod launch;
mod life;
mod mailbox;
mod picker;
pub mod probe;
pub mod protocol;
mod registry;
mod sandbox;
pub mod serve;
mod tally;
mod walk;
#[cfg(feature = "wasm")]
mod wasm;

pub use actions::ActionRequest;
pub use error::PluginError;
pub use host::{ActionDispatch, LogMessage, PluginManager, Restart, Triggers};
pub use life::{PluginInfo, PluginState};
pub use picker::{AskRequest, PickerRequest, UiRequest};
pub use protocol::{
    API_VERSION, CanvasInput, CanvasNote, DevicesCommand, DevicesNote, EventKind, JobOutcome,
    LogLevel, SetCanvas, SetDevices, SetInspect, SetPreview, SetView, Slot, StateSnapshot, Surface,
};
pub use serve::ServeLink;
