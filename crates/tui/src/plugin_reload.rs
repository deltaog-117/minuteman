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

//! `:plugins reload`: what to do to the running plugins so that they are the ones a fresh launch
//! of the edited config would start. Only the arithmetic lives here; starting and stopping is the
//! `App`'s.

use theming::PluginSpec;

/// The changes that bring `running` to `wanted`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Running now, not wanted any more.
    pub stop: Vec<String>,
    /// Wanted, not running now.
    pub start: Vec<PluginSpec>,
    /// Running and wanted, but described differently: stopped and started again.
    pub restart: Vec<PluginSpec>,
}

/// What it takes to turn the plugins of `running` into those of `wanted`. A name listed twice
/// counts once, at its first entry, as a launch would not run it twice either.
pub fn plan(running: &[PluginSpec], wanted: &[PluginSpec]) -> Plan {
    let mut plan = Plan::default();
    let mut seen: Vec<&str> = Vec::new();
    for spec in wanted {
        if seen.contains(&spec.name.as_str()) {
            continue;
        }
        seen.push(&spec.name);
        match running.iter().find(|now| now.name == spec.name) {
            None => plan.start.push(spec.clone()),
            Some(now) if now != spec => plan.restart.push(spec.clone()),
            Some(_) => {}
        }
    }
    for now in running {
        if !seen.contains(&now.name.as_str()) && !plan.stop.contains(&now.name) {
            plan.stop.push(now.name.clone());
        }
    }
    plan
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.stop.is_empty() && self.start.is_empty() && self.restart.is_empty()
    }

    /// One line for the status bar, e.g. `started notes; stopped sync`. `failed` holds what could
    /// not be started, as `name: reason`.
    pub fn summary(&self, failed: &[String]) -> String {
        let names = |specs: &[PluginSpec]| {
            specs
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        if self.is_empty() && failed.is_empty() {
            return "plugins reloaded: nothing changed".into();
        }
        let mut parts = Vec::new();
        if !self.start.is_empty() {
            parts.push(format!("started {}", names(&self.start)));
        }
        if !self.stop.is_empty() {
            parts.push(format!("stopped {}", self.stop.join(", ")));
        }
        if !self.restart.is_empty() {
            parts.push(format!("restarted {}", names(&self.restart)));
        }
        if !failed.is_empty() {
            parts.push(format!("could not start {}", failed.join("; ")));
        }
        format!("plugins reloaded: {}", parts.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn spec(name: &str, command: &str) -> PluginSpec {
        PluginSpec {
            name: name.into(),
            command: command.into(),
            args: Vec::new(),
            on_keys: Vec::new(),
            commands: Vec::new(),
            events: Vec::new(),
            provides: Vec::new(),
        }
    }

    /// Carries out `plan` on `running` the way the `App` does: stop, restart in place, start last.
    fn apply(running: &[PluginSpec], plan: &Plan) -> Vec<PluginSpec> {
        let mut now: Vec<PluginSpec> = running
            .iter()
            .filter(|spec| !plan.stop.contains(&spec.name))
            .cloned()
            .collect();
        for changed in &plan.restart {
            if let Some(slot) = now.iter_mut().find(|spec| spec.name == changed.name) {
                *slot = changed.clone();
            }
        }
        now.extend(plan.start.iter().cloned());
        now
    }

    fn sorted(mut specs: Vec<PluginSpec>) -> Vec<PluginSpec> {
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    fn lists() -> impl Strategy<Value = Vec<PluginSpec>> {
        // Few names and commands, so lists overlap, differ and repeat a name often.
        prop::collection::vec(("[abcd]", "[xy]"), 0..8)
            .prop_map(|pairs| pairs.iter().map(|(n, c)| spec(n, c)).collect())
    }

    proptest! {
        #[test]
        fn applying_the_plan_gives_exactly_the_wanted_plugins(
            mut running in lists(),
            wanted in lists(),
        ) {
            // What runs never has a name twice.
            let mut names = Vec::new();
            running.retain(|spec| {
                let fresh = !names.contains(&spec.name);
                names.push(spec.name.clone());
                fresh
            });
            let mut expected = Vec::new();
            for spec in &wanted {
                if !expected.iter().any(|kept: &PluginSpec| kept.name == spec.name) {
                    expected.push(spec.clone());
                }
            }
            let after = apply(&running, &plan(&running, &wanted));
            prop_assert_eq!(sorted(after), sorted(expected));
        }

        #[test]
        fn planning_from_what_is_wanted_changes_nothing(wanted in lists()) {
            let mut unique = Vec::new();
            for spec in wanted {
                if !unique.iter().any(|kept: &PluginSpec| kept.name == spec.name) {
                    unique.push(spec);
                }
            }
            prop_assert!(plan(&unique, &unique).is_empty());
        }
    }

    #[test]
    fn a_changed_command_restarts_and_a_missing_name_stops() {
        let running = [spec("a", "x"), spec("b", "x")];
        let wanted = [spec("a", "y"), spec("c", "x")];
        let plan = plan(&running, &wanted);
        assert_eq!(plan.restart, [spec("a", "y")]);
        assert_eq!(plan.stop, ["b"]);
        assert_eq!(plan.start, [spec("c", "x")]);
    }

    #[test]
    fn the_summary_names_each_change_and_each_failure() {
        let plan = Plan {
            stop: vec!["old".into()],
            start: vec![spec("new", "x")],
            restart: vec![spec("git", "y")],
        };
        assert_eq!(
            plan.summary(&["new: not found".into()]),
            "plugins reloaded: started new; stopped old; restarted git; could not start new: not found"
        );
        assert_eq!(
            Plan::default().summary(&[]),
            "plugins reloaded: nothing changed"
        );
    }
}
