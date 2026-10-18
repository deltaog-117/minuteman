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

//! The form that adds a plugin of the user's own from the plugin menu: one text row for each
//! field of a `[[plugin]]` table. Pure, like `compress_popup`: no `Frame`, no file and no
//! process, so `overlay_view` draws it and `app` is the only place that starts the plugin and
//! `theming` the only one that writes it down.
//!
//! The first row is a path to a plugin's folder (or its `plugin.toml`). `Enter` on it, or moving
//! off it, asks the caller to read the manifest ([`Key::Load`]) and hand the plugin back with
//! [`AddPluginForm::fill`], which writes the other rows so they can be edited before adding.
//!
//! The form checks what it can tell from the text alone (a name, a command, keys that name a
//! key, events that exist). What needs the rest of the program — a name or key or `:` word that
//! something else already has, a command that will not start — is checked by the caller, which
//! hands the reason back with [`AddPluginForm::set_error`] so the form stays open.

use std::path::{Path, PathBuf};

use crossterm::event::KeyCode;
use plugin_host::EventKind;
use theming::{DropIn, RawPluginSpec, parse_key};

/// The manifest's file name, so a path that names the file means its folder.
const MANIFEST_FILE: &str = theming::dropin::MANIFEST;

/// The longest plugin name the form accepts, in characters.
const MAX_NAME_CHARS: usize = 40;

/// One row of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Path,
    Name,
    Command,
    Args,
    Keys,
    Commands,
    Events,
    Provides,
}

impl Row {
    pub const ALL: [Row; 8] = [
        Row::Path,
        Row::Name,
        Row::Command,
        Row::Args,
        Row::Keys,
        Row::Commands,
        Row::Events,
        Row::Provides,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Row::Path => "Folder",
            Row::Name => "Name",
            Row::Command => "Command",
            Row::Args => "Arguments",
            Row::Keys => "Keys",
            Row::Commands => ": commands",
            Row::Events => "Events",
            Row::Provides => "Provides",
        }
    }

    /// What an empty row means, drawn dimmed until something is typed.
    pub fn hint(self) -> &'static str {
        match self {
            Row::Path => "optional, a plugin folder: its plugin.toml fills the rows",
            Row::Name => "required, letters, digits, - and _",
            Row::Command => "required, the program to run",
            Row::Args => "\"quoted\" if one holds a space",
            Row::Keys => "e.g. b space (a key already bound is refused)",
            Row::Commands => "e.g. blame",
            Row::Events => "dir_changed selection_changed job_finished",
            Row::Provides => "actions it answers in the interface's place",
        }
    }
}

/// What a keystroke asks the caller to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Stay,
    Close,
    /// The table the text describes; the caller may still refuse it.
    Submit(RawPluginSpec),
    /// The folder typed in the path row: the caller reads its manifest and answers with
    /// [`AddPluginForm::fill`] or [`AddPluginForm::set_error`].
    Load(String),
}

#[derive(Debug, Default)]
pub struct AddPluginForm {
    cursor: usize,
    values: [String; 8],
    error: Option<String>,
    /// The path whose manifest was last asked for, so moving off an unchanged row asks nothing.
    loaded: String,
    /// What the loaded manifest says about the plugin, shown under the rows.
    note: Option<String>,
}

impl AddPluginForm {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn value(&self, row: Row) -> &str {
        &self.values[row as usize]
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The loaded manifest's description, if it has one.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Writes the plugin a manifest describes into the rows below the path, replacing what they
    /// held, ready to be edited and added.
    ///
    /// # Errors
    ///
    /// The reason, when an argument holds a double quote: the arguments row has no way to write
    /// one, and filling it with the quote dropped would add a different plugin than the manifest's.
    pub fn fill(&mut self, found: &DropIn) -> Result<(), String> {
        let spec = &found.spec;
        let args = join_args(&spec.args)?;
        self.values[Row::Name as usize] = spec.name.clone();
        self.values[Row::Command as usize] = spec.command.clone();
        self.values[Row::Args as usize] = args;
        self.values[Row::Keys as usize] = spec.on_key.join(" ");
        self.values[Row::Commands as usize] = spec.commands.join(" ");
        self.values[Row::Events as usize] = spec.events.join(" ");
        self.values[Row::Provides as usize] = spec.provides.join(" ");
        self.note = Some(found.description.clone()).filter(|text| !text.is_empty());
        self.error = None;
        Ok(())
    }

    /// Shows why the form's plugin cannot be added; the next keystroke clears it.
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }

    /// Up, down, `Tab` and `BackTab` move focus, wrapping; every other character is typed into
    /// the focused row and `Backspace` deletes. `Enter` adds the plugin from any row, `Esc`
    /// closes the form.
    pub fn key(&mut self, code: KeyCode) -> Key {
        let rows = Row::ALL.len();
        match code {
            KeyCode::Esc => return Key::Close,
            KeyCode::Enter if self.cursor == Row::Path as usize && self.wants_load() => {
                return self.load();
            }
            KeyCode::Enter => match self.build() {
                Ok(spec) => return Key::Submit(spec),
                Err(message) => self.error = Some(message),
            },
            KeyCode::Down | KeyCode::Tab => return self.step(1, rows),
            KeyCode::Up | KeyCode::BackTab => return self.step(rows - 1, rows),
            KeyCode::Char(c) if !c.is_control() => {
                self.values[self.cursor].push(c);
                self.error = None;
            }
            KeyCode::Backspace => {
                self.values[self.cursor].pop();
                self.error = None;
            }
            _ => {}
        }
        Key::Stay
    }

    /// Moves focus; leaving the path row with a new path asks for its manifest.
    fn step(&mut self, by: usize, rows: usize) -> Key {
        let leaving_path = self.cursor == Row::Path as usize && self.wants_load();
        self.cursor = (self.cursor + by) % rows;
        self.error = None;
        if leaving_path {
            return self.load();
        }
        Key::Stay
    }

    /// Whether the path row holds a path that has not been read yet.
    fn wants_load(&self) -> bool {
        let path = self.value(Row::Path).trim();
        !path.is_empty() && path != self.loaded
    }

    fn load(&mut self) -> Key {
        let path = self.value(Row::Path).trim().to_string();
        self.loaded.clone_from(&path);
        Key::Load(path)
    }

    /// The `[[plugin]]` table the text describes, or why it is not one yet. Changes nothing.
    pub fn build(&self) -> Result<RawPluginSpec, String> {
        let name = self.value(Row::Name).trim();
        check_name(name)?;
        let command = self.value(Row::Command).trim();
        if command.is_empty() {
            return Err("the command is empty".into());
        }
        let on_key = words(self.value(Row::Keys));
        if let Some(bad) = on_key.iter().find(|word| parse_key(word).is_none()) {
            return Err(format!(
                "'{bad}' is not a key: use one character or a name like space"
            ));
        }
        let events = words(self.value(Row::Events));
        if let Some(bad) = events.iter().find(|word| EventKind::parse(word).is_none()) {
            return Err(format!("'{bad}' is not an event"));
        }
        Ok(RawPluginSpec {
            name: name.to_string(),
            command: command.to_string(),
            args: split_args(self.value(Row::Args))?,
            on_key,
            commands: words(self.value(Row::Commands)),
            events,
            provides: words(self.value(Row::Provides)),
        })
    }
}

/// The folder `text` names: `~` is `home`, a relative path starts at `cwd`, and the path of a
/// `plugin.toml` means the folder holding it.
pub fn resolve_dir(text: &str, home: Option<&Path>, cwd: &Path) -> PathBuf {
    let text = text.trim();
    let path = match (text.strip_prefix('~'), home) {
        (Some(""), Some(home)) => home.to_path_buf(),
        (Some(rest), Some(home)) if rest.starts_with('/') => home.join(&rest[1..]),
        _ => cwd.join(text),
    };
    if path.file_name().is_some_and(|name| name == MANIFEST_FILE) {
        path.parent().map(Path::to_path_buf).unwrap_or(path)
    } else {
        path
    }
}

/// `args` as the arguments row reads them back: an argument with a space (or none at all) in
/// double quotes, the rest as they are. An argument holding a `"` cannot be written, so it is
/// refused.
fn join_args(args: &[String]) -> Result<String, String> {
    if args.iter().any(|arg| arg.contains('"')) {
        return Err("an argument holds a double quote, which this form cannot show".into());
    }
    Ok(args
        .iter()
        .map(|arg| {
            if arg.is_empty() || arg.contains(char::is_whitespace) {
                format!("\"{arg}\"")
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" "))
}

/// Why `name` cannot name a plugin, or `Ok` if it can.
fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        Err("the name is empty".into())
    } else if name.chars().count() > MAX_NAME_CHARS {
        Err("the name is too long".into())
    } else if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Err("the name may hold only letters, digits, - and _".into())
    } else {
        Ok(())
    }
}

/// The words of `text`, split on spaces and commas, each once and in the order typed.
fn words(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || c == ',') {
        if !word.is_empty() && !found.iter().any(|seen| seen == word) {
            found.push(word.to_string());
        }
    }
    found
}

/// The arguments of `text`: split on spaces, except inside double quotes, which are dropped. An
/// empty pair of quotes is an empty argument.
fn split_args(text: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err("a quote in the arguments is never closed".into());
    }
    if started {
        args.push(current);
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn typed(form: &mut AddPluginForm, rows: &[(Row, &str)]) {
        for &(row, text) in rows {
            while form.cursor() != row as usize {
                form.key(KeyCode::Tab);
            }
            for c in text.chars() {
                form.key(KeyCode::Char(c));
            }
        }
    }

    #[test]
    fn a_name_and_a_command_are_enough() {
        let mut form = AddPluginForm::new();
        typed(
            &mut form,
            &[(Row::Name, "blame"), (Row::Command, "python3")],
        );
        let spec = form.build().unwrap();
        assert_eq!(spec.name, "blame");
        assert_eq!(spec.command, "python3");
        assert!(spec.args.is_empty() && spec.on_key.is_empty() && spec.provides.is_empty());
        assert!(matches!(form.key(KeyCode::Enter), Key::Submit(_)));
    }

    #[test]
    fn every_field_is_read_from_its_row() {
        let mut form = AddPluginForm::new();
        typed(
            &mut form,
            &[
                (Row::Name, "blame"),
                (Row::Command, "python3"),
                (Row::Args, "blame.py \"my file\" -v"),
                (Row::Keys, "b, space b"),
                (Row::Commands, "blame praise"),
                (Row::Events, "dir_changed"),
                (Row::Provides, "find"),
            ],
        );
        let spec = form.build().unwrap();
        assert_eq!(spec.args, ["blame.py", "my file", "-v"]);
        assert_eq!(spec.on_key, ["b", "space"]);
        assert_eq!(spec.commands, ["blame", "praise"]);
        assert_eq!(spec.events, ["dir_changed"]);
        assert_eq!(spec.provides, ["find"]);
    }

    #[test]
    fn what_is_wrong_is_said_and_keeps_the_form_open_until_the_next_key() {
        let cases: [(&[(Row, &str)], &str); 6] = [
            (&[], "name is empty"),
            (&[(Row::Name, "a b")], "may hold only"),
            (&[(Row::Name, "a")], "command is empty"),
            (
                &[(Row::Name, "a"), (Row::Command, "x"), (Row::Keys, "bb")],
                "'bb' is not a key",
            ),
            (
                &[(Row::Name, "a"), (Row::Command, "x"), (Row::Events, "nope")],
                "'nope' is not an event",
            ),
            (
                &[(Row::Name, "a"), (Row::Command, "x"), (Row::Args, "\"open")],
                "never closed",
            ),
        ];
        for (rows, said) in cases {
            let mut form = AddPluginForm::new();
            typed(&mut form, rows);
            assert_eq!(form.key(KeyCode::Enter), Key::Stay);
            assert!(form.error().unwrap().contains(said), "{:?}", form.error());
            form.key(KeyCode::Char('x'));
            assert!(form.error().is_none());
        }
    }

    fn found(args: &[&str]) -> DropIn {
        DropIn {
            dir: PathBuf::from("/p/blame"),
            spec: RawPluginSpec {
                name: "blame".into(),
                command: "/p/blame/run".into(),
                args: args.iter().map(|arg| (*arg).to_string()).collect(),
                on_key: vec!["b".into()],
                commands: vec!["blame".into()],
                events: vec!["dir_changed".into()],
                provides: vec!["find".into()],
            },
            description: "Shows who changed a line".into(),
            api_version: None,
        }
    }

    #[test]
    fn enter_on_a_new_path_asks_for_its_manifest_and_fill_writes_every_row() {
        let mut form = AddPluginForm::new();
        assert_eq!(form.cursor(), Row::Path as usize);
        typed(&mut form, &[(Row::Path, "~/p/blame")]);
        assert_eq!(form.key(KeyCode::Enter), Key::Load("~/p/blame".into()));
        // Asked once: the same path again is an add, not a second read.
        assert_eq!(form.key(KeyCode::Enter), Key::Stay);
        form.fill(&found(&["--x", "a b"])).unwrap();
        assert_eq!(form.note(), Some("Shows who changed a line"));
        let spec = form.build().unwrap();
        assert_eq!(spec, found(&["--x", "a b"]).spec);
    }

    #[test]
    fn leaving_the_path_row_with_a_new_path_loads_it_and_an_empty_one_loads_nothing() {
        let mut form = AddPluginForm::new();
        assert_eq!(form.key(KeyCode::Tab), Key::Stay);
        form.key(KeyCode::BackTab);
        typed(&mut form, &[(Row::Path, "/p")]);
        assert_eq!(form.key(KeyCode::Down), Key::Load("/p".into()));
        assert_eq!(form.cursor(), Row::Name as usize);
        form.key(KeyCode::Up);
        assert_eq!(form.key(KeyCode::Down), Key::Stay);
        form.key(KeyCode::Up);
        form.key(KeyCode::Char('x'));
        assert_eq!(form.key(KeyCode::Tab), Key::Load("/px".into()));
    }

    #[test]
    fn a_manifest_argument_with_a_quote_is_refused_and_leaves_the_rows_alone() {
        let mut form = AddPluginForm::new();
        typed(&mut form, &[(Row::Name, "mine")]);
        assert!(form.fill(&found(&["a\"b"])).unwrap_err().contains("quote"));
        assert_eq!(form.value(Row::Name), "mine");
    }

    #[test]
    fn a_path_resolves_against_home_and_the_working_directory() {
        let home = Path::new("/home/u");
        let cwd = Path::new("/w");
        for (text, want) in [
            ("~", "/home/u"),
            ("~/p/x", "/home/u/p/x"),
            ("p/x", "/w/p/x"),
            ("/abs/x/", "/abs/x/"),
            ("/abs/x/plugin.toml", "/abs/x"),
            (" ~/p ", "/home/u/p"),
        ] {
            assert_eq!(
                resolve_dir(text, Some(home), cwd),
                Path::new(want),
                "{text}"
            );
        }
        assert_eq!(resolve_dir("~/p", None, cwd), Path::new("/w/~/p"));
    }

    #[test]
    fn tab_and_the_arrows_wrap_and_backspace_edits_the_focused_row() {
        let mut form = AddPluginForm::new();
        form.key(KeyCode::Up);
        assert_eq!(form.cursor(), Row::ALL.len() - 1);
        form.key(KeyCode::Down);
        assert_eq!(form.cursor(), 0);
        typed(&mut form, &[(Row::Name, "ab")]);
        form.key(KeyCode::Backspace);
        assert_eq!(form.value(Row::Name), "a");
        assert_eq!(form.key(KeyCode::Esc), Key::Close);
    }

    proptest! {
        #[test]
        fn a_built_table_always_has_a_valid_name_and_keys_that_parse(
            name in ".{0,50}",
            command in ".{0,10}",
            keys in ".{0,20}",
        ) {
            let mut form = AddPluginForm::new();
            form.values[Row::Name as usize] = name;
            form.values[Row::Command as usize] = command;
            form.values[Row::Keys as usize] = keys;
            if let Ok(spec) = form.build() {
                prop_assert!(check_name(&spec.name).is_ok());
                prop_assert!(!spec.command.is_empty());
                prop_assert!(spec.on_key.iter().all(|key| parse_key(key).is_some()));
            }
        }

        #[test]
        fn arguments_survive_being_written_into_the_row_and_read_back(
            args in proptest::collection::vec("[a-z \\t-]{0,6}", 0..5),
        ) {
            let written = join_args(&args).unwrap();
            prop_assert_eq!(split_args(&written).unwrap(), args);
        }

        #[test]
        fn split_args_never_yields_an_unquoted_space(text in "[a-z \"]{0,20}") {
            if let Ok(args) = split_args(&text) {
                let quotes = text.matches('"').count();
                if quotes == 0 {
                    prop_assert!(args.iter().all(|arg| !arg.contains(' ') && !arg.is_empty()));
                }
            }
        }
    }
}
