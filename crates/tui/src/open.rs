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

use theming::{OpenRule, OpenWith};

use crate::mime_type::pattern_covers;

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

/// The program a command line starts with, as the shell would read that first word: single
/// quotes, double quotes and backslashes are undone, so a quoted path with spaces in it (which a
/// desktop entry for a program under `/opt/My App/` produces) is found where it really is.
pub fn program_word(command: &str) -> Option<String> {
    let mut word = String::new();
    let mut started = false;
    let mut chars = command.trim_start().chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => break,
            '\'' => {
                started = true;
                word.extend(chars.by_ref().take_while(|&q| q != '\''));
            }
            '"' => {
                started = true;
                word.extend(chars.by_ref().take_while(|&q| q != '"'));
            }
            '\\' => {
                started = true;
                word.extend(chars.next());
            }
            c => {
                started = true;
                word.push(c);
            }
        }
    }
    (started && !word.is_empty()).then_some(word)
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

/// The label for a bare command: its program's file name (`nvim` for `/usr/bin/nvim -R`).
fn program_label(command: &str) -> String {
    program_of(command)
        .and_then(|program| Path::new(program).file_name())
        .map_or_else(
            || command.to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
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
            vec![OpenWith {
                name: program_label(&editor),
                command: editor,
                terminal: false,
            }]
        })
        .unwrap_or_default()
}

/// Whether `rule` covers a file called `name` whose type is `mime`. An entry with a `/` is a MIME
/// pattern (`image/*`); any other is an extension, which may have several parts (`tar.gz`) and
/// must leave the file a name of its own (`.md` alone is a hidden file, not a Markdown one).
pub fn rule_matches(rule: &OpenRule, name: &str, mime: &str) -> bool {
    let name = name.to_lowercase();
    let mime = mime.to_lowercase();
    rule.matches.iter().any(|entry| {
        let entry = entry.trim().to_lowercase();
        if entry.contains('/') {
            return pattern_covers(&entry, &mime);
        }
        let ext = entry.trim_start_matches('.');
        !ext.is_empty()
            && name.len() > ext.len() + 1
            && name.ends_with(ext)
            && name[..name.len() - ext.len()].ends_with('.')
    })
}

/// A rule as an "Open with" choice.
pub fn rule_choice(rule: &OpenRule) -> OpenWith {
    OpenWith {
        name: rule
            .name
            .clone()
            .unwrap_or_else(|| program_label(&rule.command)),
        command: rule.command.clone(),
        terminal: rule.terminal,
    }
}

/// The first `[[open_rule]]` that covers the file: what `enter` and a double-click use ahead of
/// anything the desktop has registered.
pub fn first_rule<'a>(rules: &'a [OpenRule], name: &str, mime: &str) -> Option<&'a OpenRule> {
    rules.iter().find(|rule| rule_matches(rule, name, mime))
}

/// Everything the "Open with" submenu lists for a file, best first: each rule that covers it, the
/// hand-written `[[open_with]]` entries, then the programs installed for its type. The same
/// command is only listed once, at its first (highest) place. With nothing configured and nothing
/// installed the user's `$VISUAL`/`$EDITOR` is offered, so the list is never an empty promise.
pub fn choices(
    rules: &[OpenRule],
    configured: &[OpenWith],
    discovered: Vec<OpenWith>,
    name: &str,
    mime: &str,
) -> Vec<OpenWith> {
    let mut all: Vec<OpenWith> = rules
        .iter()
        .filter(|rule| rule_matches(rule, name, mime))
        .map(rule_choice)
        .collect();
    all.extend(configured.iter().cloned());
    if configured.is_empty() && discovered.is_empty() {
        all.extend(open_with_entries(&[]));
    }
    all.extend(discovered);
    let mut seen = std::collections::HashSet::new();
    all.retain(|choice| seen.insert(choice.command.clone()));
    all
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;

    use proptest::prelude::*;

    use super::*;

    #[test]
    fn the_first_word_is_read_the_way_the_shell_reads_it() {
        assert_eq!(program_word("nvim -R {}").as_deref(), Some("nvim"));
        assert_eq!(program_word("  vlc").as_deref(), Some("vlc"));
        assert_eq!(
            program_word("'/opt/My App/bin/app' {}").as_deref(),
            Some("/opt/My App/bin/app")
        );
        assert_eq!(program_word("\"/a b/c\" x").as_deref(), Some("/a b/c"));
        assert_eq!(program_word(r"a\ b c").as_deref(), Some("a b"));
        assert_eq!(program_word("FOO=1 app").as_deref(), Some("FOO=1"));
        assert_eq!(program_word("''"), None);
        assert_eq!(program_word("   "), None);
        assert_eq!(program_word(""), None);
    }

    proptest! {
        /// Quoting a path and reading its first word back gives the path, whatever it holds.
        #[test]
        fn a_quoted_program_path_reads_back_as_itself(path in "[^\\x00\\n]{1,30}") {
            let command = format!("{} {{}}", shell_quote(&path));
            prop_assert_eq!(program_word(&command), Some(path));
        }
    }

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

    fn rule(matches: &[&str], command: &str) -> OpenRule {
        OpenRule {
            matches: matches.iter().map(|m| (*m).to_string()).collect(),
            command: command.into(),
            name: None,
            terminal: false,
        }
    }

    fn with(name: &str, command: &str) -> OpenWith {
        OpenWith {
            name: name.into(),
            command: command.into(),
            terminal: false,
        }
    }

    #[test]
    fn a_rule_matches_by_extension_or_by_mime_pattern() {
        let r = rule(&["md", ".TXT", "image/*", "text/x-python"], "x");
        for (name, mime, hit) in [
            ("notes.md", "text/markdown", true),
            ("NOTES.MD", "text/markdown", true),
            ("a.txt", "text/plain", true),
            ("photo.webp", "image/webp", true),
            ("script", "text/x-python", true),
            ("a.tar.md", "x/y", true),
            ("readme", "text/markdown", false),
            ("a.mdx", "text/markdown", false),
            ("amd", "x/y", false),
            (".md", "x/y", false),
            ("clip.mp4", "video/mp4", false),
            ("script", "text/x-python3", false),
        ] {
            assert_eq!(rule_matches(&r, name, mime), hit, "{name} {mime}");
        }
    }

    #[test]
    fn an_extension_with_several_parts_matches_the_whole_tail() {
        let r = rule(&["tar.gz"], "x");
        assert!(rule_matches(&r, "a.tar.gz", "application/gzip"));
        assert!(!rule_matches(&r, "a.gz", "application/gzip"));
        assert!(!rule_matches(&r, "atar.gz", "application/gzip"));
    }

    #[test]
    fn the_first_matching_rule_wins() {
        let rules = [rule(&["png"], "first"), rule(&["image/*"], "second")];
        assert_eq!(
            first_rule(&rules, "a.png", "image/png").unwrap().command,
            "first"
        );
        assert_eq!(
            first_rule(&rules, "a.gif", "image/gif").unwrap().command,
            "second"
        );
        assert!(first_rule(&rules, "a.txt", "text/plain").is_none());
    }

    #[test]
    fn choices_list_rules_then_configured_then_installed_without_repeats() {
        let mut named = rule(&["md"], "glow");
        named.name = Some("Glow".into());
        let rules = [named, rule(&["txt"], "never")];
        let out = choices(
            &rules,
            &[with("Neovim", "nvim"), with("Again", "glow")],
            vec![with("Kate", "kate {}"), with("Neovim (desktop)", "nvim")],
            "a.md",
            "text/markdown",
        );
        let names: Vec<&str> = out.iter().map(|c| c.name.as_str()).collect();
        // The rule's `glow` beats the later duplicate; the second `nvim` is dropped too.
        assert_eq!(names, ["Glow", "Neovim", "Kate"]);
    }

    #[test]
    fn a_rule_without_a_name_is_labelled_by_its_program() {
        assert_eq!(
            rule_choice(&rule(&["md"], "/usr/bin/nvim -R {}")).name,
            "nvim"
        );
    }

    #[test]
    fn the_editor_is_offered_only_when_nothing_else_is() {
        // Whatever `$VISUAL`/`$EDITOR` holds on this machine, the fallback is exactly it.
        let editor = open_with_entries(&[]);
        let none = choices(&[], &[], Vec::new(), "a", "text/plain");
        assert_eq!(none, editor);
        let with_discovered = choices(&[], &[], vec![with("Kate", "kate")], "a", "text/plain");
        assert_eq!(with_discovered, [with("Kate", "kate")]);
    }

    proptest! {
        /// A rule for an extension covers every name that ends in it, in any case, and no name
        /// that merely ends in the same letters.
        #[test]
        fn an_extension_rule_covers_exactly_the_names_ending_in_dot_ext(
            stem in "[a-z]{1,6}", ext in "[a-z]{1,4}", upper in any::<bool>(),
        ) {
            let r = rule(&[&ext], "x");
            let name = if upper {
                format!("{stem}.{ext}").to_uppercase()
            } else {
                format!("{stem}.{ext}")
            };
            prop_assert!(rule_matches(&r, &name, "x/y"));
            let glued = format!("{stem}{ext}");
            prop_assert!(!rule_matches(&r, &glued, "x/y"));
        }

        /// No list `choices` returns names one command twice.
        #[test]
        fn choices_never_repeat_a_command(
            commands in proptest::collection::vec("[ab]{1,2}", 0..8),
            split in 0usize..9,
        ) {
            let items: Vec<OpenWith> = commands.iter().map(|c| with(c, c)).collect();
            let split = split.min(items.len());
            let out = choices(&[], &items[..split], items[split..].to_vec(), "f", "x/y");
            let mut seen = std::collections::HashSet::new();
            prop_assert!(out.iter().all(|c| seen.insert(c.command.clone())));
        }
    }

    #[test]
    fn configured_entries_win_over_the_editor_fallback() {
        let configured = vec![OpenWith {
            name: "Neovim".into(),
            command: "nvim".into(),
            terminal: false,
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
