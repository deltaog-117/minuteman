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

//! Parses the `:` prompt's text into a `Command`. The handful of commands that are cheap and
//! safe to run in-process (`cd`, `mkdir`, `touch`, `q`/`q!`/`qa`/`qa!`/`quit`, `trash`) are built
//! in; they go through `Vfs` and `file_ops`, so they behave the same on any backend and report
//! errors as plain messages (`trash` is the exception — see `file_ops::trash`'s own docs).
//! Everything else — and anything using shell syntax a built-in can't honour — is handed to
//! `sh -c` verbatim, so `:ls -l | wc -l` or `:git mv a b` just work.
//!
//! A command that needs the whole terminal — an editor, a pager, `ssh` — can't have its output
//! captured, so it is marked `Interactive` and run with the real terminal handed over. That is
//! decided either by its program name being in the configured list (`:nvim notes.md`) or by an
//! explicit `!` prefix (`:!python3`), so a program the list doesn't know is still one keystroke
//! away.

/// One `:` command, already split into what the app needs to run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Quit,
    Cd(String),
    Mkdir {
        parents: bool,
        names: Vec<String>,
    },
    Touch {
        names: Vec<String>,
    },
    /// Sends every marked entry (or the current selection, if none are marked) to the desktop
    /// trash — the same set `Action::Delete`'s `d` key acts on.
    Trash,
    /// A command line for `sh -c`, run in the browsed directory.
    Shell(String),
    /// A command line for `sh -c` that gets the real terminal — the program draws on it and reads
    /// the keyboard itself, and the browser waits until it exits.
    Interactive(String),
}

/// Characters that mean the user is writing shell, not naming a file: pipes, redirects,
/// substitution and globs. A built-in can't honour them, `sh` can.
const SHELL_SYNTAX: &[char] = &[
    '|', '&', ';', '<', '>', '$', '`', '*', '?', '[', ']', '{', '}', '(', ')',
];

/// Parses `buffer` (the text after the `:`). `Ok(None)` for an empty line. `interactive` names
/// the programs that get the real terminal (see the module docs).
///
/// # Errors
///
/// A message for the status line when a built-in is given something it can't use: an
/// unterminated quote or a missing operand.
pub fn parse(buffer: &str, interactive: &[String]) -> Result<Option<Command>, String> {
    let line = buffer.trim();
    if let Some(rest) = line.strip_prefix('!') {
        return match rest.trim() {
            "" => Err("!: missing command".into()),
            command => Ok(Some(Command::Interactive(command.into()))),
        };
    }
    let Some(name) = line.split_whitespace().next() else {
        return Ok(None);
    };

    match name {
        // "!"/"a" (vim's force/all) are accepted but not distinguished from a plain quit: there
        // is no unsaved-buffer or multi-window state here for them to mean something different
        // about, only the same one quit every spelling below performs.
        "q" | "q!" | "qa" | "qa!" | "quit" => Ok(Some(Command::Quit)),
        // Only bare "trash" is the built-in — "trash --empty" or similar falls through to a real
        // `trash` CLI on the shell, the same way an unrecognised `mkdir`/`touch` flag does.
        "trash" if line[name.len()..].trim().is_empty() => Ok(Some(Command::Trash)),
        "cd" => {
            let path = split_words(line[name.len()..].trim())?.join(" ");
            match path.is_empty() {
                true => Err("cd: missing path".into()),
                false => Ok(Some(Command::Cd(path))),
            }
        }
        "mkdir" | "touch" if uses_shell_syntax(line) => Ok(Some(Command::Shell(line.into()))),
        "mkdir" => {
            let mut words = split_words(line[name.len()..].trim())?
                .into_iter()
                .peekable();
            let mut parents = false;
            while let Some(flag) = words.next_if(|w| w.starts_with('-') && w.len() > 1) {
                match flag.as_str() {
                    "-p" | "--parents" => parents = true,
                    // A flag the built-in doesn't know (`-m 700`, `-v`) is `mkdir`'s to interpret.
                    _ => return Ok(Some(Command::Shell(line.into()))),
                }
            }
            operands("mkdir", words.collect(), |names| Command::Mkdir {
                parents,
                names,
            })
        }
        "touch" => {
            let words = split_words(line[name.len()..].trim())?;
            if words.iter().any(|w| w.starts_with('-') && w.len() > 1) {
                return Ok(Some(Command::Shell(line.into())));
            }
            operands("touch", words, |names| Command::Touch { names })
        }
        _ if names_interactive_program(name, interactive) => {
            Ok(Some(Command::Interactive(line.into())))
        }
        _ => Ok(Some(Command::Shell(line.into()))),
    }
}

/// Whether `program` — the first word of a command line — is one of `interactive`, by file name,
/// so `/usr/bin/nvim` counts the same as `nvim`.
pub fn names_interactive_program(program: &str, interactive: &[String]) -> bool {
    let name = std::path::Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);
    interactive.iter().any(|known| known == name)
}

fn operands(
    name: &str,
    names: Vec<String>,
    build: impl FnOnce(Vec<String>) -> Command,
) -> Result<Option<Command>, String> {
    match names.is_empty() {
        true => Err(format!("{name}: missing name")),
        false => Ok(Some(build(names))),
    }
}

/// Whether `line` contains shell syntax, or a `~` at the start of a word (home expansion).
fn uses_shell_syntax(line: &str) -> bool {
    line.contains(SHELL_SYNTAX) || line.split_whitespace().any(|w| w.starts_with('~'))
}

/// Splits `text` into words the way a shell does for plain arguments: whitespace separates,
/// `'single'` quotes are literal, `"double"` quotes honour `\"` and `\\`, and a backslash
/// outside quotes escapes the next character. An empty quoted string (`''`) is an empty word.
///
/// # Errors
///
/// An unterminated quote, or a trailing backslash with nothing to escape.
pub fn split_words(text: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    // Distinguishes "no word yet" from "an empty word from ''", which must still be emitted.
    let mut in_word = false;
    let mut chars = text.chars();

    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(inner) => word.push(inner),
                        None => return Err("unterminated ' quote".into()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(escaped @ ('"' | '\\')) => word.push(escaped),
                            Some(other) => {
                                word.push('\\');
                                word.push(other);
                            }
                            None => return Err("unterminated \" quote".into()),
                        },
                        Some(inner) => word.push(inner),
                        None => return Err("unterminated \" quote".into()),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.push(chars.next().ok_or("trailing backslash")?);
            }
            other => {
                in_word = true;
                word.push(other);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn shell(line: &str) -> Option<Command> {
        Some(Command::Shell(line.into()))
    }

    fn interactive(line: &str) -> Option<Command> {
        Some(Command::Interactive(line.into()))
    }

    /// Most tests here are about the built-ins and plain shell lines, which don't depend on
    /// which programs count as interactive.
    fn parse(line: &str) -> Result<Option<Command>, String> {
        super::parse(line, &[])
    }

    fn known() -> Vec<String> {
        vec!["nvim".into(), "less".into()]
    }

    #[test]
    fn an_empty_line_is_no_command() {
        assert_eq!(parse(""), Ok(None));
        assert_eq!(parse("   "), Ok(None));
    }

    #[test]
    fn quit_and_cd_are_built_in() {
        for spelling in ["q", "q!", "qa", "qa!", "quit"] {
            assert_eq!(parse(spelling), Ok(Some(Command::Quit)), "{spelling}");
        }
        assert_eq!(parse("cd /tmp"), Ok(Some(Command::Cd("/tmp".into()))));
        assert_eq!(parse("cd my dir"), Ok(Some(Command::Cd("my dir".into()))));
        assert_eq!(
            parse("cd 'my  dir'"),
            Ok(Some(Command::Cd("my  dir".into())))
        );
        assert_eq!(parse("cd"), Err("cd: missing path".into()));
    }

    #[test]
    fn mkdir_takes_the_parents_flag_and_any_number_of_names() {
        assert_eq!(
            parse("mkdir a b"),
            Ok(Some(Command::Mkdir {
                parents: false,
                names: vec!["a".into(), "b".into()]
            }))
        );
        assert_eq!(
            parse("mkdir -p x/y/z"),
            Ok(Some(Command::Mkdir {
                parents: true,
                names: vec!["x/y/z".into()]
            }))
        );
        assert_eq!(
            parse("mkdir 'two words'"),
            Ok(Some(Command::Mkdir {
                parents: false,
                names: vec!["two words".into()]
            }))
        );
        assert_eq!(parse("mkdir"), Err("mkdir: missing name".into()));
        assert_eq!(parse("mkdir -p"), Err("mkdir: missing name".into()));
    }

    #[test]
    fn bare_trash_is_built_in_but_anything_after_it_falls_to_the_shell() {
        assert_eq!(parse("trash"), Ok(Some(Command::Trash)));
        assert_eq!(parse("  trash  "), Ok(Some(Command::Trash)));
        assert_eq!(parse("trash --empty"), Ok(shell("trash --empty")));
        assert_eq!(parse("trash a.txt"), Ok(shell("trash a.txt")));
    }

    #[test]
    fn touch_takes_any_number_of_names() {
        assert_eq!(
            parse("touch a.txt b.txt"),
            Ok(Some(Command::Touch {
                names: vec!["a.txt".into(), "b.txt".into()]
            }))
        );
        assert_eq!(parse("touch"), Err("touch: missing name".into()));
    }

    #[test]
    fn unknown_commands_go_to_the_shell_verbatim() {
        assert_eq!(parse("ls -l"), Ok(shell("ls -l")));
        assert_eq!(parse("  git mv a b "), Ok(shell("git mv a b")));
        assert_eq!(parse("rm -rf build"), Ok(shell("rm -rf build")));
    }

    #[test]
    fn a_listed_program_gets_the_real_terminal_with_its_arguments() {
        assert_eq!(
            super::parse("nvim ROADMAP.md", &known()),
            Ok(interactive("nvim ROADMAP.md"))
        );
        assert_eq!(
            super::parse("  less  -N log.txt ", &known()),
            Ok(interactive("less  -N log.txt"))
        );
        assert_eq!(super::parse("nvim", &known()), Ok(interactive("nvim")));
    }

    #[test]
    fn a_listed_program_is_recognised_by_file_name() {
        assert_eq!(
            super::parse("/usr/bin/nvim x", &known()),
            Ok(interactive("/usr/bin/nvim x"))
        );
    }

    #[test]
    fn only_the_program_name_decides_not_its_arguments() {
        assert_eq!(super::parse("ls nvim", &known()), Ok(shell("ls nvim")));
        assert_eq!(super::parse("nvimfoo", &known()), Ok(shell("nvimfoo")));
        assert_eq!(super::parse("echo less", &known()), Ok(shell("echo less")));
    }

    #[test]
    fn a_bang_forces_the_real_terminal_for_any_command() {
        assert_eq!(parse("!python3"), Ok(interactive("python3")));
        assert_eq!(parse("! python3 -i"), Ok(interactive("python3 -i")));
        assert_eq!(
            parse("!cd /tmp && bash"),
            Ok(interactive("cd /tmp && bash"))
        );
        assert_eq!(parse("!"), Err("!: missing command".into()));
        assert_eq!(parse("!   "), Err("!: missing command".into()));
    }

    #[test]
    fn built_ins_keep_their_names_even_if_the_list_names_them() {
        let list = vec!["cd".to_string(), "touch".to_string()];
        assert_eq!(
            super::parse("cd /tmp", &list),
            Ok(Some(Command::Cd("/tmp".into())))
        );
        assert_eq!(
            super::parse("touch a", &list),
            Ok(Some(Command::Touch {
                names: vec!["a".into()]
            }))
        );
    }

    #[test]
    fn a_built_in_defers_to_the_shell_when_it_cannot_honour_the_syntax() {
        for line in [
            "mkdir -m 700 secret",
            "mkdir -v a",
            "touch -d yesterday a",
            "touch *.txt",
            "mkdir a{1,2}",
            "touch a && touch b",
            "mkdir ~/x",
            "touch $HOME/x",
        ] {
            assert_eq!(parse(line), Ok(shell(line)), "{line}");
        }
    }

    #[test]
    fn an_unterminated_quote_is_reported_for_a_built_in_only() {
        assert_eq!(parse("mkdir 'oops"), Err("unterminated ' quote".into()));
        // `sh` reports its own syntax error for a command the app doesn't interpret.
        assert_eq!(parse("echo 'oops"), Ok(shell("echo 'oops")));
    }

    #[test]
    fn split_words_follows_shell_quoting() {
        assert_eq!(
            split_words("a  b\tc"),
            Ok(vec!["a".into(), "b".into(), "c".into()])
        );
        assert_eq!(
            split_words("'a b' \"c d\""),
            Ok(vec!["a b".into(), "c d".into()])
        );
        assert_eq!(
            split_words(r#""say \"hi\"""#),
            Ok(vec![r#"say "hi""#.into()])
        );
        assert_eq!(split_words(r"a\ b"), Ok(vec!["a b".into()]));
        assert_eq!(split_words("a'b c'd"), Ok(vec!["ab cd".into()]));
        assert_eq!(split_words("'' x"), Ok(vec![String::new(), "x".into()]));
        assert_eq!(split_words(""), Ok(vec![]));
        assert!(split_words(r"trailing\").is_err());
        assert!(split_words("\"open").is_err());
    }

    /// Quotes a word so `split_words` reads it back unchanged: everything inside single quotes,
    /// with each `'` written as close-quote, escaped quote, reopen-quote.
    fn quote(word: &str) -> String {
        format!("'{}'", word.replace('\'', r"'\''"))
    }

    proptest! {
        #[test]
        fn quoting_then_splitting_round_trips_any_words(
            words in proptest::collection::vec(any::<String>(), 0..6)
        ) {
            let line = words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ");
            prop_assert_eq!(split_words(&line), Ok(words));
        }

        #[test]
        fn a_bang_line_is_interactive_with_its_trimmed_rest_or_an_error(rest in any::<String>()) {
            match super::parse(&format!("!{rest}"), &[]) {
                Ok(Some(Command::Interactive(text))) => prop_assert_eq!(text, rest.trim()),
                Err(_) => prop_assert!(rest.trim().is_empty()),
                other => prop_assert!(false, "unexpected {other:?}"),
            }
        }

        #[test]
        fn parse_never_panics_and_a_shell_command_keeps_its_trimmed_text(line in any::<String>()) {
            if let Ok(Some(Command::Shell(text))) = parse(&line) {
                prop_assert_eq!(text, line.trim());
            }
        }
    }
}
