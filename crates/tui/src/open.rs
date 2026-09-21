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

//! What "open" and "open with" turn into: a command line for `sh -c`. Pure text-building and a
//! `PATH` lookup, so the quoting — the part that must never be wrong, since it decides what a
//! file *name* can make the shell do — can be tested (and fuzzed) against a real `sh`.

use std::path::Path;

use theming::OpenWith;

/// The program that opens a file with whatever the desktop has registered for its type.
pub const DEFAULT_OPENER: &str = if cfg!(target_os = "macos") {
    "open"
} else {
    "xdg-open"
};

/// `text` as one shell word. Single quotes make everything literal, so the only character that
/// needs care is a single quote itself: close the quote, add an escaped one, reopen.
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The command line that runs `command` on `path`. A `{}` in `command` is replaced by the quoted
/// path (every one of them); without one the path goes on the end.
pub fn command_line(command: &str, path: &Path) -> String {
    let quoted = shell_quote(&path.to_string_lossy());
    if command.contains("{}") {
        command.replace("{}", &quoted)
    } else {
        format!("{command} {quoted}")
    }
}

/// The program a command line starts with: its first whitespace-separated word.
pub fn program_of(command: &str) -> Option<&str> {
    command.split_whitespace().next()
}

/// Whether `program` can be run: a path is checked as it stands, a bare name is looked for in
/// each `PATH` directory. Keeps a mistyped `open_with` command from failing silently — a
/// detached program has nowhere to print its "not found".
pub fn program_exists(program: &str) -> bool {
    let is_runnable = |path: &Path| {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return is_runnable(Path::new(program));
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| is_runnable(&dir.join(program)))
    })
}

/// The "Open with" entries to offer: the configured ones, or — when none are configured — the
/// user's `$VISUAL` / `$EDITOR`, so the submenu is never an empty promise.
pub fn open_with_entries(configured: &[OpenWith]) -> Vec<OpenWith> {
    if !configured.is_empty() {
        return configured.to_vec();
    }
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|editor| !editor.trim().is_empty())
        .map(|editor| {
            let name = program_of(&editor)
                .and_then(|program| Path::new(program).file_name())
                .map_or_else(
                    || editor.clone(),
                    |name| name.to_string_lossy().into_owned(),
                );
            vec![OpenWith {
                name,
                command: editor,
            }]
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;

    use proptest::prelude::*;

    use super::*;

    #[test]
    fn a_plain_name_is_wrapped_and_a_quote_is_escaped() {
        assert_eq!(shell_quote("notes.md"), "'notes.md'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn the_path_is_appended_or_put_where_the_braces_are() {
        let path = PathBuf::from("/tmp/a b.txt");
        assert_eq!(command_line("nvim", &path), "nvim '/tmp/a b.txt'");
        assert_eq!(
            command_line("vlc --play {} --loop", &path),
            "vlc --play '/tmp/a b.txt' --loop"
        );
        assert_eq!(
            command_line("cp {} {}.bak", &path),
            "cp '/tmp/a b.txt' '/tmp/a b.txt'.bak"
        );
    }

    #[test]
    fn the_program_is_the_first_word() {
        assert_eq!(program_of("  nvim -R {}"), Some("nvim"));
        assert_eq!(program_of("   "), None);
    }

    #[test]
    fn programs_are_found_on_the_path_or_by_path_and_a_made_up_one_is_not() {
        assert!(program_exists("sh"));
        assert!(program_exists("/bin/sh"));
        assert!(!program_exists("minuteman-no-such-program"));
        assert!(!program_exists("/no/such/dir/sh"));
    }

    #[test]
    fn configured_entries_win_over_the_editor_fallback() {
        let configured = vec![OpenWith {
            name: "Neovim".into(),
            command: "nvim".into(),
        }];
        assert_eq!(open_with_entries(&configured), configured);
    }

    proptest! {
        // Case count kept low: every case starts a real `sh`.
        #![proptest_config(ProptestConfig::with_cases(48))]

        /// Whatever a file is called, the shell sees it back as exactly one, unchanged word.
        #[test]
        fn a_quoted_name_survives_the_shell_untouched(name in "[^\\x00]{0,40}") {
            let output = Command::new("sh")
                .arg("-c")
                .arg(format!("printf %s {}", shell_quote(&name)))
                .output()
                .unwrap();
            prop_assert_eq!(String::from_utf8_lossy(&output.stdout), name);
        }
    }
}
