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

//! Which terminal the picker opens in, and how to ask it to run a command.
//!
//! `mman` is a terminal program, so a pick needs a window to draw in. The terminal must stay in
//! the foreground until the command ends (a terminal that hands the job to a server process and
//! exits at once cannot be waited for), which is true of the ones listed here.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// A terminal known to run a command in a new window and wait for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminal {
    Foot,
    Alacritty,
    Kitty,
    Wezterm,
    Xterm,
    /// The user's own: this command line, with `mman` and its arguments appended.
    Custom(Vec<OsString>),
}

/// Tried in this order when none is asked for.
const PREFERRED: [Terminal; 5] = [
    Terminal::Foot,
    Terminal::Alacritty,
    Terminal::Kitty,
    Terminal::Wezterm,
    Terminal::Xterm,
];

impl Terminal {
    pub fn program(&self) -> &OsStr {
        match self {
            Self::Foot => OsStr::new("foot"),
            Self::Alacritty => OsStr::new("alacritty"),
            Self::Kitty => OsStr::new("kitty"),
            Self::Wezterm => OsStr::new("wezterm"),
            Self::Xterm => OsStr::new("xterm"),
            Self::Custom(words) => words.first().map_or(OsStr::new(""), OsString::as_os_str),
        }
    }

    /// A built-in terminal by its program name.
    pub fn named(name: &str) -> Option<Self> {
        PREFERRED
            .into_iter()
            .find(|terminal| terminal.program() == OsStr::new(name))
    }

    /// The first built-in terminal found on `path_var` (a `PATH`-style list).
    pub fn detect(path_var: &OsStr) -> Option<Self> {
        PREFERRED
            .into_iter()
            .find(|terminal| find_in_path(terminal.program(), path_var).is_some())
    }

    /// The full command line that opens a window titled `title`, with window class `class`,
    /// running `command` (a program followed by its arguments).
    pub fn argv(&self, title: &str, class: &str, command: &[OsString]) -> Vec<OsString> {
        let mut argv: Vec<OsString> = Vec::new();
        let mut push = |word: &str| argv.push(OsString::from(word));
        match self {
            Self::Foot => {
                push("foot");
                push("--app-id");
                push(class);
                push("--title");
                push(title);
                push("--");
            }
            Self::Alacritty => {
                push("alacritty");
                push("--class");
                push(class);
                push("--title");
                push(title);
                push("-e");
            }
            Self::Kitty => {
                push("kitty");
                push("--class");
                push(class);
                push("--title");
                push(title);
            }
            Self::Wezterm => {
                push("wezterm");
                push("start");
                push("--class");
                push(class);
                push("--");
            }
            Self::Xterm => {
                push("xterm");
                push("-class");
                push(class);
                push("-T");
                push(title);
                push("-e");
            }
            Self::Custom(words) => argv.extend(words.iter().cloned()),
        }
        argv.extend(command.iter().cloned());
        argv
    }
}

fn find_in_path(program: &OsStr, path_var: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path_var)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(argv: &[OsString]) -> Vec<&str> {
        argv.iter()
            .map(|word| word.to_str().expect("ascii"))
            .collect()
    }

    #[test]
    fn every_builtin_ends_with_the_command_after_its_own_options() {
        let command = [
            OsString::from("mman"),
            OsString::from("--pick"),
            OsString::from("file"),
        ];
        for terminal in PREFERRED {
            let argv = terminal.argv("Open", "mman-chooser", &command);
            let argv = words(&argv);
            assert_eq!(
                &argv[argv.len() - 3..],
                ["mman", "--pick", "file"],
                "{terminal:?}"
            );
            assert!(
                argv.contains(&"mman-chooser"),
                "{terminal:?} sets the class"
            );
        }
    }

    #[test]
    fn a_title_with_option_like_text_stays_one_word() {
        let argv = Terminal::Alacritty.argv("--evil -e rm", "c", &[OsString::from("mman")]);
        assert!(words(&argv).contains(&"--evil -e rm"));
    }

    #[test]
    fn a_custom_terminal_is_its_words_then_the_command() {
        let terminal = Terminal::Custom(vec!["env".into(), "A=1".into()]);
        assert_eq!(
            words(&terminal.argv("t", "c", &[OsString::from("mman")])),
            ["env", "A=1", "mman"]
        );
    }

    #[test]
    fn names_resolve_to_builtins_only() {
        assert_eq!(Terminal::named("kitty"), Some(Terminal::Kitty));
        assert_eq!(Terminal::named("gnome-terminal"), None);
    }

    #[test]
    fn detection_follows_the_preference_order_over_the_path() {
        let dir = std::env::temp_dir().join(format!("mman-portal-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir is writable");
        for name in ["kitty", "xterm"] {
            let file = dir.join(name);
            std::fs::write(&file, b"#!/bin/sh\n").expect("temp dir is writable");
            std::fs::set_permissions(&file, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .expect("temp dir is writable");
        }
        let detected = Terminal::detect(dir.as_os_str());
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(detected, Some(Terminal::Kitty));
        assert_eq!(Terminal::detect(OsStr::new("/nonexistent")), None);
    }
}
