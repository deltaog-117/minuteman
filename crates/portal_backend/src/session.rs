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

//! Runs one pick: opens `mman` in a terminal, waits for the window to close, reads the answer.

use std::ffi::{OsStr, OsString};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{fs, io};

use shared::chooser::{Selection, SelectionError};
use tokio::process::Command;
use tokio::sync::Notify;

use crate::WINDOW_CLASS;
use crate::request::PickRequest;
use crate::terminal::Terminal;

/// How a pick ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Chosen(Selection),
    /// The user backed out, closed the window, or the caller withdrew the request.
    Cancelled,
}

#[derive(Debug, thiserror::Error)]
pub enum PickError {
    #[error("cannot make a private place for the answer: {0}")]
    AnswerDir(io::Error),
    #[error("cannot start {program}: {source}")]
    Spawn { program: String, source: io::Error },
    #[error("cannot read the answer: {0}")]
    Read(io::Error),
    #[error("the answer file is not an answer: {0}")]
    Answer(SelectionError),
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SetupError {
    #[error(
        "MINUTEMAN_PORTAL_TERMINAL names '{0}', which is not one of foot, alacritty, kitty, wezterm, xterm"
    )]
    UnknownTerminal(String),
    #[error(
        "no terminal found: install foot, alacritty, kitty, wezterm or xterm, or set \
         MINUTEMAN_PORTAL_TERMINAL_CMD to a command that runs its arguments in a window"
    )]
    NoTerminal,
}

/// Picks the terminal: a custom command line wins, then a named built-in, then the first one
/// found on `path_var`.
pub fn resolve_terminal(
    custom: Option<&OsStr>,
    name: Option<&str>,
    path_var: &OsStr,
) -> Result<Terminal, SetupError> {
    if let Some(command) = custom {
        let words: Vec<OsString> = command
            .to_string_lossy()
            .split_whitespace()
            .map(OsString::from)
            .collect();
        if !words.is_empty() {
            return Ok(Terminal::Custom(words));
        }
    }
    match name {
        Some(name) => Terminal::named(name).ok_or_else(|| SetupError::UnknownTerminal(name.into())),
        None => Terminal::detect(path_var).ok_or(SetupError::NoTerminal),
    }
}

/// What starts a pick.
#[derive(Debug)]
pub struct Launcher {
    terminal: Terminal,
    /// The `mman` to run; a bare name is looked up on `PATH` by the terminal.
    mman: OsString,
    /// Where the window starts when the caller named no folder.
    home: Option<PathBuf>,
}

impl Launcher {
    pub fn new(terminal: Terminal, mman: OsString, home: Option<PathBuf>) -> Self {
        Self {
            terminal,
            mman,
            home,
        }
    }

    /// From `MINUTEMAN_PORTAL_TERMINAL_CMD`, `MINUTEMAN_PORTAL_TERMINAL`, `MINUTEMAN_PORTAL_MMAN`,
    /// `HOME` and `PATH`.
    pub fn from_env() -> Result<Self, SetupError> {
        let terminal = resolve_terminal(
            std::env::var_os("MINUTEMAN_PORTAL_TERMINAL_CMD").as_deref(),
            std::env::var("MINUTEMAN_PORTAL_TERMINAL").ok().as_deref(),
            &std::env::var_os("PATH").unwrap_or_default(),
        )?;
        let mman = std::env::var_os("MINUTEMAN_PORTAL_MMAN").unwrap_or_else(|| "mman".into());
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Ok(Self::new(terminal, mman, home))
    }

    /// Runs `request` to its end. `withdrawn` ends it early (the portal's `Close`), killing the
    /// window.
    pub async fn pick(
        &self,
        request: &PickRequest,
        withdrawn: &Notify,
    ) -> Result<Outcome, PickError> {
        let dir = AnswerDir::create().map_err(PickError::AnswerDir)?;
        let answer = dir.path().join("answer");

        let mut command = vec![self.mman.clone()];
        command.extend(mman_args(request, &answer));
        let argv = self.terminal.argv(&request.title, WINDOW_CLASS, &command);

        let mut process = Command::new(&argv[0]);
        process
            .args(&argv[1..])
            .stdin(Stdio::null())
            .kill_on_drop(true);
        if let Some(home) = self.home.as_deref().filter(|home| home.is_dir()) {
            process.current_dir(home);
        }
        let mut child = process.spawn().map_err(|source| PickError::Spawn {
            program: argv[0].to_string_lossy().into_owned(),
            source,
        })?;

        tokio::select! {
            // Whatever the exit status: the answer file, not the terminal's status, says what
            // the user did. A closed window leaves no file, and reads as a cancel.
            _ = child.wait() => {}
            () = withdrawn.notified() => {
                let _ = child.kill().await;
                return Ok(Outcome::Cancelled);
            }
        }

        match fs::read(&answer) {
            Ok(bytes) => match Selection::decode(&bytes).map_err(PickError::Answer)? {
                Some(selection) => Ok(Outcome::Chosen(selection)),
                None => Ok(Outcome::Cancelled),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Outcome::Cancelled),
            Err(error) => Err(PickError::Read(error)),
        }
    }
}

/// The arguments `mman` is given for `request`, answer file included.
pub fn mman_args(request: &PickRequest, answer: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "--chooser-file".into(),
        answer.into(),
        "--pick".into(),
        request.kind.name().into(),
    ];
    if let Some(name) = &request.name {
        args.push("--pick-name".into());
        args.push(name.into());
    }
    if let Some(dir) = &request.start_dir {
        args.push(dir.into());
    }
    args
}

/// A directory only this user can enter, removed when dropped: the answer is read from a file
/// here, so no other user may be able to plant one.
struct AnswerDir(PathBuf);

impl AnswerDir {
    fn create() -> io::Result<Self> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute() && dir.is_dir())
            .unwrap_or_else(std::env::temp_dir);
        let path = base.join(format!(
            "mman-pick-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        // `create` fails if the name is taken, so a directory someone else made first is never
        // reused.
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for AnswerDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::chooser::PickKind;

    fn request(kind: PickKind, name: Option<&str>, dir: Option<&str>) -> PickRequest {
        PickRequest {
            kind,
            title: "t".into(),
            start_dir: dir.map(PathBuf::from),
            name: name.map(str::to_owned),
        }
    }

    fn strs(args: &[OsString]) -> Vec<&str> {
        args.iter().map(|a| a.to_str().expect("utf-8")).collect()
    }

    #[test]
    fn mman_is_told_the_kind_the_name_and_the_folder_last() {
        let args = mman_args(
            &request(PickKind::Save, Some("a b.pdf"), Some("/home/u")),
            Path::new("/run/x/answer"),
        );
        assert_eq!(
            strs(&args),
            [
                "--chooser-file",
                "/run/x/answer",
                "--pick",
                "save",
                "--pick-name",
                "a b.pdf",
                "/home/u"
            ]
        );
    }

    #[test]
    fn nothing_optional_is_passed_when_nothing_was_asked() {
        let args = mman_args(&request(PickKind::Files, None, None), Path::new("/a"));
        assert_eq!(strs(&args), ["--chooser-file", "/a", "--pick", "files"]);
    }

    #[test]
    fn a_custom_command_wins_over_a_name_which_wins_over_detection() {
        let path = OsStr::new("/nonexistent");
        assert_eq!(
            resolve_terminal(Some(OsStr::new("env  A=1")), Some("kitty"), path),
            Ok(Terminal::Custom(vec!["env".into(), "A=1".into()]))
        );
        assert_eq!(
            resolve_terminal(None, Some("kitty"), path),
            Ok(Terminal::Kitty)
        );
        assert_eq!(
            resolve_terminal(Some(OsStr::new("  ")), Some("kitty"), path),
            Ok(Terminal::Kitty)
        );
    }

    #[test]
    fn an_unknown_name_and_an_empty_path_are_setup_errors() {
        let path = OsStr::new("/nonexistent");
        assert_eq!(
            resolve_terminal(None, Some("st"), path),
            Err(SetupError::UnknownTerminal("st".into()))
        );
        assert_eq!(
            resolve_terminal(None, None, path),
            Err(SetupError::NoTerminal)
        );
    }

    #[test]
    fn the_answer_directory_is_private_and_removed_on_drop() {
        use std::os::unix::fs::PermissionsExt;
        let dir = AnswerDir::create().expect("temp dir is writable");
        let path = dir.path().to_path_buf();
        let mode = fs::metadata(&path).expect("exists").permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        drop(dir);
        assert!(!path.exists());
    }
}
