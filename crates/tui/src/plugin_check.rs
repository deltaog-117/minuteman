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

//! `mman plugin check <name or folder>`: the plugin menu's "Start and test" for a shell. The
//! target is a folder holding a `plugin.toml`, or the name of a plugin the menu lists. A folder
//! (or a drop-in's name) is also held against what is already taken: its manifest's `api_version`,
//! and its keys and `:` words against the interface and the other plugins. A plugin from
//! `config.toml` or a bundled one is only started, as its keys were settled when it was loaded.

use std::path::Path;

use plugin_host::probe;
use theming::dropin::{MANIFEST, ManifestError};
use theming::{Config, DropIn, PluginSpec, RawPluginSpec};

use crate::bundled;
use crate::plugin_roster::{self, ConfigSwitches, Entry};

/// What is being checked, and what can be known about it before it starts.
struct Target {
    spec: PluginSpec,
    /// The manifest, when the target came from a folder: it holds the raw keys and the version.
    manifest: Option<DropIn>,
}

/// Checks `target` and reports in a few lines. `Ok` when it looks healthy, `Err` with the same
/// kind of report when it does not.
///
/// # Errors
///
/// The report, when the target is unknown, its manifest is not accepted, its version or keys do
/// not fit, or the program ends or complains at start.
pub fn run(target: &str, config: &Config, cwd: &Path) -> Result<String, String> {
    run_for(target, config, cwd, probe::WINDOW)
}

fn run_for(
    target: &str,
    config: &Config,
    cwd: &Path,
    window: std::time::Duration,
) -> Result<String, String> {
    let roster = roster(config);
    let Target { spec, manifest } = resolve(target, &roster)?;
    let name = spec.name.clone();

    let mut problems = Vec::new();
    if let Some(found) = &manifest {
        if let Some(why) = plugin_roster::api_mismatch(found.api_version) {
            // Starting it would only show the plugin refusing the same thing.
            return Err(format!("{name}: {why}"));
        }
        let others: Vec<Entry> = roster
            .iter()
            .filter(|entry| entry.folder.as_deref() != Some(found.dir.as_path()))
            .cloned()
            .collect();
        if let Some(why) = plugin_roster::conflict(&others, &found.spec, &config.keys) {
            problems.push(format!("{name}: {why}"));
        }
    }

    match probe::probe(&name, &spec.command, &spec.args, cwd, window) {
        Ok(report) => match report.verdict(&name) {
            Ok(line) if problems.is_empty() => Ok(line),
            Ok(line) => {
                problems.push(line);
                Err(problems.join("\n"))
            }
            Err(why) => {
                problems.push(why);
                Err(problems.join("\n"))
            }
        },
        Err(e) => {
            problems.push(format!("could not start {name}: {e}"));
            Err(problems.join("\n"))
        }
    }
}

/// What the plugin menu would list with nothing switched on or off by hand.
fn roster(config: &Config) -> Vec<Entry> {
    let listing = bundled::listing(&config.plugins, &config.preview_hooks);
    let entries = plugin_roster::build(
        &config.plugins,
        listing,
        ConfigSwitches {
            git_status: config.git_status,
            system_hud: config.system_hud,
        },
        &[],
        &[],
        &[],
    );
    plugin_roster::with_dropins(entries, &config.dropins, &[]).0
}

fn resolve(target: &str, roster: &[Entry]) -> Result<Target, String> {
    // A folder with no readable manifest is not a folder target; the name is tried next.
    match DropIn::load(Path::new(target)) {
        Ok(found) => {
            return Ok(Target {
                spec: PluginSpec::from(RawPluginSpec::clone(&found.spec)),
                manifest: Some(found),
            });
        }
        Err(ManifestError::Unreadable(_)) => {}
        Err(e) => return Err(format!("{target}: {e}")),
    }
    let entry = roster
        .iter()
        .find(|entry| entry.name == target)
        .ok_or_else(|| {
            format!("no plugin called {target} and no {MANIFEST} in a folder of that name")
        })?;
    let spec = entry.spec.clone().ok_or_else(|| {
        format!("{target}: its program is not installed, so there is nothing to start")
    })?;
    let manifest = entry
        .folder
        .as_deref()
        .map(DropIn::load)
        .transpose()
        .map_err(|e| format!("{target}: {e}"))?;
    Ok(Target { spec, manifest })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    /// A folder of its own under the system's temporary one, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "minuteman-plugin-check-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).expect("create scratch folder");
            Self(dir)
        }

        /// A plugin folder whose manifest is `manifest`.
        fn plugin(&self, folder: &str, manifest: &str) -> PathBuf {
            let dir = self.0.join(folder);
            fs::create_dir_all(&dir).expect("create plugin folder");
            fs::write(dir.join(MANIFEST), manifest).expect("write manifest");
            dir
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn config() -> Config {
        Config::from_sources(None, None, None)
    }

    fn check(dir: &Path) -> Result<String, String> {
        run_for(
            &dir.to_string_lossy(),
            &config(),
            Path::new("/"),
            Duration::from_millis(100),
        )
    }

    #[test]
    fn a_folder_whose_program_keeps_running_is_healthy() {
        let scratch = Scratch::new();
        let dir = scratch.plugin(
            "calm",
            "command = 'sh'\nargs = ['-c', 'sleep 30']\napi_version = 1",
        );
        assert_eq!(check(&dir), Ok("calm starts and keeps running".to_string()));
    }

    #[test]
    fn another_api_version_is_refused_without_starting_anything() {
        let scratch = Scratch::new();
        // The program would fail the check if it were started: it does not exist.
        let dir = scratch.plugin("old", "command = '/no/such/program'\napi_version = 999");
        let report = check(&dir).unwrap_err();
        assert!(
            report.contains("old: written for plugin API 999"),
            "{report}"
        );
        assert!(!report.contains("could not start"), "{report}");
    }

    #[test]
    fn a_program_that_ends_at_start_says_how() {
        let scratch = Scratch::new();
        let dir = scratch.plugin("brief", "command = 'sh'\nargs = ['-c', 'exit 4']");
        assert_eq!(
            check(&dir),
            Err("brief ended at start: exit status: 4".to_string())
        );
    }

    #[test]
    fn a_key_the_interface_already_binds_is_reported_with_the_start_result() {
        let scratch = Scratch::new();
        let dir = scratch.plugin(
            "greedy",
            "command = 'sh'\nargs = ['-c', 'sleep 30']\non_key = 'j'",
        );
        let report = check(&dir).unwrap_err();
        assert!(
            report.contains("greedy: key j is already bound"),
            "{report}"
        );
        assert!(
            report.contains("greedy starts and keeps running"),
            "{report}"
        );
    }

    #[test]
    fn a_program_that_is_missing_is_a_could_not_start() {
        let scratch = Scratch::new();
        let dir = scratch.plugin("ghost", "command = '/no/such/program-minuteman'");
        assert!(
            check(&dir)
                .unwrap_err()
                .starts_with("could not start ghost")
        );
    }

    #[test]
    fn a_bad_manifest_and_an_unknown_name_are_named() {
        let scratch = Scratch::new();
        let dir = scratch.plugin("bad", "command = ");
        assert!(
            check(&dir)
                .unwrap_err()
                .contains("plugin.toml is not valid")
        );
        let unknown = run_for(
            "no-such-plugin-minuteman",
            &config(),
            Path::new("/"),
            Duration::from_millis(100),
        );
        assert!(unknown.unwrap_err().starts_with("no plugin called"));
    }

    #[test]
    fn a_folder_is_not_in_conflict_with_itself_when_it_is_also_a_drop_in() {
        let scratch = Scratch::new();
        let dir = scratch.plugin("same", "command = 'sh'\nargs = ['-c', 'sleep 30']");
        let mut config = config();
        config.dropins = vec![DropIn::load(&dir).unwrap()];
        let report = run_for(
            &dir.to_string_lossy(),
            &config,
            Path::new("/"),
            Duration::from_millis(100),
        );
        assert_eq!(report, Ok("same starts and keeps running".to_string()));
    }
}
