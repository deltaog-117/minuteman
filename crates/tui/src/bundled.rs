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

//! The plugins that ship with Minuteman and start on a fresh install without being configured,
//! the way a browser starts with its system add-ons. Each is its own program installed beside
//! `mman` (`cargo install --path plugins/<name>`, or a build's `target` folder) and speaks the
//! same plugin protocol as one the user writes.
//!
//! A bundled plugin is a default, not a requirement. A plugin the user configures under the same
//! name, or one that provides the same actions, takes its place, and with the program missing the
//! action does nothing but say how to get it.

use std::path::PathBuf;

use theming::PluginSpec;

use crate::open::program_exists;

struct Bundled {
    name: &'static str,
    /// The program, found beside `mman` or on `PATH`.
    binary: &'static str,
    provides: &'static [&'static str],
    /// What to tell the user when the program cannot be found.
    how_to_get: &'static str,
}

const BUNDLED: [Bundled; 1] = [Bundled {
    name: "finder",
    binary: "mman-finder",
    provides: &["find"],
    how_to_get: "the fuzzy finder is a plugin and mman-finder was not found; \
        install it with `cargo install --path plugins/finder` (next to mman, or on PATH)",
}];

/// The place a bundled program is looked for first: next to the running `mman`.
fn beside_mman(binary: &str) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let path: PathBuf = exe.parent()?.join(binary);
    program_exists(&path.to_string_lossy()).then(|| path.to_string_lossy().into_owned())
}

/// The bundled plugins to start besides `configured`, with the command that runs each.
pub fn specs(configured: &[PluginSpec]) -> Vec<PluginSpec> {
    specs_found(configured, |binary| {
        beside_mman(binary).or_else(|| program_exists(binary).then(|| binary.to_string()))
    })
}

/// `specs`, with `locate` standing in for the search for a program, which says where it is.
fn specs_found(
    configured: &[PluginSpec],
    locate: impl Fn(&str) -> Option<String>,
) -> Vec<PluginSpec> {
    BUNDLED
        .iter()
        .filter(|bundled| !replaced(bundled, configured))
        .filter_map(|bundled| {
            Some(PluginSpec {
                name: bundled.name.to_string(),
                command: locate(bundled.binary)?,
                args: Vec::new(),
                on_keys: Vec::new(),
                commands: Vec::new(),
                events: Vec::new(),
                provides: bundled
                    .provides
                    .iter()
                    .map(|name| name.to_string())
                    .collect(),
            })
        })
        .collect()
}

/// Whether the user configured a plugin that stands in for `bundled`.
fn replaced(bundled: &Bundled, configured: &[PluginSpec]) -> bool {
    configured.iter().any(|spec| {
        spec.name == bundled.name
            || bundled
                .provides
                .iter()
                .all(|action| spec.provides.iter().any(|provided| provided == action))
    })
}

/// What to say when no plugin provides `action`.
pub fn missing(action: &str) -> String {
    BUNDLED
        .iter()
        .find(|bundled| bundled.provides.contains(&action))
        .map_or_else(
            || format!("no plugin provides '{action}'"),
            |bundled| bundled.how_to_get.to_string(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(name: &str, provides: &[&str]) -> PluginSpec {
        PluginSpec {
            name: name.to_string(),
            command: "true".to_string(),
            args: Vec::new(),
            on_keys: Vec::new(),
            commands: Vec::new(),
            events: Vec::new(),
            provides: provides.iter().map(|name| name.to_string()).collect(),
        }
    }

    fn found(binary: &str) -> Option<String> {
        Some(format!("/opt/bin/{binary}"))
    }

    #[test]
    fn a_fresh_install_starts_the_finder_providing_find() {
        let started = specs_found(&[], found);
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].name, "finder");
        assert_eq!(started[0].command, "/opt/bin/mman-finder");
        assert_eq!(started[0].provides, ["find"]);
    }

    #[test]
    fn a_missing_program_is_not_started() {
        assert!(specs_found(&[], |_| None).is_empty());
    }

    #[test]
    fn a_plugin_of_the_same_name_or_providing_find_takes_its_place() {
        assert!(specs_found(&[configured("finder", &[])], found).is_empty());
        assert!(specs_found(&[configured("fzf-picker", &["find"])], found).is_empty());
    }

    #[test]
    fn an_unrelated_plugin_leaves_the_finder_alone() {
        assert_eq!(
            specs_found(&[configured("blame", &["blame"])], found).len(),
            1
        );
    }

    #[test]
    fn what_is_missing_says_how_to_get_it() {
        assert!(missing("find").contains("cargo install --path plugins/finder"));
        assert_eq!(missing("sort"), "no plugin provides 'sort'");
    }
}
