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

use theming::{HookKind, PluginSpec, PreviewHook};

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

const BUNDLED: [Bundled; 7] = [
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
    // Provides no action and follows no event: it fills the header pill on its own clock.
    Bundled {
        name: "hud",
        binary: "mman-hud",
        provides: &[],
        events: &[],
        how_to_get: "the system figures are a plugin and mman-hud was not found; \
            install it with `cargo install --path plugins/hud` (next to mman, or on PATH)",
    },
    // Provides the `preview` role rather than a key's action: the interface tells it which file the
    // preview column is about and it answers with the document to show.
    Bundled {
        name: "preview",
        binary: "mman-preview",
        provides: &["preview"],
        events: &[],
        how_to_get: "the preview column is a plugin and mman-preview was not found; \
            install it with `cargo install --path plugins/preview` (next to mman, or on PATH)",
    },
    // Provides the `inspect` role: the interface names the entry its panel is about and the plugin
    // answers with the rows to show.
    Bundled {
        name: "inspect",
        binary: "mman-inspect",
        provides: &["inspect"],
        events: &[],
        how_to_get: "the Inspect panel is a plugin and mman-inspect was not found; \
            install it with `cargo install --path plugins/inspect` (next to mman, or on PATH)",
    },
    // Provides the `archive` role: the interface names the work (compress or extract) and the
    // plugin does it on the local disk, reporting each entry and how the request ended.
    Bundled {
        name: "archive",
        binary: "mman-archive",
        provides: &["archive"],
        events: &[],
        how_to_get: "compressing and extracting are a plugin and mman-archive was not found; \
            install it with `cargo install --path plugins/archive` (next to mman, or on PATH)",
    },
];

/// The place a bundled program is looked for first: next to the running `mman`.
fn beside_mman(binary: &str) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let path: PathBuf = exe.parent()?.join(binary);
    program_exists(&path.to_string_lossy()).then(|| path.to_string_lossy().into_owned())
}

/// The bundled plugins to start besides `configured`, with the command that runs each.
/// `git_status = false` in the config leaves the git plugin out, and the system plugin starts
/// only with `system_hud = true`.
pub fn specs(
    configured: &[PluginSpec],
    git_status: bool,
    system_hud: bool,
    hooks: &[PreviewHook],
) -> Vec<PluginSpec> {
    let mut found = specs_found(
        configured,
        |binary| beside_mman(binary).or_else(|| program_exists(binary).then(|| binary.to_string())),
        hooks,
    );
    if !git_status {
        found.retain(|spec| spec.name != "git");
    }
    if !system_hud {
        found.retain(|spec| spec.name != "hud");
    }
    found
}

/// `specs`, with `locate` standing in for the search for a program, which says where it is.
fn specs_found(
    configured: &[PluginSpec],
    locate: impl Fn(&str) -> Option<String>,
    hooks: &[PreviewHook],
) -> Vec<PluginSpec> {
    BUNDLED
        .iter()
        .filter(|bundled| !replaced(bundled, configured))
        .filter_map(|bundled| {
            Some(PluginSpec {
                name: bundled.name.to_string(),
                command: locate(bundled.binary)?,
                args: if bundled.name == "preview" {
                    hook_args(hooks)
                } else {
                    Vec::new()
                },
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

/// The command-line arguments that hand the preview plugin the `[[preview_hook]]` tables, one
/// `--text-hook <json>` for each that extracts text and one `--image-hook <json>` for each that
/// renders a picture.
///
/// The first hook that names an extension decides it, whatever its kind, so an image hook for
/// `pdf` ahead of a text hook for `pdf` leaves the text hook nothing: the extension is taken out
/// of every later hook before it is passed on, and a hook left with no extension is dropped.
fn hook_args(hooks: &[PreviewHook]) -> Vec<String> {
    let mut claimed: Vec<String> = Vec::new();
    let mut args = Vec::new();
    for hook in hooks {
        let free: Vec<&String> = hook
            .extensions
            .iter()
            .filter(|ext| !claimed.iter().any(|taken| taken.eq_ignore_ascii_case(ext)))
            .collect();
        claimed.extend(hook.extensions.iter().cloned());
        if free.is_empty() {
            continue;
        }
        args.push(
            match hook.kind {
                HookKind::Text => "--text-hook",
                HookKind::Image => "--image-hook",
            }
            .to_string(),
        );
        args.push(
            serde_json::json!({
                "extensions": free,
                "command": hook.command,
                "timeout_ms": hook.timeout_ms,
            })
            .to_string(),
        );
    }
    args
}

/// What to tell the user at startup when the bundled preview plugin could not be started and
/// nothing the user configured takes its place: without it the preview column shows only names.
pub fn missing_previewer(configured: &[PluginSpec], started: &[PluginSpec]) -> Option<String> {
    let bundled = BUNDLED.iter().find(|b| b.name == "preview")?;
    let stands_in = replaced(bundled, configured);
    let running = started.iter().any(|spec| spec.name == bundled.name);
    (!stands_in && !running).then(|| bundled.how_to_get.to_string())
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
        let started = specs_found(&[], found, &[]);
        assert_eq!(started.len(), 7);
        assert_eq!(started[0].name, "finder");
        assert_eq!(started[0].command, "/opt/bin/mman-finder");
        assert_eq!(started[0].provides, ["find"]);
        assert_eq!(started[1].name, "search");
        assert_eq!(started[1].command, "/opt/bin/mman-search");
        assert_eq!(started[1].provides, ["locate"]);
    }

    #[test]
    fn the_git_plugin_follows_the_browser_and_provides_no_action() {
        let started = specs_found(&[], found, &[]);
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
        let names = |git: bool, system: bool| -> Vec<String> {
            let mut found = specs_found(&[], found, &[]);
            if !git {
                found.retain(|spec| spec.name != "git");
            }
            if !system {
                found.retain(|spec| spec.name != "hud");
            }
            found.into_iter().map(|spec| spec.name).collect()
        };
        assert_eq!(
            names(true, true),
            [
                "finder", "search", "git", "hud", "preview", "inspect", "archive"
            ]
        );
        assert_eq!(
            names(false, true),
            ["finder", "search", "hud", "preview", "inspect", "archive"]
        );
        assert_eq!(
            names(true, false),
            ["finder", "search", "git", "preview", "inspect", "archive"]
        );
    }

    #[test]
    fn the_system_plugin_follows_no_event_and_provides_no_action() {
        let started = specs_found(&[], found, &[]);
        let hud = started
            .iter()
            .find(|spec| spec.name == "hud")
            .expect("bundled");
        assert_eq!(hud.command, "/opt/bin/mman-hud");
        assert!(hud.provides.is_empty() && hud.events.is_empty());
    }

    #[test]
    fn the_archive_plugin_provides_the_archive_role_and_says_how_to_get_it() {
        let started = specs_found(&[], found, &[]);
        let archive = started
            .iter()
            .find(|spec| spec.name == "archive")
            .expect("bundled");
        assert_eq!(archive.command, "/opt/bin/mman-archive");
        assert_eq!(archive.provides, ["archive"]);
        assert!(missing("archive").contains("mman-archive"));
    }

    #[test]
    fn a_missing_program_is_not_started() {
        assert!(specs_found(&[], |_| None, &[]).is_empty());
        let only_search = specs_found(
            &[],
            |binary| (binary == "mman-search").then(|| binary.to_string()),
            &[],
        );
        assert_eq!(only_search.len(), 1);
        assert_eq!(only_search[0].name, "search");
    }

    #[test]
    fn a_plugin_of_the_same_name_or_providing_the_action_takes_its_place() {
        let names = |configured: &[PluginSpec]| -> Vec<String> {
            specs_found(configured, found, &[])
                .into_iter()
                .map(|spec| spec.name)
                .collect()
        };
        assert_eq!(
            names(&[configured("finder", &[])]),
            ["search", "git", "hud", "preview", "inspect", "archive"]
        );
        assert_eq!(
            names(&[configured("fzf-picker", &["find"])]),
            ["search", "git", "hud", "preview", "inspect", "archive"]
        );
        assert_eq!(
            names(&[configured("search", &[])]),
            ["finder", "git", "hud", "preview", "inspect", "archive"]
        );
        assert_eq!(
            names(&[configured("grep-ui", &["locate"])]),
            ["finder", "git", "hud", "preview", "inspect", "archive"]
        );
        assert_eq!(
            names(&[configured("git", &[])]),
            ["finder", "search", "hud", "preview", "inspect", "archive"]
        );
        // A plugin providing nothing must not count as standing in for the git plugin.
        assert_eq!(
            names(&[configured("blame", &[])]),
            [
                "finder", "search", "git", "hud", "preview", "inspect", "archive"
            ]
        );
    }

    #[test]
    fn an_unrelated_plugin_leaves_the_bundled_ones_alone() {
        assert_eq!(
            specs_found(&[configured("blame", &["blame"])], found, &[]).len(),
            7
        );
    }

    #[test]
    fn what_is_missing_says_how_to_get_it() {
        assert!(missing("find").contains("cargo install --path plugins/finder"));
        assert!(missing("locate").contains("cargo install --path plugins/search"));
        assert!(missing("preview").contains("cargo install --path plugins/preview"));
        assert!(missing("inspect").contains("cargo install --path plugins/inspect"));
        assert!(missing("archive").contains("cargo install --path plugins/archive"));
        assert_eq!(missing("sort"), "no plugin provides 'sort'");
    }

    fn hook(extensions: &[&str], kind: HookKind) -> PreviewHook {
        PreviewHook {
            extensions: extensions.iter().map(|ext| (*ext).to_string()).collect(),
            command: format!("extract {{in}} {{out}} # {extensions:?}"),
            kind,
            timeout_ms: Some(2000),
        }
    }

    #[test]
    fn the_preview_plugin_provides_preview_and_follows_no_event() {
        let started = specs_found(&[], found, &[]);
        let preview = started
            .iter()
            .find(|spec| spec.name == "preview")
            .expect("bundled");
        assert_eq!(preview.command, "/opt/bin/mman-preview");
        assert_eq!(preview.provides, ["preview"]);
        assert!(preview.events.is_empty() && preview.args.is_empty());
    }

    #[test]
    fn the_inspect_plugin_provides_inspect_and_follows_no_event() {
        let started = specs_found(&[], found, &[]);
        let inspect = started
            .iter()
            .find(|spec| spec.name == "inspect")
            .expect("bundled");
        assert_eq!(inspect.command, "/opt/bin/mman-inspect");
        assert_eq!(inspect.provides, ["inspect"]);
        assert!(inspect.events.is_empty() && inspect.args.is_empty());
    }

    #[test]
    fn a_hook_reaches_the_preview_plugin_as_json_under_the_flag_of_its_kind() {
        let hooks = [
            hook(&["pdf"], HookKind::Text),
            hook(&["mp4"], HookKind::Image),
        ];
        let started = specs_found(&[], found, &hooks);
        let args = &started
            .iter()
            .find(|spec| spec.name == "preview")
            .expect("bundled")
            .args;
        assert_eq!(args.len(), 4, "{args:?}");
        assert_eq!(args[0], "--text-hook");
        let json: serde_json::Value = serde_json::from_str(&args[1]).expect("JSON");
        assert_eq!(json["extensions"], serde_json::json!(["pdf"]));
        assert_eq!(json["timeout_ms"], 2000);
        assert!(json["command"].as_str().is_some_and(|c| c.contains("{in}")));
        assert_eq!(args[2], "--image-hook");
        let json: serde_json::Value = serde_json::from_str(&args[3]).expect("JSON");
        assert_eq!(json["extensions"], serde_json::json!(["mp4"]));
        // No other bundled plugin is handed anything.
        assert!(
            started
                .iter()
                .filter(|spec| spec.name != "preview")
                .all(|spec| spec.args.is_empty())
        );
    }

    #[test]
    fn the_first_hook_to_name_an_extension_decides_it_whatever_its_kind() {
        // An image hook for pdf comes first, so a later text hook for pdf never ran and must not
        // now; its other extension is still its own.
        let hooks = [
            hook(&["pdf"], HookKind::Image),
            hook(&["PDF", "doc"], HookKind::Text),
            hook(&["doc"], HookKind::Text),
        ];
        let args = hook_args(&hooks);
        assert_eq!(args.len(), 4, "{args:?}");
        assert_eq!(
            (args[0].as_str(), args[2].as_str()),
            ("--image-hook", "--text-hook")
        );
        let json: serde_json::Value = serde_json::from_str(&args[1]).expect("JSON");
        assert_eq!(json["extensions"], serde_json::json!(["pdf"]));
        let json: serde_json::Value = serde_json::from_str(&args[3]).expect("JSON");
        assert_eq!(json["extensions"], serde_json::json!(["doc"]));
        assert!(hook_args(&[]).is_empty());
    }

    #[test]
    fn a_missing_preview_plugin_is_worth_saying_unless_the_user_replaced_it() {
        let started = specs_found(&[], found, &[]);
        assert_eq!(missing_previewer(&[], &started), None, "it started");

        let without: Vec<PluginSpec> = started
            .iter()
            .filter(|spec| spec.name != "preview")
            .cloned()
            .collect();
        let notice = missing_previewer(&[], &without).expect("it did not start");
        assert!(notice.contains("cargo install --path plugins/preview"));

        // Their own previewer, by name or by the role, stands in for it: nothing to say.
        assert_eq!(
            missing_previewer(&[configured("preview", &[])], &without),
            None
        );
        assert_eq!(
            missing_previewer(&[configured("bat-preview", &["preview"])], &without),
            None
        );
    }
}
