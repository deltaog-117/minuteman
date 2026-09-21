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

/// Runs `line` under `sh -c` with `cwd` as its working directory on the caller's own terminal
/// — stdin, stdout and stderr are inherited, so an editor, pager or `ssh` works exactly as it
/// would typed at a prompt — and blocks until it exits. Like `spawn_shell`, the caller owns
/// leaving and re-entering raw mode and the alternate screen around this call.
///
/// # Errors
///
/// Returns the I/O error if `sh` cannot be spawned or waited on.
pub fn run_foreground(cwd: &Path, line: &str) -> std::io::Result<ExitStatus> {
    let _sigint = SigintGuard::install();
    Command::new("sh")
        .arg("-c")
        .arg(line)
        .current_dir(cwd)
        .status()
}

/// Starts `line` under `sh -c` with `cwd` as its working directory and returns at once, leaving it
/// running on its own: no terminal input or output, and its own process group so a `Ctrl-C`
/// meant for Minuteman does not reach it. This is for a program that opens its own window (an
/// image viewer, a browser). A thread reaps it when it ends, so it never lingers as a zombie.
///
/// # Errors
///
/// Returns the I/O error if `sh` cannot be spawned.
pub fn spawn_detached(cwd: &Path, line: &str) -> std::io::Result<()> {
    let mut sh = Command::new("sh");
    sh.arg("-c")
        .arg(line)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut sh, 0);
    let mut child = sh.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Keeps `Ctrl-C` for the child while it owns the terminal. With raw mode off, the terminal turns
/// that key into `SIGINT` for the whole foreground process group — this process included — so
/// interrupting `:!ping host` would otherwise take Minuteman down with it. A handler that does
/// nothing, rather than `SIG_IGN`, is the point: a handler is reset to the default across `exec`,
/// so the child still dies to `Ctrl-C` as it should, whereas an ignored signal stays ignored.
#[cfg(unix)]
struct SigintGuard {
    previous: libc::sighandler_t,
}

#[cfg(unix)]
extern "C" fn ignore_sigint(_signal: libc::c_int) {}

#[cfg(unix)]
impl SigintGuard {
    fn install() -> Self {
        // SAFETY: `signal` is given a handler that does nothing, which is trivially
        // async-signal-safe, and the previous disposition is put back in `drop`.
        let previous = unsafe {
            libc::signal(
                libc::SIGINT,
                ignore_sigint as extern "C" fn(libc::c_int) as libc::sighandler_t,
            )
        };
        Self { previous }
    }
}

#[cfg(unix)]
impl Drop for SigintGuard {
    fn drop(&mut self) {
        // SAFETY: restores the disposition `install` replaced.
        unsafe {
            libc::signal(libc::SIGINT, self.previous);
        }
    }
}

#[cfg(not(unix))]
struct SigintGuard;

#[cfg(not(unix))]
impl SigintGuard {
    fn install() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-run-foreground-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_foreground_command_runs_in_the_given_directory_and_reports_its_status() {
        let dir = scratch("cwd");
        let status = run_foreground(&dir, "touch marker; exit 5").unwrap();
        assert_eq!(status.code(), Some(5));
        assert!(dir.join("marker").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn shell_syntax_in_a_foreground_command_is_honoured() {
        let dir = scratch("syntax");
        let status = run_foreground(&dir, "echo a > one.txt && echo b >> one.txt").unwrap();
        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(dir.join("one.txt")).unwrap(),
            "a\nb\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The child interrupts its own parent, as the terminal would on `Ctrl-C`. Surviving to
    /// read the exit status is the assertion.
    #[test]
    fn a_ctrl_c_meant_for_the_child_does_not_kill_the_caller() {
        let dir = scratch("sigint");
        let status = run_foreground(&dir, "kill -INT $PPID; sleep 0.1; exit 0").unwrap();
        assert!(status.success());
        std::fs::remove_dir_all(&dir).unwrap();
    }

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

    #[test]
    fn a_detached_command_runs_in_the_given_directory_without_being_waited_on() {
        let dir = scratch("detached");
        spawn_detached(&dir, "pwd > where.txt").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let written = loop {
            match std::fs::read_to_string(dir.join("where.txt")) {
                Ok(text) if !text.is_empty() => break text,
                _ => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "the command never ran"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        };
        assert_eq!(
            std::path::Path::new(written.trim()).canonicalize().unwrap(),
            dir.canonicalize().unwrap()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
