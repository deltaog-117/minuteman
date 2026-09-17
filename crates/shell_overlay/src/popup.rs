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

//! An embedded popup terminal: spawns `$SHELL` on its own pty sized to a small on-screen window
//! and parses its output into a virtual screen buffer (`vt100`), instead of `spawn_shell`'s
//! full-screen takeover. A background thread continuously feeds pty output into the parser; the
//! caller drives everything else from its own render loop — `with_screen` to read the current
//! buffer, `write_input` to forward keystrokes, `resize` to keep the pty in sync with however
//! large the popup is drawn, and `try_wait` to notice the shell exiting.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// How the child shell finished, decoupled from the pty backend's own status type so callers
/// never need to depend on `portable_pty` directly.
#[derive(Debug, Clone, Copy)]
pub struct ExitOutcome {
    pub success: bool,
    pub code: u32,
}

/// A shell running on its own pty, rendered as a small popup instead of taking over the whole
/// terminal. `rows`/`cols` at construction (and on every `resize`) size the pty — and thus what
/// the shell believes its terminal size is — so the caller must keep this in sync with however
/// large it actually draws the popup, or programs like `vim`/`less` inside it will misrender.
pub struct PopupShell {
    parser: Arc<Mutex<vt100::Parser>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl PopupShell {
    /// Spawns `$SHELL` (falling back to `/bin/sh` if unset) on a new pty of the given size, with
    /// `cwd` as its working directory. A background thread starts feeding the pty's output into
    /// the screen buffer immediately, so the shell can produce output before the caller ever
    /// calls `with_screen`.
    pub fn spawn(cwd: &Path, rows: u16, cols: u16) -> anyhow::Result<Self> {
        let (rows, cols) = (rows.max(1), cols.max(1));
        let pair = native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = CommandBuilder::new(shell);
        cmd.cwd(cwd);
        let child = pair.slave.spawn_command(cmd)?;
        // The parent has no further use for the slave end once the child owns it — dropping it
        // here (rather than holding it for the struct's lifetime) is what lets the pty report
        // EOF to the reader thread once the child actually exits.
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));

        let reader_parser = Arc::clone(&parser);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => lock_parser(&reader_parser).process(&buf[..n]),
                }
            }
        });

        Ok(Self {
            parser,
            writer,
            master: pair.master,
            child,
        })
    }

    /// Forwards raw bytes — already encoded as whatever escape sequences the terminal protocol
    /// needs — to the shell's stdin.
    pub fn write_input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)
    }

    /// Resizes both the pty and the screen buffer to match. Call whenever the popup's on-screen
    /// area changes (e.g. the terminal itself is resized).
    pub fn resize(&self, rows: u16, cols: u16) -> anyhow::Result<()> {
        let (rows, cols) = (rows.max(1), cols.max(1));
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        lock_parser(&self.parser).screen_mut().set_size(rows, cols);
        Ok(())
    }

    /// Runs `f` against the current screen buffer under the parser's lock. Callers build their
    /// rendering (cells, cursor position) from `f`'s return value rather than holding a
    /// reference into the screen past this call.
    pub fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> R {
        f(lock_parser(&self.parser).screen())
    }

    /// Non-blocking check for whether the shell has exited. `Ok(None)` means it's still running.
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitOutcome>> {
        Ok(self.child.try_wait()?.map(|status| ExitOutcome {
            success: status.success(),
            code: status.exit_code(),
        }))
    }
}

/// Locks `parser`, recovering the guard even if the background reader thread panicked while
/// holding it — a single bad write from a malformed escape sequence shouldn't take the whole
/// popup (and the TUI hosting it) down.
fn lock_parser(parser: &Mutex<vt100::Parser>) -> MutexGuard<'_, vt100::Parser> {
    parser
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    /// Polls `f` until it returns `Some`, or panics once `timeout` elapses — the reader thread
    /// updates the screen buffer asynchronously, so tests can't assert on it immediately after
    /// writing input.
    fn wait_for<T>(timeout: Duration, mut f: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(Instant::now() < deadline, "timed out waiting for condition");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-popup-shell-test-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes an executable `sh` script at `dir/fake_shell.sh` and points `$SHELL` at it for the
    /// duration of the closure `f`. Serialized via `crate::SHELL_ENV_LOCK` since
    /// `std::env::set_var` affects the whole process and these tests run on multiple threads —
    /// including `lib.rs`'s own `$SHELL`-overriding test.
    fn with_fake_shell<T>(dir: &Path, script: &str, f: impl FnOnce() -> T) -> T {
        let _guard = crate::SHELL_ENV_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        let path = dir.join("fake_shell.sh");
        std::fs::write(&path, script).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();

        // SAFETY: serialized by `SHELL_ENV_LOCK` above.
        unsafe {
            std::env::set_var("SHELL", &path);
        }
        let result = f();
        unsafe {
            std::env::remove_var("SHELL");
        }
        result
    }

    #[test]
    fn reads_child_output_into_the_screen_buffer() {
        let dir = unique_temp_dir("output");
        let popup = with_fake_shell(&dir, "#!/bin/sh\necho hello from popup\nexit 0\n", || {
            PopupShell::spawn(&dir, 24, 80).unwrap()
        });

        wait_for(Duration::from_secs(3), || {
            popup.with_screen(|s| s.contents().contains("hello from popup").then_some(()))
        });

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn forwards_input_and_reports_exit_status() {
        let dir = unique_temp_dir("input");
        let mut popup = with_fake_shell(
            &dir,
            "#!/bin/sh\nread line\necho \"got:$line\"\nexit 5\n",
            || PopupShell::spawn(&dir, 24, 80).unwrap(),
        );

        popup.write_input(b"hello\n").unwrap();

        wait_for(Duration::from_secs(3), || {
            popup.with_screen(|s| s.contents().contains("got:hello").then_some(()))
        });

        let outcome = wait_for(Duration::from_secs(3), || popup.try_wait().unwrap());
        assert!(!outcome.success);
        assert_eq!(outcome.code, 5);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resize_updates_the_screen_buffer_dimensions() {
        let dir = unique_temp_dir("resize");
        let popup = with_fake_shell(&dir, "#!/bin/sh\nsleep 5\n", || {
            PopupShell::spawn(&dir, 24, 80).unwrap()
        });

        assert_eq!(popup.with_screen(|s| s.size()), (24, 80));
        popup.resize(10, 40).unwrap();
        assert_eq!(popup.with_screen(|s| s.size()), (10, 40));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
