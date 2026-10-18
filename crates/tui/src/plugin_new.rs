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

//! `mman plugin new <name> [--lang sh|python]`: a working plugin to start from. It writes a folder
//! into the drop-in directory with a `plugin.toml` and a short program that answers the `:<name>`
//! command by putting a line on the status bar, so the loop (edit, `mman plugin check`, switch it
//! on in the plugin menu) can be tried at once. A name is a [`PluginName`], which can only be built
//! from characters that are safe in a folder name, a TOML string and the generated program alike,
//! so nothing here needs to quote it.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use theming::dropin::MANIFEST;

/// The longest name accepted.
const MAX_NAME_CHARS: usize = 64;

/// A plugin's name: 1 to 64 ASCII letters, digits, `-` and `_`, starting with a letter or digit.
/// Stricter than a manifest requires, so it can never be `..`, hidden, or need escaping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginName(String);

/// Why a name was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    BadStart(char),
    BadChar(char),
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "the plugin name is empty"),
            Self::TooLong => write!(
                f,
                "the plugin name is longer than {MAX_NAME_CHARS} characters"
            ),
            Self::BadStart(c) => write!(
                f,
                "the plugin name must start with a letter or digit, not {c:?}"
            ),
            Self::BadChar(c) => write!(
                f,
                "the plugin name may hold only letters, digits, '-' and '_', not {c:?}"
            ),
        }
    }
}

impl PluginName {
    /// # Errors
    ///
    /// A [`NameError`] saying which rule `name` breaks.
    pub fn try_new(name: &str) -> Result<Self, NameError> {
        let first = name.chars().next().ok_or(NameError::Empty)?;
        if !first.is_ascii_alphanumeric() {
            return Err(NameError::BadStart(first));
        }
        if let Some(bad) = name
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
        {
            return Err(NameError::BadChar(bad));
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(NameError::TooLong);
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The language of the program a new plugin starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    Sh,
    Python,
}

impl Lang {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "sh" => Some(Self::Sh),
            "python" => Some(Self::Python),
            _ => None,
        }
    }

    /// The program's file name, which the manifest's `command` points at.
    fn program(self) -> &'static str {
        match self {
            Self::Sh => "plugin.sh",
            Self::Python => "plugin.py",
        }
    }
}

/// Why a plugin could not be created.
#[derive(Debug)]
pub enum NewError {
    NoConfigDir,
    Exists(PathBuf),
    Io(PathBuf, io::Error),
}

impl fmt::Display for NewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoConfigDir => write!(f, "could not find the config folder to put the plugin in"),
            Self::Exists(dir) => write!(f, "{} already exists; nothing was changed", dir.display()),
            Self::Io(path, e) => write!(f, "{}: {e}", path.display()),
        }
    }
}

impl std::error::Error for NewError {}

fn manifest(name: &PluginName, lang: Lang) -> String {
    let name = name.as_str();
    format!(
        "# {name}: edit this file, then run `mman plugin check {name}` to start it for half a second.\n\
         name = \"{name}\"\n\
         description = \"A plugin made with `mman plugin new`.\"\n\
         command = \"./{program}\"\n\
         api_version = {api}\n\
         \n\
         # Run it from the `:` line as `:{name}`.\n\
         commands = [\"{name}\"]\n\
         # Run it from a key (pick one that is free; `mman plugin check` says if it is taken).\n\
         # on_key = \"\"\n\
         # Also hear about these (dir_changed, selection_changed, job_finished).\n\
         # events = [\"dir_changed\"]\n",
        program = lang.program(),
        api = plugin_host::API_VERSION,
    )
}

fn program(name: &PluginName, lang: Lang) -> String {
    let name = name.as_str();
    match lang {
        Lang::Sh => format!(
            "#!/bin/sh\n\
             # {name}: a Minuteman plugin. Minuteman starts this program and talks to it on stdin and\n\
             # stdout, one JSON object per line. Each line it reads is an event; a line it prints with\n\
             # method \"log\" shows on the status bar.\n\
             while IFS= read -r line; do\n\
             \x20   case $line in\n\
             \x20       *'\"name\":\"command\"'*)\n\
             \x20           printf '%s\\n' '{{\"jsonrpc\":\"2.0\",\"method\":\"log\",\"params\":{{\"level\":\"info\",\"message\":\"hello from {name}\"}}}}'\n\
             \x20           ;;\n\
             \x20   esac\n\
             done\n"
        ),
        Lang::Python => format!(
            "#!/usr/bin/env python3\n\
             \"\"\"{name}: a Minuteman plugin.\n\
             \n\
             Minuteman starts this program and talks to it on stdin and stdout, one JSON object per\n\
             line. Each line it reads is an event; a line it prints with method \"log\" shows on the\n\
             status bar.\n\
             \"\"\"\n\
             import json\n\
             import sys\n\
             \n\
             \n\
             def log(message, level=\"info\"):\n\
             \x20   line = {{\"jsonrpc\": \"2.0\", \"method\": \"log\", \"params\": {{\"level\": level, \"message\": message}}}}\n\
             \x20   print(json.dumps(line), flush=True)\n\
             \n\
             \n\
             for line in sys.stdin:\n\
             \x20   event = json.loads(line)[\"params\"]\n\
             \x20   if event[\"name\"] == \"command\":\n\
             \x20       log(\"hello from {name}\")\n"
        ),
    }
}

/// Writes the new plugin into `<root>/<name>/` and returns that folder. Nothing existing is
/// overwritten: a folder of that name makes it [`NewError::Exists`], and a failure part-way
/// removes what was written.
///
/// # Errors
///
/// [`NewError::Exists`] when the folder is there already, [`NewError::Io`] when a write fails.
pub fn create(root: &Path, name: &PluginName, lang: Lang) -> Result<PathBuf, NewError> {
    let dir = root.join(name.as_str());
    fs::create_dir_all(root).map_err(|e| NewError::Io(root.to_path_buf(), e))?;
    match fs::create_dir(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(NewError::Exists(dir)),
        Err(e) => return Err(NewError::Io(dir, e)),
    }
    let written = write_file(&dir.join(MANIFEST), &manifest(name, lang), 0o644)
        .and_then(|()| write_file(&dir.join(lang.program()), &program(name, lang), 0o755));
    match written {
        Ok(()) => Ok(dir),
        Err((path, e)) => {
            // Half a plugin would be listed by the plugin menu as broken.
            let _ = fs::remove_dir_all(&dir);
            Err(NewError::Io(path, e))
        }
    }
}

fn write_file(path: &Path, text: &str, mode: u32) -> Result<(), (PathBuf, io::Error)> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|e| (path.to_path_buf(), e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU32, Ordering};
    use theming::DropIn;

    /// A folder of its own under the system's temporary one, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "minuteman-plugin-new-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).expect("create scratch folder");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn name(text: &str) -> PluginName {
        PluginName::try_new(text).expect("a valid name")
    }

    #[test]
    fn names_that_could_escape_the_folder_or_the_quoting_are_refused() {
        assert_eq!(PluginName::try_new(""), Err(NameError::Empty));
        assert_eq!(PluginName::try_new(".."), Err(NameError::BadStart('.')));
        assert_eq!(PluginName::try_new("-x"), Err(NameError::BadStart('-')));
        assert_eq!(PluginName::try_new("a/b"), Err(NameError::BadChar('/')));
        assert_eq!(PluginName::try_new("a b"), Err(NameError::BadChar(' ')));
        assert_eq!(PluginName::try_new("a\"b"), Err(NameError::BadChar('"')));
        assert_eq!(PluginName::try_new("a'b"), Err(NameError::BadChar('\'')));
        assert_eq!(PluginName::try_new("é"), Err(NameError::BadStart('é')));
        assert_eq!(
            PluginName::try_new(&"a".repeat(65)),
            Err(NameError::TooLong)
        );
        assert!(PluginName::try_new(&"a".repeat(64)).is_ok());
    }

    proptest! {
        /// Whatever name is accepted, the generated manifest is read back with that name and a
        /// command inside the new folder, in either language.
        #[test]
        fn an_accepted_name_makes_a_manifest_the_loader_accepts(text in "\\PC{0,70}") {
            if let Ok(accepted) = PluginName::try_new(&text) {
                for lang in [Lang::Sh, Lang::Python] {
                    let dir = Path::new("/p").join(accepted.as_str());
                    let found = DropIn::from_manifest(&dir, &manifest(&accepted, lang))
                        .expect("the generated manifest loads");
                    prop_assert_eq!(&found.spec.name, accepted.as_str());
                    prop_assert_eq!(found.spec.commands.clone(), vec![accepted.as_str().to_owned()]);
                    prop_assert_eq!(found.api_version, Some(plugin_host::API_VERSION));
                    prop_assert!(found.spec.command.starts_with("/p/"));
                }
            }
        }
    }

    #[test]
    fn a_new_plugin_is_a_folder_the_loader_finds_and_the_program_can_run() {
        let scratch = Scratch::new();
        for (lang, folder, file) in [
            (Lang::Sh, "with-sh", "plugin.sh"),
            (Lang::Python, "with-python", "plugin.py"),
        ] {
            let made = create(&scratch.0, &name(folder), lang).expect("created");
            assert!(made.join(file).is_file());
            let found = DropIn::load(&made).expect("manifest loads");
            assert_eq!(
                found.spec.command,
                made.join(format!("./{file}")).to_string_lossy()
            );
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(made.join(file)).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "the program is executable");
        }
    }

    /// Starts the new plugin's program, sends it a `command` event and returns the line it logs.
    fn answer(lang: Lang) -> Option<String> {
        let scratch = Scratch::new();
        let made = create(&scratch.0, &name("hello"), lang).expect("created");
        let mut child = Command::new(made.join(lang.program()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .ok()?;
        let mut stdin = child.stdin.take()?;
        let event = r#"{"jsonrpc":"2.0","method":"event","params":{"name":"key","cwd":"/","selection":[]}}"#;
        let command = r#"{"jsonrpc":"2.0","method":"event","params":{"name":"command","cwd":"/","selection":[],"command":"hello"}}"#;
        writeln!(stdin, "{event}\n{command}").ok()?;
        drop(stdin);
        let mut lines = BufReader::new(child.stdout.take()?).lines();
        let first = lines.next()?.ok();
        let rest = lines.next();
        let _ = child.wait();
        assert!(rest.is_none(), "only the command is answered: {rest:?}");
        first
    }

    #[test]
    fn the_sh_program_answers_a_command_event_and_nothing_else() {
        let line = answer(Lang::Sh).expect("an answer");
        let value: serde_json::Value = serde_json::from_str(&line).expect("one JSON line");
        assert_eq!(value["method"], "log");
        assert_eq!(value["params"]["message"], "hello from hello");
    }

    #[test]
    fn the_python_program_answers_a_command_event_and_nothing_else() {
        // A machine without python3 has nothing to run here; the sh program is tested above.
        if Command::new("python3").arg("--version").output().is_err() {
            return;
        }
        let line = answer(Lang::Python).expect("an answer");
        let value: serde_json::Value = serde_json::from_str(&line).expect("one JSON line");
        assert_eq!(value["params"]["message"], "hello from hello");
    }

    #[test]
    fn an_existing_folder_is_never_touched() {
        let scratch = Scratch::new();
        let first = create(&scratch.0, &name("same"), Lang::Sh).expect("created");
        fs::write(first.join("plugin.sh"), "mine").unwrap();
        assert!(matches!(
            create(&scratch.0, &name("same"), Lang::Python),
            Err(NewError::Exists(_))
        ));
        assert_eq!(fs::read_to_string(first.join("plugin.sh")).unwrap(), "mine");
    }

    #[test]
    fn a_failed_write_leaves_no_half_made_folder() {
        let scratch = Scratch::new();
        // A file where the folder's name would go makes the root itself unusable.
        let blocker = scratch.0.join("blocked");
        fs::write(&blocker, "").unwrap();
        assert!(matches!(
            create(&blocker, &name("x"), Lang::Sh),
            Err(NewError::Io(..))
        ));
        assert!(blocker.is_file());
    }
}
