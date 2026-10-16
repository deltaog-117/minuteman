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

//! What `ssh` asks while it connects (a password, a key's passphrase, whether to trust a host's
//! key) reaches the user through the host. This is the plugin's side of that: the question to ask
//! with, and the place the connection finds it.
//!
//! The question is asked with a blocking call because it is `ssh`'s own helper that waits for the
//! answer, on a thread of its own; nothing here runs on the render thread or the connection's task.

use std::fmt;
use std::sync::{Arc, OnceLock};

use plugin_api::{AskAnswer, AskSpec};

/// Whatever can put a question to the user and bring back the answer.
pub trait Ask: Send + Sync + fmt::Debug {
    /// Asks and waits.
    ///
    /// # Errors
    ///
    /// Why there is no answer: the user declined, or the host went away.
    fn ask(&self, spec: AskSpec) -> Result<AskAnswer, String>;
}

/// The question a [`ConnectOptions`](crate::ConnectOptions) connects with. It starts empty and is
/// filled once the plugin knows how it reaches the host, which is after its server is built; until
/// then, and in a process with no host to ask, a connection that needs an answer fails instead of
/// waiting for one.
#[derive(Clone, Default)]
pub struct Asker(Arc<OnceLock<Arc<dyn Ask>>>);

impl Asker {
    /// Sets how questions are asked. The first call wins; a later one has no effect.
    pub fn install(&self, ask: Arc<dyn Ask>) {
        let _ = self.0.set(ask);
    }

    pub(crate) fn get(&self) -> Option<Arc<dyn Ask>> {
        self.0.get().cloned()
    }
}

impl fmt::Debug for Asker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.get().is_some() {
            "Asker(installed)"
        } else {
            "Asker(empty)"
        })
    }
}
