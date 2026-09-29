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

//! One XDG `.desktop` file, read down to what opening a file needs: a name to show, the command
//! to run, and whether it wants a terminal. The `Exec` line is the fiddly part — quoting, escapes
//! and `%f`-style field codes — and is turned into the same `{}`-placeholder command line the
//! hand-written `[[open_with]]` entries use, so both launch through one path.

use std::path::{Path, PathBuf};

use crate::open::shell_quote;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    /// The desktop file ID (`org.kde.kate.desktop`), which `mimeapps.list` names apps by.
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub exec: String,
    /// `Terminal=true`: the program needs the terminal to itself.
    pub terminal: bool,
    /// `NoDisplay=true`: never listed in a menu, though still a valid handler.
    pub no_display: bool,
    pub try_exec: Option<String>,
}

/// Reads the `[Desktop Entry]` group of `text`. `None` when it is not a launchable application:
/// hidden, not `Type=Application`, or missing a `Name` or `Exec`. `locales` picks the `Name[..]`
/// translation, most specific first.
pub fn parse(id: &str, path: &Path, text: &str, locales: &[String]) -> Option<DesktopEntry> {
    let mut in_entry = false;
    let mut seen_entry = false;
    let mut name: Option<String> = None;
    let mut localized: Option<(usize, String)> = None;
    let (mut exec, mut kind, mut try_exec) = (None, None, None);
    let (mut terminal, mut no_display, mut hidden) = (false, false, false);

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            // Only the first group is the application; later ones are its actions.
            if in_entry {
                break;
            }
            in_entry = line == "[Desktop Entry]";
            seen_entry |= in_entry;
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "Name" => name = Some(unescape(value)),
            "Exec" => exec = Some(unescape(value)),
            "Type" => kind = Some(value.to_string()),
            "TryExec" => try_exec = Some(unescape(value)).filter(|v| !v.is_empty()),
            "Terminal" => terminal = value == "true",
            "NoDisplay" => no_display = value == "true",
            "Hidden" => hidden = value == "true",
            _ => {
                if let Some(locale) = key
                    .strip_prefix("Name[")
                    .and_then(|rest| rest.strip_suffix(']'))
                    && let Some(rank) = locales.iter().position(|l| l == locale)
                    && localized.as_ref().is_none_or(|(best, _)| rank < *best)
                {
                    localized = Some((rank, unescape(value)));
                }
            }
        }
    }

    if !seen_entry || hidden || kind.as_deref() != Some("Application") {
        return None;
    }
    let name = localized
        .map(|(_, n)| n)
        .or(name)
        .filter(|n| !n.is_empty())?;
    let exec = exec.filter(|e| !e.trim().is_empty())?;
    Some(DesktopEntry {
        id: id.to_string(),
        path: path.to_path_buf(),
        name,
        exec,
        terminal,
        no_display,
        try_exec,
    })
}

/// The file format's string escapes: `\s` a space, `\n`, `\t`, `\r`, `\\`. Anything else after a
/// backslash is kept as written.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Splits an `Exec` value into arguments: spaces separate them, double quotes group, and inside
/// quotes a backslash escapes `"`, `` ` ``, `$` and `\`. An unterminated quote runs to the end.
pub fn split_exec(exec: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (false, '"') => {
                quoted = true;
                started = true;
            }
            (true, '"') => quoted = false,
            (true, '\\')
                if chars
                    .peek()
                    .is_some_and(|n| matches!(n, '"' | '`' | '$' | '\\')) =>
            {
                word.extend(chars.next());
            }
            (false, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (_, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// A piece of one `Exec` argument after its field codes are read.
enum Piece {
    Text(String),
    /// Where the file goes (`%f`, `%F`, `%u`, `%U`).
    File,
}

/// Reads the field codes in one argument. `%c` is the app's name, `%k` its `.desktop` path and
/// `%%` a percent sign; `%i` and the deprecated codes take no part and vanish. Returns the pieces
/// and whether a code was seen, since an argument that was only a vanished code goes away
/// altogether while an empty quoted argument (`""`) stays.
fn pieces(word: &str, entry: &DesktopEntry) -> (Vec<Piece>, bool) {
    let mut out: Vec<Piece> = Vec::new();
    let mut text = String::new();
    let mut coded = false;
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            text.push(c);
            continue;
        }
        coded = true;
        match chars.next() {
            Some('%') | None => text.push('%'),
            Some('c') => text.push_str(&entry.name),
            Some('k') => text.push_str(&entry.path.to_string_lossy()),
            Some('f' | 'F' | 'u' | 'U') => {
                if !text.is_empty() {
                    out.push(Piece::Text(std::mem::take(&mut text)));
                }
                out.push(Piece::File);
            }
            Some(_) => {}
        }
    }
    if !text.is_empty() {
        out.push(Piece::Text(text));
    }
    (out, coded)
}

/// `text` as shell input that reads back as exactly `text` and never contains a `{}` (which the
/// launcher would take for the file's place). Plain words stay bare, so the program name is still
/// recognisable in the result.
fn quote_text(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if plain {
        return text.to_string();
    }
    // `{}` inside single quotes would still be replaced textually; closing and reopening the
    // quotes between the braces breaks it up without changing what the shell reads.
    shell_quote(text).replace("{}", "{''}")
}

/// The command line that runs `entry`, in the form `[[open_with]]` commands take: arguments quoted
/// for `sh -c` and the file's place marked `{}`. With no file code in `Exec` there is no marker,
/// and the launcher appends the file, since the point of choosing this app was to open it.
pub fn command_line(entry: &DesktopEntry) -> String {
    let mut parts: Vec<String> = Vec::new();
    for word in split_exec(&entry.exec) {
        let (pieces, coded) = pieces(&word, entry);
        if pieces.is_empty() && coded {
            continue;
        }
        if pieces.is_empty() {
            parts.push("''".into());
            continue;
        }
        parts.push(
            pieces
                .into_iter()
                .map(|piece| match piece {
                    Piece::Text(text) => quote_text(&text),
                    Piece::File => "{}".to_string(),
                })
                .collect(),
        );
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use proptest::prelude::*;

    use super::*;
    use crate::open;

    fn entry(exec: &str) -> DesktopEntry {
        DesktopEntry {
            id: "t.desktop".into(),
            path: "/usr/share/applications/t.desktop".into(),
            name: "The App".into(),
            exec: exec.into(),
            terminal: false,
            no_display: false,
            try_exec: None,
        }
    }

    fn parsed(text: &str, locales: &[&str]) -> Option<DesktopEntry> {
        let locales: Vec<String> = locales.iter().map(|l| (*l).to_string()).collect();
        parse("t.desktop", Path::new("/t.desktop"), text, &locales)
    }

    /// Runs `command` on `file` under a real `sh` the way the launcher does, and returns what it
    /// printed.
    fn run(command: &str, file: &str) -> String {
        let line = open::command_line(command, Path::new(file));
        let out = Command::new("sh").arg("-c").arg(&line).output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn a_plain_entry_reads_name_exec_and_flags() {
        let e = parsed(
            "[Desktop Entry]\nType=Application\nName=Kate\nExec=kate %F\nTerminal=false\n\
             NoDisplay=true\nTryExec=kate\n\n[Desktop Action new]\nName=New\nExec=other\n",
            &[],
        )
        .unwrap();
        assert_eq!((e.name.as_str(), e.exec.as_str()), ("Kate", "kate %F"));
        assert!(!e.terminal && e.no_display);
        assert_eq!(e.try_exec.as_deref(), Some("kate"));
    }

    #[test]
    fn hidden_wrong_type_and_incomplete_entries_are_not_apps() {
        for text in [
            "[Desktop Entry]\nType=Application\nName=A\nExec=a\nHidden=true\n",
            "[Desktop Entry]\nType=Link\nName=A\nURL=x\n",
            "[Desktop Entry]\nType=Application\nName=A\n",
            "[Desktop Entry]\nType=Application\nExec=a\n",
            "[Something Else]\nType=Application\nName=A\nExec=a\n",
            "",
        ] {
            assert_eq!(parsed(text, &[]), None, "{text:?}");
        }
    }

    #[test]
    fn the_best_translation_of_the_name_wins() {
        let text = "[Desktop Entry]\nType=Application\nName=Files\nName[pt]=Arquivos\n\
                    Name[pt_BR]=Ficheiros\nExec=f\n";
        assert_eq!(parsed(text, &["pt_BR", "pt"]).unwrap().name, "Ficheiros");
        assert_eq!(parsed(text, &["pt"]).unwrap().name, "Arquivos");
        assert_eq!(parsed(text, &["de"]).unwrap().name, "Files");
        assert_eq!(parsed(text, &[]).unwrap().name, "Files");
    }

    #[test]
    fn string_escapes_are_read_in_names_and_exec() {
        let e = parsed(
            "[Desktop Entry]\nType=Application\nName=A\\sB\nExec=a\\\\b\n",
            &[],
        )
        .unwrap();
        assert_eq!((e.name.as_str(), e.exec.as_str()), ("A B", "a\\b"));
    }

    #[test]
    fn exec_splits_on_spaces_and_groups_in_double_quotes() {
        assert_eq!(split_exec("a b  c"), ["a", "b", "c"]);
        assert_eq!(split_exec(r#"a "b c" d"#), ["a", "b c", "d"]);
        assert_eq!(
            split_exec(r#"a "say \"hi\" \$x \\ \q""#),
            ["a", r#"say "hi" $x \ \q"#]
        );
        assert_eq!(split_exec(r#"a "" b"#), ["a", "", "b"]);
        assert_eq!(split_exec(r#"a "unterminated b"#), ["a", "unterminated b"]);
        assert!(split_exec("   ").is_empty());
    }

    #[test]
    fn field_codes_become_the_file_marker_or_vanish() {
        assert_eq!(command_line(&entry("kate %F")), "kate {}");
        assert_eq!(
            command_line(&entry("app --new-window %U")),
            "app --new-window {}"
        );
        assert_eq!(command_line(&entry("app %i --flag %f")), "app --flag {}");
        assert_eq!(command_line(&entry("app --file=%f")), "app --file={}");
        assert_eq!(command_line(&entry("app %d %D %n %N %v %m x")), "app x");
        assert_eq!(command_line(&entry("app 100%% done")), "app 100% done");
        assert_eq!(command_line(&entry("app %c")), "app 'The App'");
        assert_eq!(
            command_line(&entry("app %k")),
            "app /usr/share/applications/t.desktop"
        );
        // No file code: nothing marks the file's place, and the launcher appends it.
        assert_eq!(command_line(&entry("vim")), "vim");
        assert_eq!(command_line(&entry(r#"app "" x"#)), "app '' x");
    }

    #[test]
    fn a_launched_command_receives_the_file_exactly() {
        for file in [
            "/tmp/plain.txt",
            "/tmp/a b.txt",
            "/tmp/it's.txt",
            "/tmp/$HOME `x`.txt",
            "/tmp/{}.txt",
        ] {
            let done = command_line(&entry("printf \"[%%s]\" %f"));
            assert_eq!(run(&done, file), format!("[{file}]"), "{done}");
            let embedded = command_line(&entry("printf \"[%%s]\" --file=%f"));
            assert_eq!(run(&embedded, file), format!("[--file={file}]"));
            let appended = command_line(&entry("printf \"[%%s]\""));
            assert_eq!(run(&appended, file), format!("[{file}]"));
        }
    }

    proptest! {
        /// Whatever the file is called, a launched `Exec` prints that name back and nothing
        /// else: quoting never lets a name reach the shell as syntax.
        #[test]
        fn any_file_name_survives_the_trip_through_exec_and_sh(
            name in "[^\\x00/\\n]{1,20}",
            template in prop::sample::select(vec![
                "printf \"[%%s]\" %f",
                "printf \"[%%s]\" %F",
                "printf \"[%%s]\" %u",
                "printf \"[%%s]\" %U",
                "printf \"[%%s]\" \"%f\"",
            ]),
        ) {
            let file = format!("/tmp/{name}");
            let command = command_line(&entry(template));
            prop_assert_eq!(run(&command, &file), format!("[{file}]"), "{}", command);
        }

        /// The app's own name, whatever it holds, reaches the program as one argument.
        #[test]
        fn a_name_with_odd_characters_stays_one_argument(name in "[^\\x00\\n]{1,20}") {
            let mut e = entry("printf \"[%%s]\" %c");
            e.name = name.clone();
            let command = command_line(&e);
            let out = Command::new("sh").arg("-c").arg(&command).output().unwrap();
            prop_assert_eq!(String::from_utf8_lossy(&out.stdout).into_owned(), format!("[{name}]"));
        }

        /// Splitting never panics on any text and never invents an argument out of nothing.
        #[test]
        fn splitting_any_exec_is_total(exec in ".{0,40}") {
            let words = split_exec(&exec);
            prop_assert!(words.len() <= exec.chars().count() + 1);
        }
    }
}
