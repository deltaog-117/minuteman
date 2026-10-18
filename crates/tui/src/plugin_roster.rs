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
//! A third kind is the drop-in: a folder with a `plugin.toml` under the config directory (see
//! `theming::dropin`). Finding one starts nothing: it is listed off, and switching it on is the
//! user's say-so, kept in the same list as the system figures. The menu does not remove it; its
//! folder is the user's to delete.
//!
//! Most plugins start on, so `local.toml` keeps the names switched off. The system figures start
//! off unless `system_hud = true`, so `local.toml` keeps the names switched on for them; which
//! list a plugin belongs to is its `default_on`.

use std::path::PathBuf;

use theming::{DropIn, KeyMap, PluginSpec, RawPluginSpec, parse_key};

use crate::bundled::Listed;
use crate::command::BUILTIN_WORDS;

/// Where a plugin comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Bundled,
    Yours,
    DropIn,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Bundled => "bundled",
            Origin::Yours => "yours",
            Origin::DropIn => "drop-in",
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
    /// Whether `local.toml` holds the plugin's table, which makes it the one kind the menu can
    /// remove; a `[[plugin]]` in the hand-written `config.toml` is the user's to edit there.
    pub kept_by_menu: bool,
    /// The folder a drop-in plugin lives in.
    pub folder: Option<PathBuf>,
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
/// they have not replaced. `disabled` and `enabled` are the lists `local.toml` keeps, and
/// `kept_by_menu` the names of the `configured` plugins that `local.toml` holds.
pub fn build(
    configured: &[PluginSpec],
    bundled: Vec<Listed>,
    switches: ConfigSwitches,
    disabled: &[String],
    enabled: &[String],
    kept_by_menu: &[String],
) -> Vec<Entry> {
    let yours = configured.iter().map(|spec| Entry {
        name: spec.name.clone(),
        origin: Origin::Yours,
        provides: spec.provides.clone(),
        spec: Some(spec.clone()),
        locked: None,
        enabled: !disabled.contains(&spec.name),
        default_on: true,
        kept_by_menu: kept_by_menu.contains(&spec.name),
        folder: None,
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
            kept_by_menu: false,
            folder: None,
            spec: listed.spec,
            locked,
        }
    });
    yours.chain(shipped).collect()
}

/// `entries` with the drop-in plugins listed after the user's own, all switched off unless
/// `enabled` names them, and a line for each that was left out because its name is already taken
/// by `entries` (bundled ones included, installed or not) or by an earlier drop-in. The plugin
/// that was there first keeps the name, as the add form also decides.
pub fn with_dropins(
    mut entries: Vec<Entry>,
    dropins: &[DropIn],
    enabled: &[String],
) -> (Vec<Entry>, Vec<String>) {
    let at = entries
        .iter()
        .take_while(|entry| entry.origin == Origin::Yours)
        .count();
    let mut notices = Vec::new();
    let mut added = Vec::new();
    for dropin in dropins {
        let name = &dropin.spec.name;
        let taken = entries
            .iter()
            .chain(&added)
            .find(|entry| entry.name == *name);
        if let Some(taken) = taken {
            notices.push(format!(
                "drop-in {name}: skipped, a {} plugin has that name",
                taken.origin.label()
            ));
            continue;
        }
        let spec = PluginSpec::from(dropin.spec.clone());
        added.push(Entry {
            name: name.clone(),
            origin: Origin::DropIn,
            provides: spec.provides.clone(),
            spec: Some(spec),
            locked: None,
            enabled: enabled.contains(name),
            default_on: false,
            kept_by_menu: false,
            folder: Some(dropin.dir.clone()),
        });
    }
    entries.splice(at..at, added);
    (entries, notices)
}

fn locked_reason(listed: &Listed, switches: ConfigSwitches) -> Option<String> {
    match listed.name {
        "git" if !switches.git_status => Some("off: git_status = false in config.toml".into()),
        _ if listed.spec.is_none() => Some("not installed".into()),
        _ => None,
    }
}

/// Why `spec`, a plugin the add form describes, cannot join `entries`, or `None` if it can: its
/// name is taken (a bundled plugin's too, installed or not), one of its keys is already bound by
/// the interface or fires another plugin, or one of its `:` words is a built-in command or
/// another plugin's. Keys and words are refused rather than left to the first to claim them, as
/// a plugin that silently never fires is harder to find than a form that says no.
pub fn conflict(entries: &[Entry], spec: &RawPluginSpec, keys: &KeyMap) -> Option<String> {
    if let Some(taken) = entries.iter().find(|entry| entry.name == spec.name) {
        return Some(format!(
            "a {} plugin called {} already exists",
            taken.origin.label(),
            spec.name
        ));
    }
    for word in &spec.on_key {
        let Some(code) = parse_key(word) else {
            continue;
        };
        if let Some(action) = keys.resolve(code) {
            return Some(format!("key {word} is already bound to {action:?}"));
        }
        let user = entries.iter().find(|entry| {
            entry
                .spec
                .as_ref()
                .is_some_and(|s| s.on_keys.contains(&code))
        });
        if let Some(entry) = user {
            return Some(format!("key {word} already fires plugin {}", entry.name));
        }
    }
    for word in &spec.commands {
        if BUILTIN_WORDS.contains(&word.as_str()) {
            return Some(format!(":{word} is a built-in command"));
        }
        let user = entries.iter().find(|entry| {
            entry
                .spec
                .as_ref()
                .is_some_and(|s| s.commands.iter().any(|c| c == word))
        });
        if let Some(entry) = user {
            return Some(format!(":{word} already runs plugin {}", entry.name));
        }
    }
    None
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
            &[],
        );
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["mine", "finder"]);
        assert_eq!(entries[0].origin, Origin::Yours);
        assert_eq!(entries[1].origin, Origin::Bundled);
        assert!(entries.iter().all(|e| e.enabled && e.can_switch()));
    }

    #[test]
    fn only_the_plugins_local_toml_holds_are_kept_by_the_menu() {
        let entries = build(
            &[spec("hand"), spec("menu")],
            vec![listed("finder", true)],
            ALL_ON,
            &[],
            &[],
            &["menu".to_string(), "finder".to_string()],
        );
        let kept: Vec<bool> = entries.iter().map(|e| e.kept_by_menu).collect();
        assert_eq!(kept, [false, true, false]);
    }

    #[test]
    fn a_name_in_the_disabled_list_is_off_for_bundled_and_for_the_users_own() {
        let entries = build(
            &[spec("mine")],
            vec![listed("finder", true)],
            ALL_ON,
            &["mine".to_string(), "finder".to_string()],
            &[],
            &[],
        );
        assert!(entries.iter().all(|e| !e.enabled && e.can_switch()));
    }

    #[test]
    fn a_bundled_plugin_that_is_not_installed_is_off_and_cannot_be_switched() {
        let entries = build(&[], vec![listed("finder", false)], ALL_ON, &[], &[], &[]);
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
            &[],
        );
        assert!(!entries[0].enabled && !entries[0].can_switch());
        assert!(entries[0].locked.as_deref().unwrap().contains("git_status"));

        let on = build(&[], vec![listed("git", true)], ALL_ON, &[], &[], &[]);
        assert!(on[0].enabled && on[0].can_switch() && on[0].default_on);
    }

    #[test]
    fn the_system_figures_start_off_and_the_menu_can_switch_them_on() {
        let off = ConfigSwitches {
            git_status: true,
            system_hud: false,
        };
        let fresh = build(&[], vec![listed("hud", true)], off, &[], &[], &[]);
        assert!(!fresh[0].enabled && fresh[0].can_switch() && !fresh[0].default_on);

        let chosen = build(
            &[],
            vec![listed("hud", true)],
            off,
            &[],
            &["hud".to_string()],
            &[],
        );
        assert!(chosen[0].enabled);

        // Listing it in the wrong list changes nothing: only the default's own list counts.
        let stray = build(
            &[],
            vec![listed("hud", true)],
            off,
            &["hud".to_string()],
            &[],
            &[],
        );
        assert!(!stray[0].enabled);
    }

    fn dropin(name: &str) -> DropIn {
        DropIn::from_manifest(
            std::path::Path::new("/plugins").join(name).as_path(),
            "command = 'true'",
        )
        .unwrap()
    }

    #[test]
    fn a_drop_in_is_listed_after_the_users_own_and_starts_off() {
        let base = build(
            &[spec("mine")],
            vec![listed("finder", true)],
            ALL_ON,
            &[],
            &[],
            &[],
        );
        let (entries, notices) = with_dropins(base, &[dropin("extra")], &[]);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["mine", "extra", "finder"]);
        assert!(notices.is_empty());
        let extra = &entries[1];
        assert_eq!(extra.origin, Origin::DropIn);
        assert!(!extra.enabled && !extra.default_on && !extra.kept_by_menu);
        assert!(extra.can_switch());
        assert_eq!(extra.folder.as_deref(), Some("/plugins/extra".as_ref()));
    }

    #[test]
    fn a_drop_in_the_user_switched_on_starts_on() {
        let (entries, _) = with_dropins(Vec::new(), &[dropin("extra")], &["extra".to_string()]);
        assert!(entries[0].enabled);
    }

    #[test]
    fn a_drop_in_never_takes_a_name_that_is_taken() {
        let base = build(
            &[spec("mine")],
            vec![listed("finder", false)],
            ALL_ON,
            &[],
            &[],
            &[],
        );
        let found = [dropin("mine"), dropin("finder"), dropin("fresh")];
        let (entries, notices) = with_dropins(base, &found, &[]);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["mine", "fresh", "finder"]);
        assert_eq!(notices.len(), 2);
        assert!(notices[0].contains("mine") && notices[0].contains("yours"));
        assert!(notices[1].contains("finder") && notices[1].contains("bundled"));
    }

    #[test]
    fn system_hud_true_starts_them_on_and_the_menu_can_switch_them_off() {
        let on = build(&[], vec![listed("hud", true)], ALL_ON, &[], &[], &[]);
        assert!(on[0].enabled && on[0].default_on);
        let off = build(
            &[],
            vec![listed("hud", true)],
            ALL_ON,
            &["hud".to_string()],
            &[],
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

    fn raw(name: &str) -> RawPluginSpec {
        RawPluginSpec {
            name: name.to_string(),
            command: "true".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn a_new_plugin_conflicts_with_a_taken_name_a_bound_key_or_a_taken_command_word() {
        let keys = theming::Config::from_sources(None, None, None).keys;
        // A key the interface leaves unbound, whichever that is.
        let free = ('A'..='Z')
            .map(|c| c.to_string())
            .find(|c| keys.resolve(parse_key(c).unwrap()).is_none())
            .unwrap();
        let mut mine = spec("mine");
        mine.on_keys = vec![parse_key(&free).unwrap()];
        mine.commands = vec!["blame".to_string()];
        let entries = build(
            &[mine],
            vec![listed("finder", false)],
            ALL_ON,
            &[],
            &[],
            &[],
        );

        assert_eq!(conflict(&entries, &raw("fresh"), &keys), None);
        for taken in ["mine", "finder"] {
            let said = conflict(&entries, &raw(taken), &keys).unwrap();
            assert!(said.contains("already exists"), "{said}");
        }
        let mut wants_key = raw("a");
        wants_key.on_key = vec![free];
        assert!(
            conflict(&entries, &wants_key, &keys)
                .unwrap()
                .contains("fires plugin mine")
        );
        wants_key.on_key = vec!["j".to_string()];
        assert!(
            conflict(&entries, &wants_key, &keys)
                .unwrap()
                .contains("bound to MoveDown")
        );
        let mut wants_word = raw("a");
        wants_word.commands = vec!["blame".to_string()];
        assert!(
            conflict(&entries, &wants_word, &keys)
                .unwrap()
                .contains("runs plugin mine")
        );
        wants_word.commands = vec![BUILTIN_WORDS[0].to_string()];
        assert!(
            conflict(&entries, &wants_word, &keys)
                .unwrap()
                .contains("built-in")
        );
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
