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
//! The form checks what it can tell from the text alone (a name, a command, keys that name a
//! key, events that exist). What needs the rest of the program — a name or key or `:` word that
//! something else already has, a command that will not start — is checked by the caller, which
//! hands the reason back with [`AddPluginForm::set_error`] so the form stays open.

use crossterm::event::KeyCode;
use plugin_host::EventKind;
use theming::{RawPluginSpec, parse_key};

/// The longest plugin name the form accepts, in characters.
const MAX_NAME_CHARS: usize = 40;

/// One row of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Name,
    Command,
    Args,
    Keys,
    Commands,
    Events,
    Provides,
}

impl Row {
    pub const ALL: [Row; 7] = [
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
}

#[derive(Debug, Default)]
pub struct AddPluginForm {
    cursor: usize,
    values: [String; 7],
    error: Option<String>,
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
            KeyCode::Enter => match self.build() {
                Ok(spec) => return Key::Submit(spec),
                Err(message) => self.error = Some(message),
            },
            KeyCode::Down | KeyCode::Tab => self.step(1, rows),
            KeyCode::Up | KeyCode::BackTab => self.step(rows - 1, rows),
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

    fn step(&mut self, by: usize, rows: usize) {
        self.cursor = (self.cursor + by) % rows;
        self.error = None;
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
