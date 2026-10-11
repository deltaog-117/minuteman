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
    /// The events it follows without being addressed (\`events = [...]\` of a \`[[plugin]]\`).
    events: &'static [&'static str],
    /// What to tell the user when the program cannot be found.
    how_to_get: &'static str,
}

const BUNDLED: [Bundled; 3] = [
    Bundled {
        name: "finder",
        binary: "mman-finder",
        provides: &["find"],
        events: &[],
        how_to_get: "the fuzzy finder is a plugin and mman-finder was not found; \
            install it with `cargo install --path plugins/finder` (next to mman, or on PATH)",
    },
    Bundled {
        name: "search",
        binary: "mman-search",
        provides: &["locate"],
        events: &[],
        how_to_get: "the structured search is a plugin and mman-search was not found; \
            install it with `cargo install --path plugins/search` (next to mman, or on PATH)",
    },
    // Provides no action: it fills the status bar's git slots and follows the browser to do so.
    Bundled {
        name: "git",
        binary: "mman-git",
        provides: &[],
        events: &["dir_changed", "selection_changed"],
        how_to_get: "the git status is a plugin and mman-git was not found; \
            install it with `cargo install --path plugins/git` (next to mman, or on PATH)",
    },
];

/// The place a bundled program is looked for first: next to the running `mman`.
fn beside_mman(binary: &str) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let path: PathBuf = exe.parent()?.join(binary);
    program_exists(&path.to_string_lossy()).then(|| path.to_string_lossy().into_owned())
}

/// The bundled plugins to start besides `configured`, with the command that runs each.
/// `git_status = false` in the config leaves the git plugin out.
pub fn specs(configured: &[PluginSpec], git_status: bool) -> Vec<PluginSpec> {
    let mut found = specs_found(configured, |binary| {
        beside_mman(binary).or_else(|| program_exists(binary).then(|| binary.to_string()))
    });
    if !git_status {
        found.retain(|spec| spec.name != "git");
    }
    found
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
                events: bundled.events.iter().map(|name| name.to_string()).collect(),
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
            // A bundled plugin that provides no action can only be replaced by name.
            || (!bundled.provides.is_empty()
                && bundled
                    .provides
                    .iter()
                    .all(|action| spec.provides.iter().any(|provided| provided == action)))
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
    fn a_fresh_install_starts_each_bundled_plugin_providing_its_action() {
        let started = specs_found(&[], found);
        assert_eq!(started.len(), 3);
        assert_eq!(started[0].name, "finder");
        assert_eq!(started[0].command, "/opt/bin/mman-finder");
        assert_eq!(started[0].provides, ["find"]);
        assert_eq!(started[1].name, "search");
        assert_eq!(started[1].command, "/opt/bin/mman-search");
        assert_eq!(started[1].provides, ["locate"]);
    }

    #[test]
    fn the_git_plugin_follows_the_browser_and_provides_no_action() {
        let started = specs_found(&[], found);
        let git = started
            .iter()
            .find(|spec| spec.name == "git")
            .expect("bundled");
        assert_eq!(git.command, "/opt/bin/mman-git");
        assert!(git.provides.is_empty());
        assert_eq!(git.events, ["dir_changed", "selection_changed"]);
    }

    #[test]
    fn switching_git_status_off_leaves_only_the_git_plugin_out() {
        let names = |on: bool| -> Vec<String> {
            let mut found = specs_found(&[], found);
            if !on {
                found.retain(|spec| spec.name != "git");
            }
            found.into_iter().map(|spec| spec.name).collect()
        };
        assert_eq!(names(true), ["finder", "search", "git"]);
        assert_eq!(names(false), ["finder", "search"]);
    }

    #[test]
    fn a_missing_program_is_not_started() {
        assert!(specs_found(&[], |_| None).is_empty());
        let only_search = specs_found(&[], |binary| {
            (binary == "mman-search").then(|| binary.to_string())
        });
        assert_eq!(only_search.len(), 1);
        assert_eq!(only_search[0].name, "search");
    }

    #[test]
    fn a_plugin_of_the_same_name_or_providing_the_action_takes_its_place() {
        let names = |configured: &[PluginSpec]| -> Vec<String> {
            specs_found(configured, found)
                .into_iter()
                .map(|spec| spec.name)
                .collect()
        };
        assert_eq!(names(&[configured("finder", &[])]), ["search", "git"]);
        assert_eq!(
            names(&[configured("fzf-picker", &["find"])]),
            ["search", "git"]
        );
        assert_eq!(names(&[configured("search", &[])]), ["finder", "git"]);
        assert_eq!(
            names(&[configured("grep-ui", &["locate"])]),
            ["finder", "git"]
        );
        assert_eq!(names(&[configured("git", &[])]), ["finder", "search"]);
        // A plugin providing nothing must not count as standing in for the git plugin.
        assert_eq!(
            names(&[configured("blame", &[])]),
            ["finder", "search", "git"]
        );
    }

    #[test]
    fn an_unrelated_plugin_leaves_the_bundled_ones_alone() {
        assert_eq!(
            specs_found(&[configured("blame", &["blame"])], found).len(),
            3
        );
    }

    #[test]
    fn what_is_missing_says_how_to_get_it() {
        assert!(missing("find").contains("cargo install --path plugins/finder"));
        assert!(missing("locate").contains("cargo install --path plugins/search"));
        assert_eq!(missing("sort"), "no plugin provides 'sort'");
    }
}
