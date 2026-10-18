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

//! The plugins the plugin menu lists, and whether each can be switched on and off. Pure: no
//! process and no file, so the rules for what the menu may do are tested without either; `app`
//! starts and stops the processes and `main` draws the rows.
//!
//! A plugin is either bundled (it ships with Minuteman and needs no config) or the user's own
//! (a `[[plugin]]` table). Both can be switched off. Two things cannot be switched from the menu,
//! and the entry says why instead of pretending: a bundled plugin whose program is not installed,
//! and git while `config.toml` says `git_status = false`, since the menu writes only `local.toml`
//! and a hand-written veto must keep winning.
//!
//! Most plugins start on, so `local.toml` keeps the names switched off. The system figures start
//! off unless `system_hud = true`, so `local.toml` keeps the names switched on for them; which
//! list a plugin belongs to is its `default_on`.

use theming::PluginSpec;

use crate::bundled::Listed;

/// Where a plugin comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Bundled,
    Yours,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Bundled => "bundled",
            Origin::Yours => "yours",
        }
    }
}

/// One line of the menu.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub origin: Origin,
    /// What to start when it is switched on; `None` for a bundled plugin that is not installed.
    pub spec: Option<PluginSpec>,
    /// The actions it answers, so a refused action can say the plugin is switched off.
    pub provides: Vec<String>,
    /// Why the menu cannot switch it, when it cannot.
    pub locked: Option<String>,
    pub enabled: bool,
    /// Whether it starts on when the menu has not touched it; decides which list of `local.toml`
    /// remembers a switch.
    pub default_on: bool,
}

impl Entry {
    pub fn can_switch(&self) -> bool {
        self.locked.is_none()
    }
}

/// What the config decides about the two bundled plugins it has a setting for.
#[derive(Debug, Clone, Copy)]
pub struct ConfigSwitches {
    /// `false` is a veto: git never runs and the menu cannot switch it on.
    pub git_status: bool,
    /// Whether the system figures start on; the menu can still switch them either way.
    pub system_hud: bool,
}

/// Every plugin the menu lists: the user's `[[plugin]]` tables in order, then the bundled ones
/// they have not replaced. `disabled` and `enabled` are the lists `local.toml` keeps.
pub fn build(
    configured: &[PluginSpec],
    bundled: Vec<Listed>,
    switches: ConfigSwitches,
    disabled: &[String],
    enabled: &[String],
) -> Vec<Entry> {
    let yours = configured.iter().map(|spec| Entry {
        name: spec.name.clone(),
        origin: Origin::Yours,
        provides: spec.provides.clone(),
        spec: Some(spec.clone()),
        locked: None,
        enabled: !disabled.contains(&spec.name),
        default_on: true,
    });
    let shipped = bundled.into_iter().map(|listed| {
        let locked = locked_reason(&listed, switches);
        let default_on = listed.name != "hud" || switches.system_hud;
        let named = |list: &[String]| list.iter().any(|name| name == listed.name);
        Entry {
            name: listed.name.to_string(),
            origin: Origin::Bundled,
            provides: listed.provides.iter().map(ToString::to_string).collect(),
            enabled: locked.is_none()
                && if default_on {
                    !named(disabled)
                } else {
                    named(enabled)
                },
            default_on,
            spec: listed.spec,
            locked,
        }
    });
    yours.chain(shipped).collect()
}

fn locked_reason(listed: &Listed, switches: ConfigSwitches) -> Option<String> {
    match listed.name {
        "git" if !switches.git_status => Some("off: git_status = false in config.toml".into()),
        _ if listed.spec.is_none() => Some("not installed".into()),
        _ => None,
    }
}

/// The names to keep in `local.toml` after `name` is switched on or off: the old list without the
/// name, plus the name again when it is switched off. Sorted, so the file does not shuffle.
pub fn disabled_after(disabled: &[String], name: &str, enabled: bool) -> Vec<String> {
    kept_after(disabled, name, !enabled)
}

/// The names to keep as switched on after `name` is switched on or off, for a plugin that starts
/// off: the mirror of [`disabled_after`].
pub fn enabled_after(enabled_list: &[String], name: &str, enabled: bool) -> Vec<String> {
    kept_after(enabled_list, name, enabled)
}

/// `list` without `name`, plus `name` again when `keep`; sorted and without repeats.
fn kept_after(list: &[String], name: &str, keep: bool) -> Vec<String> {
    let mut next: Vec<String> = list
        .iter()
        .filter(|kept| kept.as_str() != name)
        .cloned()
        .collect();
    if keep {
        next.push(name.to_string());
    }
    next.sort();
    next.dedup();
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn spec(name: &str) -> PluginSpec {
        PluginSpec {
            name: name.to_string(),
            command: "true".to_string(),
            args: Vec::new(),
            on_keys: Vec::new(),
            commands: Vec::new(),
            events: Vec::new(),
            provides: Vec::new(),
        }
    }

    fn listed(name: &'static str, found: bool) -> Listed {
        Listed {
            name,
            spec: found.then(|| spec(name)),
            provides: &["find"],
        }
    }

    const ALL_ON: ConfigSwitches = ConfigSwitches {
        git_status: true,
        system_hud: true,
    };

    #[test]
    fn the_users_plugins_come_first_then_the_bundled_ones() {
        let entries = build(
            &[spec("mine")],
            vec![listed("finder", true)],
            ALL_ON,
            &[],
            &[],
        );
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["mine", "finder"]);
        assert_eq!(entries[0].origin, Origin::Yours);
        assert_eq!(entries[1].origin, Origin::Bundled);
        assert!(entries.iter().all(|e| e.enabled && e.can_switch()));
    }

    #[test]
    fn a_name_in_the_disabled_list_is_off_for_bundled_and_for_the_users_own() {
        let entries = build(
            &[spec("mine")],
            vec![listed("finder", true)],
            ALL_ON,
            &["mine".to_string(), "finder".to_string()],
            &[],
        );
        assert!(entries.iter().all(|e| !e.enabled && e.can_switch()));
    }

    #[test]
    fn a_bundled_plugin_that_is_not_installed_is_off_and_cannot_be_switched() {
        let entries = build(&[], vec![listed("finder", false)], ALL_ON, &[], &[]);
        assert!(!entries[0].enabled);
        assert_eq!(entries[0].locked.as_deref(), Some("not installed"));
        assert!(!entries[0].can_switch());
    }

    #[test]
    fn git_status_false_in_the_config_is_a_veto_the_menu_cannot_lift() {
        let off = ConfigSwitches {
            git_status: false,
            system_hud: false,
        };
        let entries = build(
            &[],
            vec![listed("git", true)],
            off,
            &[],
            &["git".to_string()],
        );
        assert!(!entries[0].enabled && !entries[0].can_switch());
        assert!(entries[0].locked.as_deref().unwrap().contains("git_status"));

        let on = build(&[], vec![listed("git", true)], ALL_ON, &[], &[]);
        assert!(on[0].enabled && on[0].can_switch() && on[0].default_on);
    }

    #[test]
    fn the_system_figures_start_off_and_the_menu_can_switch_them_on() {
        let off = ConfigSwitches {
            git_status: true,
            system_hud: false,
        };
        let fresh = build(&[], vec![listed("hud", true)], off, &[], &[]);
        assert!(!fresh[0].enabled && fresh[0].can_switch() && !fresh[0].default_on);

        let chosen = build(
            &[],
            vec![listed("hud", true)],
            off,
            &[],
            &["hud".to_string()],
        );
        assert!(chosen[0].enabled);

        // Listing it in the wrong list changes nothing: only the default's own list counts.
        let stray = build(
            &[],
            vec![listed("hud", true)],
            off,
            &["hud".to_string()],
            &[],
        );
        assert!(!stray[0].enabled);
    }

    #[test]
    fn system_hud_true_starts_them_on_and_the_menu_can_switch_them_off() {
        let on = build(&[], vec![listed("hud", true)], ALL_ON, &[], &[]);
        assert!(on[0].enabled && on[0].default_on);
        let off = build(
            &[],
            vec![listed("hud", true)],
            ALL_ON,
            &["hud".to_string()],
            &[],
        );
        assert!(!off[0].enabled && off[0].can_switch());
    }

    #[test]
    fn switching_on_adds_the_name_for_a_plugin_that_starts_off() {
        let none: Vec<String> = Vec::new();
        let on = enabled_after(&none, "hud", true);
        assert_eq!(on, ["hud"]);
        assert_eq!(enabled_after(&on, "hud", true), ["hud"]);
        assert!(enabled_after(&on, "hud", false).is_empty());
    }

    #[test]
    fn switching_off_adds_the_name_once_and_switching_on_takes_it_out() {
        let none: Vec<String> = Vec::new();
        let off = disabled_after(&none, "git", false);
        assert_eq!(off, ["git"]);
        assert_eq!(disabled_after(&off, "git", false), ["git"]);
        assert_eq!(disabled_after(&off, "finder", false), ["finder", "git"]);
        assert!(disabled_after(&off, "git", true).is_empty());
    }

    proptest! {
        #[test]
        fn the_saved_list_is_sorted_has_no_repeats_and_reflects_the_last_switch(
            start in proptest::collection::vec("[a-c]{1,2}", 0..6),
            name in "[a-c]{1,2}",
            enabled in any::<bool>(),
        ) {
            let next = disabled_after(&start, &name, enabled);
            prop_assert!(next.windows(2).all(|pair| pair[0] < pair[1]));
            prop_assert_eq!(next.contains(&name), !enabled);
            for kept in start.iter().filter(|kept| **kept != name) {
                prop_assert!(next.contains(kept));
            }
        }
    }
}
