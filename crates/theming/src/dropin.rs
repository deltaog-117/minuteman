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

//! Drop-in plugins: a folder under `~/.config/minuteman/plugins/` holding a `plugin.toml`.
//!
//! The manifest uses the keys of a `[[plugin]]` table (`name`, `command`, `args`, `on_key`,
//! `commands`, `events`, `provides`) plus a `description`, so a plugin moves between the two by
//! copying the lines. `name` may be left out, and is then the folder's name. In `command` and
//! `args`, `{dir}` stands for the plugin's folder, and a `command` that begins with `./` is taken
//! from it, so a plugin can ship its program beside the manifest and run from wherever it is put.
//!
//! Finding a folder never starts anything: the plugin menu lists it switched off, and the user
//! switches it on. A folder whose manifest cannot be read is reported and left out, so one bad
//! plugin never hides the others.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::config::RawPluginSpec;

/// The manifest's file name inside a plugin's folder.
pub const MANIFEST: &str = "plugin.toml";

/// A manifest larger than this is not read: it is a few lines in any real plugin.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// Why a folder's manifest was not accepted.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("{MANIFEST} is not valid: {0}")]
    Malformed(String),
    #[error("{MANIFEST} is larger than {} KiB", MAX_MANIFEST_BYTES / 1024)]
    TooLarge,
    #[error("{MANIFEST} could not be read: {0}")]
    Unreadable(String),
    #[error("it has no name, and the folder's name is not one")]
    NoName,
    #[error("the name {0:?} must have no spaces or slashes")]
    BadName(String),
    #[error("it has no command")]
    NoCommand,
}

/// The file as written: a `[[plugin]]` table with a description.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawManifest {
    #[serde(flatten)]
    spec: RawPluginSpec,
    description: String,
}

/// A plugin found in a folder: its table, ready to start, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropIn {
    pub dir: PathBuf,
    pub spec: RawPluginSpec,
    pub description: String,
}

impl DropIn {
    /// Reads the manifest `text` of the plugin in `dir`.
    ///
    /// # Errors
    ///
    /// A [`ManifestError`] when the text is not TOML of the right shape, or the plugin ends up
    /// with no usable name or no command.
    pub fn from_manifest(dir: &Path, text: &str) -> Result<Self, ManifestError> {
        let raw: RawManifest =
            toml::from_str(text).map_err(|e| ManifestError::Malformed(e.message().to_string()))?;
        let mut spec = raw.spec;
        if spec.name.trim().is_empty() {
            spec.name = dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        spec.name = spec.name.trim().to_string();
        if spec.name.is_empty() {
            return Err(ManifestError::NoName);
        }
        if spec.name.contains(char::is_whitespace) || spec.name.contains('/') {
            return Err(ManifestError::BadName(spec.name));
        }
        if spec.command.trim().is_empty() {
            return Err(ManifestError::NoCommand);
        }
        let here = dir.to_string_lossy();
        spec.command = if spec.command.starts_with("./") {
            dir.join(&spec.command).to_string_lossy().into_owned()
        } else {
            spec.command.replace("{dir}", &here)
        };
        for arg in &mut spec.args {
            *arg = arg.replace("{dir}", &here);
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            spec,
            description: raw.description.trim().to_string(),
        })
    }

    /// Reads `<dir>/plugin.toml`.
    ///
    /// # Errors
    ///
    /// A [`ManifestError`] when the file cannot be read, is too large, or is not accepted.
    pub fn load(dir: &Path) -> Result<Self, ManifestError> {
        let file = fs::File::open(dir.join(MANIFEST))
            .map_err(|e| ManifestError::Unreadable(e.to_string()))?;
        let mut text = String::new();
        // One byte past the cap tells a file at the cap from one beyond it.
        file.take(MAX_MANIFEST_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|e| ManifestError::Unreadable(e.to_string()))?;
        if text.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge);
        }
        Self::from_manifest(dir, &text)
    }
}

/// What a scan of the plugins folder found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    /// The plugins, in the order of their folders' names.
    pub found: Vec<DropIn>,
    /// One line for each folder with a manifest that was not accepted, and one for each name
    /// two folders both claim (the first folder keeps it).
    pub notices: Vec<String>,
}

/// Looks in each folder of `root` for a `plugin.toml`. A missing `root` is an empty scan, and a
/// folder without a manifest is not a plugin and is passed over without a word.
pub fn scan(root: &Path) -> Scan {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join(MANIFEST).is_file())
        .collect();
    dirs.sort();
    let mut out = Scan::default();
    for dir in dirs {
        let folder = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        match DropIn::load(&dir) {
            Ok(found) if out.found.iter().any(|f| f.spec.name == found.spec.name) => {
                out.notices.push(format!(
                    "drop-in {folder}: the name {} is already used by another folder",
                    found.spec.name
                ));
            }
            Ok(found) => out.found.push(found),
            Err(e) => out.notices.push(format!("drop-in {folder}: {e}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn at(dir: &str, text: &str) -> Result<DropIn, ManifestError> {
        DropIn::from_manifest(Path::new(dir), text)
    }

    #[test]
    fn the_keys_are_those_of_a_plugin_table_plus_a_description() {
        let found = at(
            "/p/stats",
            r#"
            name = "stats"
            command = "python3"
            args = ["{dir}/stats.py", "-q"]
            on_key = "b"
            commands = ["stats"]
            events = ["dir_changed"]
            provides = ["find"]
            description = "  Counts files.  "
            "#,
        )
        .unwrap();
        assert_eq!(found.spec.name, "stats");
        assert_eq!(found.spec.command, "python3");
        assert_eq!(found.spec.args, ["/p/stats/stats.py", "-q"]);
        assert_eq!(found.spec.on_key, ["b"]);
        assert_eq!(found.spec.commands, ["stats"]);
        assert_eq!(found.spec.events, ["dir_changed"]);
        assert_eq!(found.spec.provides, ["find"]);
        assert_eq!(found.description, "Counts files.");
    }

    #[test]
    fn the_name_defaults_to_the_folders() {
        let found = at("/p/stats", r#"command = "x""#).unwrap();
        assert_eq!(found.spec.name, "stats");
    }

    #[test]
    fn a_command_with_a_dot_slash_is_taken_from_the_folder() {
        let found = at("/p/stats", r#"command = "./run.py""#).unwrap();
        assert_eq!(found.spec.command, "/p/stats/./run.py");
        let found = at("/p/stats", r#"command = "{dir}/run.py""#).unwrap();
        assert_eq!(found.spec.command, "/p/stats/run.py");
        let found = at("/p/stats", r#"command = "python3""#).unwrap();
        assert_eq!(found.spec.command, "python3");
    }

    #[test]
    fn what_is_wrong_is_named() {
        assert_eq!(at("/p/a", ""), Err(ManifestError::NoCommand));
        assert_eq!(at("/p/a", "command = '  '"), Err(ManifestError::NoCommand));
        assert_eq!(
            at("/p/a", "name = 'a b'\ncommand = 'x'"),
            Err(ManifestError::BadName("a b".into()))
        );
        assert_eq!(
            at("/p/a", "name = 'a/b'\ncommand = 'x'"),
            Err(ManifestError::BadName("a/b".into()))
        );
        assert_eq!(at("/", "command = 'x'"), Err(ManifestError::NoName));
        assert!(matches!(
            at("/p/a", "command = "),
            Err(ManifestError::Malformed(_))
        ));
        assert!(matches!(
            at("/p/a", "command = 3"),
            Err(ManifestError::Malformed(_))
        ));
    }

    /// A folder of its own under the system's temporary one, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "mman-dropin-{label}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn plugin(&self, folder: &str, manifest: &str) {
            let dir = self.0.join(folder);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(MANIFEST), manifest).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_missing_folder_is_an_empty_scan() {
        let root = Scratch::new("missing");
        assert_eq!(scan(&root.0.join("nope")), Scan::default());
    }

    #[test]
    fn the_scan_lists_folders_in_order_and_reports_the_bad_ones() {
        let root = Scratch::new("scan");
        root.plugin("b", "command = 'b'");
        root.plugin("a", "command = 'a'");
        root.plugin("broken", "command = ");
        root.plugin("empty", "");
        fs::create_dir_all(root.0.join("no_manifest")).unwrap();
        fs::write(root.0.join("loose_file"), "command = 'x'").unwrap();
        let scan = scan(&root.0);
        let names: Vec<&str> = scan.found.iter().map(|f| f.spec.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(scan.notices.len(), 2);
        assert!(scan.notices[0].starts_with("drop-in broken: "));
        assert!(scan.notices[1].starts_with("drop-in empty: it has no command"));
    }

    #[test]
    fn two_folders_with_one_name_keep_the_first() {
        let root = Scratch::new("twice");
        root.plugin("one", "name = 'same'\ncommand = '1'");
        root.plugin("two", "name = 'same'\ncommand = '2'");
        let scan = scan(&root.0);
        assert_eq!(scan.found.len(), 1);
        assert_eq!(scan.found[0].spec.command, "1");
        assert_eq!(scan.notices.len(), 1);
        assert!(scan.notices[0].contains("drop-in two"));
    }

    #[test]
    fn a_manifest_over_the_cap_is_refused_unread() {
        let root = Scratch::new("big");
        root.plugin("big", &format!("command = 'x'\n#{}", "a".repeat(70_000)));
        assert_eq!(
            DropIn::load(&root.0.join("big")),
            Err(ManifestError::TooLarge)
        );
    }

    proptest! {
        /// However the text is mangled, reading it never panics, and what is accepted always has
        /// a name without spaces or slashes and a command.
        #[test]
        fn what_is_accepted_always_has_a_clean_name_and_a_command(text in "\\PC{0,200}") {
            if let Ok(found) = at("/p/some", &text) {
                prop_assert!(!found.spec.name.is_empty());
                prop_assert!(!found.spec.name.contains(char::is_whitespace));
                prop_assert!(!found.spec.name.contains('/'));
                prop_assert!(!found.spec.command.trim().is_empty());
            }
        }
    }
}
