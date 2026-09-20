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

//! Drops the user into a real, fully interactive `$SHELL` — not Ranger's auto-close-after-one-
//! command behavior. The shell inherits the caller's stdio directly (no pty multiplexing): the
//! caller is already running in a real terminal, so once raw mode/the alternate screen are
//! suspended, handing the same stdio straight to the child is simplest and behaves exactly like
//! a normal shell. The caller owns suspending/resuming those terminal modes around this call —
//! this crate only knows how to spawn and wait.

use std::path::Path;
use std::process::{Command, ExitStatus};

mod command;
mod popup;
pub use command::{CommandOutcome, CommandOutput, run_command};
pub use popup::{ExitOutcome, PopupShell};

/// Serializes tests (here and in `popup`) that temporarily override the process-wide `$SHELL`
/// env var — `std::env::set_var` isn't scoped to a thread, so two such tests running
/// concurrently would stomp on each other's shell.
#[cfg(test)]
static SHELL_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Spawns `$SHELL` (falling back to `/bin/sh` if unset) with `cwd` as its working directory,
/// and blocks until the user exits it.
pub fn spawn_shell(cwd: &Path) -> std::io::Result<ExitStatus> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    Command::new(shell).current_dir(cwd).status()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn spawns_the_configured_shell_in_the_given_directory() {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-shell-overlay-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // A fake "shell" that proves both cwd propagation (writes a marker via a relative path)
        // and exit-status propagation (a distinctive, non-zero code), without needing to
        // capture stdout — spawn_shell inherits it by design.
        let script = dir.join("fake_shell.sh");
        std::fs::write(&script, "#!/bin/sh\ntouch marker\nexit 7\n").unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        let _guard = SHELL_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: serialized by `SHELL_ENV_LOCK` above.
        unsafe {
            std::env::set_var("SHELL", &script);
        }
        let status = spawn_shell(&dir).unwrap();
        unsafe {
            std::env::remove_var("SHELL");
        }

        assert_eq!(status.code(), Some(7));
        assert!(dir.join("marker").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
