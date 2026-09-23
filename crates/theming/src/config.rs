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

use std::io;
use std::path::PathBuf;

use crossterm::event::KeyCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::appearance::{Font, RawFont, RawStyles, Styles};
use crate::keymap::{KeyMap, RawKeyMap, parse_key};
use crate::panels::{PanelsConfig, RawPanels};
use crate::theme::{RawTheme, Theme};
use crate::ui::{RawUi, Ui};

/// One entry of the context menu's "Open with" submenu: a label and the command line that opens
/// a file. `command` is run under `sh -c`; a `{}` in it stands for the file's quoted path, and
/// without one the path is appended.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OpenWith {
    pub name: String,
    pub command: String,
}

/// One `[[plugin]]` table: an external process Minuteman spawns at startup and fires on
/// `on_key`, speaking the `plugins` crate's line-delimited JSON-RPC protocol over its own stdio.
/// Any language that can read a line and print one works — no compile step, no per-language host
/// bindings.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawPluginSpec {
    name: String,
    command: String,
    args: Vec<String>,
    on_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PluginSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// The key that fires this plugin, already resolved from its config string the same way
    /// every other keybinding is (see `keymap::parse_key`) — `None` for an absent or
    /// unrecognised string, the same "skip rather than crash" rule every other key field follows.
    pub on_key: Option<KeyCode>,
}

impl From<RawPluginSpec> for PluginSpec {
    fn from(raw: RawPluginSpec) -> Self {
        Self {
            name: raw.name,
            command: raw.command,
            args: raw.args,
            on_key: raw.on_key.as_deref().and_then(parse_key),
        }
    }
}

/// `config.toml`. Its `[theme]` and `[ui]` predate `appearance.toml` and are still honored, but
/// anything `appearance.toml` sets wins.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct RawConfig {
    /// `None` when absent, so "unset" can default to on rather than serde's `false`.
    alt_tap: Option<bool>,
    /// `None` when absent, for the same reason as `alt_tap`.
    browser_mouse: Option<bool>,
    /// `None` when absent, for the same reason as `alt_tap`.
    git_status: Option<bool>,
    /// `None` when absent, so "unset" can default to hidden rather than serde's `false` meaning
    /// the same thing by accident.
    show_hidden: Option<bool>,
    /// `None` when absent, so "unset" means the built-in list while an explicit `[]` means none.
    interactive_commands: Option<Vec<String>>,
    /// The `[[open_with]]` tables, in the order the submenu lists them.
    open_with: Vec<OpenWith>,
    /// The `[[plugin]]` tables, in the order they were spawned.
    plugin: Vec<RawPluginSpec>,
    keys: RawKeyMap,
    theme: RawTheme,
    ui: RawUi,
    panels: RawPanels,
}

/// Programs that draw on the whole terminal or read the keyboard, so a `:` command that names
/// one needs the real terminal rather than a captured pipe. Editors, pagers, monitors, a media
/// player, a remote shell and a multiplexer; anything else can still be forced with `:!`.
const INTERACTIVE_COMMANDS: &[&str] = &[
    "nvim", "vim", "vi", "nano", "emacs", "micro", "hx", "less", "more", "man", "htop", "btop",
    "top", "mpv", "ssh", "tmux", "fzf",
];

fn default_interactive_commands() -> Vec<String> {
    INTERACTIVE_COMMANDS
        .iter()
        .map(|&name| name.into())
        .collect()
}

/// `appearance.toml`: the whole look in one file.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawAppearance {
    pub(crate) theme: RawTheme,
    pub(crate) ui: RawUi,
    pub(crate) style: RawStyles,
    pub(crate) font: RawFont,
}

/// `local.toml`: what the settings and appearance popups persist. Unlike `config.toml`/
/// `appearance.toml`, nothing but Minuteman itself ever writes this file, so it is always sparse
/// — only the fields some popup actually changed are ever present — and a plain `toml::to_string`
/// re-serialize (which would lose comments in a hand-edited file) is exactly the right tool for
/// it. Layered highest of the three, so a saved pick always wins over the hand-edited files until
/// it is changed again from inside the app.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct RawLocal {
    pub theme: RawTheme,
    pub ui: RawUi,
    pub panels: RawPanels,
    /// Themes saved from the appearance popup's "Save theme" row, each a full color snapshot
    /// (see [`RawTheme::from_theme`]) rather than an overlay — a saved theme must still look the
    /// same after `theme`/`config.toml` change around it.
    pub custom_themes: Vec<CustomTheme>,
    /// The name of `custom_themes` entry `theme` currently matches (or started from, before it
    /// drifted) — how the appearance popup tells "update this theme" from "this is now a
    /// different theme" when "Save theme" is used again. `None` once `Reset` clears it, or if
    /// `theme` was never saved as a custom theme this session.
    pub active_custom_theme: Option<String>,
}

/// One theme a user named and saved from the appearance popup. `name` is unique within
/// `RawLocal::custom_themes` — the popup upserts by name rather than allowing duplicates.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CustomTheme {
    pub name: String,
    pub theme: RawTheme,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Whether tapping `Alt` on its own switches between the mini-shell and the file browser.
    /// Needs the terminal's keyboard protocol, which reports every key differently (see the
    /// note in `config.example.toml`), so it can be turned off.
    pub alt_tap: bool,
    /// Whether clicks and the wheel act on the three file columns. A switch rather than a
    /// constant so a misbehaving terminal or a habit of clicking by accident can turn it off
    /// without a rebuild; the mini-shell's own mouse gestures don't depend on it.
    pub browser_mouse: bool,
    /// Whether the status bar shows the branch and the selection's git state. It runs the `git`
    /// binary in the background, so it is a switch for anyone who would rather it never did.
    pub git_status: bool,
    /// Whether dot-prefixed entries start out visible; the `hidden` key flips it at runtime.
    pub show_hidden: bool,
    /// Program names a `:` command hands the whole terminal to (`:nvim notes.md`), instead of
    /// running with its output captured. `:!cmd` does the same for any one command.
    pub interactive_commands: Vec<String>,
    /// What the context menu's "Open with" lists. Empty means the menu offers `$VISUAL` or
    /// `$EDITOR` alone.
    pub open_with: Vec<OpenWith>,
    /// The `[[plugin]]` tables, in the order they were spawned — see `plugins::PluginManager`.
    pub plugins: Vec<PluginSpec>,
    pub keys: KeyMap,
    pub theme: Theme,
    /// Whether `[theme]` (a `name`, or even a single overridden field) was actually set in any of
    /// the three config files. `false` is what tells `main` it's safe to replace `theme` with a
    /// live, terminal-background-adapted pick (`Theme::auto`) instead of the static default it
    /// already carries as a fallback — any explicit customization, however small (including one
    /// ever saved from the appearance popup), is left alone.
    pub theme_is_customized: bool,
    /// `local.toml`'s own `[theme]` table, unmerged with `config.toml`/`appearance.toml` — `main`
    /// seeds the appearance popup's live overrides from this, not from `theme`, so a save only
    /// ever writes back what a popup actually changed and never masks a later hand-edit to
    /// `appearance.toml`.
    pub local_theme: RawTheme,
    pub ui: Ui,
    /// `local.toml`'s own `[ui]` table — see `local_theme`.
    pub local_ui: RawUi,
    /// Bold, italic, ... per interface element.
    pub styles: Styles,
    /// The font `init-terminal` prints; not something the TUI itself can apply.
    pub font: Font,
    /// How many file columns are drawn and whether the header/status-bar chrome is shown. The
    /// settings popup (`Space` then `t`) edits this live, in memory, for the running session, and
    /// saves it to `local.toml`.
    pub panels: PanelsConfig,
    /// `local.toml`'s own `[panels]` table — see `local_theme`.
    pub local_panels: RawPanels,
    /// Themes saved from the appearance popup, in the order they were saved — see
    /// `RawLocal::custom_themes`.
    pub local_custom_themes: Vec<CustomTheme>,
    /// See `RawLocal::active_custom_theme`.
    pub local_active_custom_theme: Option<String>,
}

impl Config {
    /// Loads `config.toml`, `appearance.toml` and `local.toml` from `~/.config/minuteman/` (XDG),
    /// falling back to built-in defaults for whatever is absent or fails to parse. A malformed
    /// file must never prevent the app from starting.
    pub fn load() -> Self {
        let read = |path: Option<PathBuf>| path.and_then(|p| std::fs::read_to_string(p).ok());
        Self::from_sources(
            read(Self::config_path()).as_deref(),
            read(Self::appearance_path()).as_deref(),
            read(Self::local_path()).as_deref(),
        )
    }

    /// Builds a config from the text of the three files (each `None` when absent). Split out from
    /// `load` so the layering can be tested without touching the filesystem.
    pub fn from_sources(
        config: Option<&str>,
        appearance: Option<&str>,
        local: Option<&str>,
    ) -> Self {
        let config: RawConfig = parse("config.toml", config);
        let appearance: RawAppearance = parse("appearance.toml", appearance);
        let local: RawLocal = parse("local.toml", local);

        let theme = config
            .theme
            .overlay(appearance.theme)
            .overlay(local.theme.clone());
        let theme_is_customized = theme != RawTheme::default();
        let panels: PanelsConfig = config.panels.overlay(local.panels.clone()).into();

        Self {
            alt_tap: config.alt_tap.unwrap_or(true),
            browser_mouse: config.browser_mouse.unwrap_or(true),
            git_status: config.git_status.unwrap_or(true),
            show_hidden: config.show_hidden.unwrap_or(false),
            interactive_commands: config
                .interactive_commands
                .unwrap_or_else(default_interactive_commands),
            open_with: config.open_with,
            plugins: config.plugin.into_iter().map(PluginSpec::from).collect(),
            keys: config.keys.into(),
            theme: theme.into(),
            theme_is_customized,
            local_theme: local.theme,
            ui: config
                .ui
                .overlay(appearance.ui)
                .overlay(local.ui.clone())
                .into(),
            local_ui: local.ui,
            styles: appearance.style.into(),
            font: appearance.font.into(),
            panels,
            local_panels: local.panels,
            local_custom_themes: local.custom_themes,
            local_active_custom_theme: local.active_custom_theme,
        }
    }

    /// Serializes `local` (the settings/appearance popups' combined live overrides) to
    /// `local.toml`, creating `~/.config/minuteman/` if it doesn't exist yet. The caller decides
    /// what to do with a failure (e.g. a read-only filesystem) — it must never be fatal, since a
    /// save is always in addition to the session already having applied the change in memory.
    pub fn save_local(local: &RawLocal) -> io::Result<()> {
        let dir = Self::config_dir()
            .ok_or_else(|| io::Error::other("could not determine the config directory"))?;
        std::fs::create_dir_all(&dir)?;
        let text = toml::to_string(local)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        std::fs::write(dir.join("local.toml"), text)
    }

    fn config_path() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join("config.toml"))
    }

    fn appearance_path() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join("appearance.toml"))
    }

    fn local_path() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join("local.toml"))
    }

    fn config_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "minuteman")
            .map(|dirs| dirs.config_dir().to_path_buf())
    }
}

/// Parses `text` as `T`, or returns `T::default()` — with a warning on stderr — if it is
/// malformed. Absent text is just the default.
fn parse<T: DeserializeOwned + Default>(file: &str, text: Option<&str>) -> T {
    match text.map(toml::from_str::<T>) {
        None => T::default(),
        Some(Ok(value)) => value,
        Some(Err(err)) => {
            eprintln!("minuteman: failed to parse {file}, using defaults: {err}");
            T::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::Mods;
    use crate::panels::PanelsConfig;
    use crate::ui::GlyphSet;

    #[test]
    fn partial_config_falls_back_per_missing_field() {
        let raw: RawConfig = toml::from_str("[keys]\nmove_down = [\"n\"]\n").unwrap();
        assert_eq!(raw.keys.move_down, vec!["n".to_string()]);
        // move_up was not specified, so it keeps the default.
        assert_eq!(raw.keys.move_up, vec!["k".to_string(), "up".to_string()]);
    }

    /// The shipped `config.example.toml` must always parse and, being nothing but the built-in
    /// default keys spelled out explicitly, resolve to exactly the default keymap — catches the
    /// example silently drifting out of sync with a future default change.
    #[test]
    fn shipped_example_config_parses_and_matches_defaults() {
        let text = include_str!("../../../config.example.toml");
        let raw: RawConfig = toml::from_str(text).unwrap();
        assert_eq!(raw.alt_tap, Some(true));
        assert_eq!(raw.browser_mouse, Some(true));
        assert_eq!(raw.git_status, Some(true));
        assert_eq!(raw.show_hidden, Some(false));
        assert_eq!(
            raw.interactive_commands,
            Some(default_interactive_commands())
        );
        let keys: KeyMap = raw.keys.into();
        let default_keys: KeyMap = RawKeyMap::default().into();
        assert_eq!(keys, default_keys);
        let panels: PanelsConfig = raw.panels.into();
        assert_eq!(panels, PanelsConfig::default());
    }

    #[test]
    fn interactive_commands_default_to_the_built_in_list_and_can_be_replaced() {
        let defaults = Config::from_sources(None, None, None).interactive_commands;
        assert!(defaults.iter().any(|name| name == "nvim"));
        assert_eq!(
            Config::from_sources(Some("[keys]\nquit = [\"x\"]\n"), None, None).interactive_commands,
            defaults
        );
        let custom = Config::from_sources(Some("interactive_commands = [\"kak\"]\n"), None, None);
        assert_eq!(custom.interactive_commands, vec!["kak".to_string()]);
        let none = Config::from_sources(Some("interactive_commands = []\n"), None, None);
        assert!(none.interactive_commands.is_empty());
    }

    #[test]
    fn alt_tap_defaults_on_and_can_be_switched_off() {
        assert!(Config::from_sources(None, None, None).alt_tap);
        assert!(Config::from_sources(Some("[keys]\nquit = [\"x\"]\n"), None, None).alt_tap);
        assert!(!Config::from_sources(Some("alt_tap = false\n"), None, None).alt_tap);
        assert!(Config::from_sources(Some("alt_tap = true\n"), None, None).alt_tap);
    }

    #[test]
    fn show_hidden_defaults_off_and_can_be_switched_on() {
        assert!(!Config::from_sources(None, None, None).show_hidden);
        assert!(!Config::from_sources(Some("[keys]\nquit = [\"x\"]\n"), None, None).show_hidden);
        assert!(Config::from_sources(Some("show_hidden = true\n"), None, None).show_hidden);
        assert!(!Config::from_sources(Some("show_hidden = false\n"), None, None).show_hidden);
    }

    #[test]
    fn git_status_defaults_on_and_can_be_switched_off() {
        assert!(Config::from_sources(None, None, None).git_status);
        assert!(Config::from_sources(Some("[keys]\nquit = [\"x\"]\n"), None, None).git_status);
        assert!(!Config::from_sources(Some("git_status = false\n"), None, None).git_status);
    }

    #[test]
    fn browser_mouse_defaults_on_and_can_be_switched_off() {
        assert!(Config::from_sources(None, None, None).browser_mouse);
        assert!(Config::from_sources(Some("[keys]\nquit = [\"x\"]\n"), None, None).browser_mouse);
        assert!(!Config::from_sources(Some("browser_mouse = false\n"), None, None).browser_mouse);
        assert!(Config::from_sources(Some("browser_mouse = true\n"), None, None).browser_mouse);
    }

    /// The same drift guard for `appearance.example.toml`, covering all four of its tables.
    #[test]
    fn shipped_appearance_example_parses_and_matches_defaults() {
        let text = include_str!("../../../appearance.example.toml");
        let config = Config::from_sources(None, Some(text), None);
        assert_eq!(config.theme, Theme::default());
        // The example spells out `name = "neon"` plus every field, so it counts as an explicit
        // pin — it must never get silently swapped for an auto-detected palette (see
        // `theme_is_customized_is_false_only_when_the_theme_table_is_entirely_absent`).
        assert!(config.theme_is_customized);
        assert_eq!(config.ui, Ui::default());
        assert_eq!(config.styles, Styles::default());
        assert_eq!(config.font, Font::default());
    }

    /// Every `[style]` element the code knows must be spelled out in the example, so a new
    /// element can't be added without documenting it.
    #[test]
    fn the_appearance_example_lists_every_style_element() {
        let text = include_str!("../../../appearance.example.toml");
        for element in crate::appearance::STYLE_ELEMENTS {
            assert!(
                text.lines()
                    .any(|l| l.starts_with(&format!("{element} = "))),
                "appearance.example.toml lacks `{element}`"
            );
        }
    }

    #[test]
    fn appearance_toml_carries_all_four_tables() {
        let config = Config::from_sources(
            None,
            Some(
                "[theme]\nname = \"classic\"\n[ui]\nglyphs = \"nerd\"\n\
                 [style]\ndoc = [\"italic\"]\n[font]\nfamily = \"Iosevka\"\nsize = 13.0\n",
            ),
            None,
        );
        assert_eq!(config.theme.selection_bg, "blue");
        assert_eq!(config.ui.glyphs, GlyphSet::Nerd);
        assert!(config.styles.doc.italic);
        assert_eq!(
            (config.font.family.as_str(), config.font.size),
            ("Iosevka", 13.0)
        );
    }

    #[test]
    fn a_theme_left_in_config_toml_still_works() {
        let config = Config::from_sources(Some("[theme]\nname = \"dracula\"\n"), None, None);
        // Dracula's magenta selection, not neon's violet.
        assert_eq!(config.theme.selection_bg, "magenta");
        assert_ne!(config.theme, Theme::default());
        assert!(config.theme_is_customized);
    }

    /// Even a single overridden field (no `name` at all) must count as customized — otherwise
    /// `main` would blow away that one field by wholesale-replacing `theme` with an auto-detected
    /// palette on top of it.
    #[test]
    fn theme_is_customized_is_false_only_when_the_theme_table_is_entirely_absent() {
        assert!(!Config::from_sources(None, None, None).theme_is_customized);
        assert!(
            !Config::from_sources(Some("[keys]\nquit = [\"x\"]\n"), None, None).theme_is_customized
        );
        assert!(
            Config::from_sources(Some("[theme]\nborder_fg = \"green\"\n"), None, None)
                .theme_is_customized
        );
        assert!(
            Config::from_sources(None, Some("[theme]\nname = \"nord\"\n"), None)
                .theme_is_customized
        );
        assert!(
            Config::from_sources(None, None, Some("[theme]\nname = \"nord\"\n"))
                .theme_is_customized,
            "a theme saved from the appearance popup counts as customized too"
        );
    }

    /// `local.toml` (what the popups save) is layered highest — above both hand-edited files —
    /// and `Config` also exposes its own `[theme]`/`[ui]`/`[panels]` tables unmerged, so `main`
    /// can seed a popup's live overrides without re-saving what came from a lower layer.
    #[test]
    fn local_toml_wins_over_both_hand_edited_files_but_is_also_exposed_unmerged() {
        let config = Config::from_sources(
            Some("[theme]\nname = \"dracula\"\n[panels]\ncolumns = \"two\"\n"),
            Some("[theme]\nborder_fg = \"red\"\n[ui]\nglyphs = \"ascii\"\n"),
            Some(
                "[theme]\nname = \"nord\"\naccent_fg = \"#123456\"\n[ui]\nglyphs = \"nerd\"\n\
                  [panels]\nshow_hud = false\n",
            ),
        );
        // Nord's border, not dracula's or a leftover appearance.toml override.
        assert_eq!(config.theme.border_focused_fg, "#88c0d0");
        assert_eq!(config.theme.accent_fg, "#123456");
        assert_eq!(config.ui.glyphs, GlyphSet::Nerd);
        assert_eq!(config.panels.columns, crate::panels::ColumnLayout::TwoPane);
        assert!(!config.panels.show_hud);

        // The raw local layer alone, not merged with config.toml's `columns = "two"`.
        assert_eq!(config.local_theme.name.as_deref(), Some("nord"));
        assert_eq!(config.local_theme.accent_fg.as_deref(), Some("#123456"));
        assert_eq!(config.local_ui.glyphs.as_deref(), Some("nerd"));
        assert_eq!(config.local_panels.columns, None);
        assert_eq!(config.local_panels.show_hud, Some(false));
    }

    /// `local.toml` is program-owned and rewritten wholesale on every save, so it only ever needs
    /// to carry what a popup actually touched — `toml`'s serializer already skips a `None` field
    /// entirely (no `skip_serializing_if` needed), which is what keeps that guarantee true.
    #[test]
    fn raw_local_serializes_only_the_fields_that_are_set() {
        let local = RawLocal {
            theme: RawTheme {
                name: Some("nord".into()),
                accent_fg: Some("#123456".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let text = toml::to_string(&local).unwrap();
        assert!(text.contains("name = \"nord\""));
        assert!(text.contains("accent_fg = \"#123456\""));
        assert!(!text.contains("border_fg"), "unset fields are omitted");
        assert!(
            !text.contains("glyphs") && !text.contains("columns") && !text.contains("show_hud"),
            "a table nothing was ever set on stays present but empty (`[ui]`/`[panels]` with no \
             keys) — harmless, since an empty table can never override a lower layer"
        );

        // Round-trips back to exactly the same value through `RawLocal`'s own `Deserialize`.
        let parsed: RawLocal = toml::from_str(&text).unwrap();
        assert_eq!(parsed, local);
    }

    #[test]
    fn custom_themes_and_the_active_pick_are_read_from_local_toml_unmerged() {
        let config = Config::from_sources(
            None,
            None,
            Some(
                "active_custom_theme = \"sunset\"\n\
                 [[custom_themes]]\nname = \"sunset\"\n[custom_themes.theme]\naccent_fg = \"#ff8800\"\n\
                 [[custom_themes]]\nname = \"midnight\"\n[custom_themes.theme]\naccent_fg = \"#3300aa\"\n",
            ),
        );
        assert_eq!(config.local_active_custom_theme.as_deref(), Some("sunset"));
        let names: Vec<&str> = config
            .local_custom_themes
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(names, ["sunset", "midnight"]);
        assert_eq!(
            config.local_custom_themes[0].theme.accent_fg.as_deref(),
            Some("#ff8800")
        );
        // Not merged into the running theme — saving a theme doesn't apply it on its own.
        assert_ne!(config.theme.accent_fg, "#ff8800");
    }

    #[test]
    fn a_custom_theme_saved_from_the_popup_round_trips_through_local_toml() {
        let local = RawLocal {
            active_custom_theme: Some("sunset".into()),
            custom_themes: vec![CustomTheme {
                name: "sunset".into(),
                theme: RawTheme::from_theme(&Theme::named("dracula")),
            }],
            ..Default::default()
        };
        let text = toml::to_string(&local).unwrap();
        let parsed: RawLocal = toml::from_str(&text).unwrap();
        assert_eq!(parsed, local);
        let resolved: Theme = parsed.custom_themes[0].theme.clone().into();
        assert_eq!(resolved, Theme::named("dracula"));
    }

    #[test]
    fn appearance_wins_over_a_legacy_theme_field_by_field() {
        let config = Config::from_sources(
            Some("[theme]\nname = \"dracula\"\nborder_fg = \"red\"\n[ui]\nglyphs = \"ascii\"\n"),
            Some("[theme]\nborder_fg = \"green\"\n"),
            None,
        );
        assert_eq!(
            config.theme.border_fg, "green",
            "appearance overrides config"
        );
        // Untouched by appearance.toml, so the config.toml palette shows through.
        assert_eq!(config.theme.selection_bg, "magenta");
        assert_eq!(config.ui.glyphs, GlyphSet::Ascii);
    }

    #[test]
    fn a_malformed_appearance_file_falls_back_to_defaults_without_touching_keys() {
        let config = Config::from_sources(
            Some("[keys]\nmove_down = [\"n\"]\n"),
            Some("this is [not valid toml"),
            None,
        );
        assert_eq!(config.theme, Theme::default());
        assert!(!config.theme_is_customized);
        assert_eq!(config.styles, Styles::default());
        let down: KeyMap = RawKeyMap {
            move_down: vec!["n".into()],
            ..RawKeyMap::default()
        }
        .into();
        assert_eq!(config.keys, down, "the good config.toml still applies");
    }

    #[test]
    fn no_files_at_all_is_the_built_in_look() {
        let config = Config::from_sources(None, None, None);
        assert_eq!(config.theme, Theme::default());
        assert!(
            !config.theme_is_customized,
            "nothing set [theme] at all, so main is free to auto-detect"
        );
        assert_eq!(config.styles.dir, Mods::parse(&["bold"]));
        assert_eq!(config.font, Font::default());
    }

    #[test]
    fn panels_default_to_three_pane_with_hud_and_command_bar_shown() {
        let config = Config::from_sources(None, None, None);
        assert_eq!(config.panels, PanelsConfig::default());
    }

    #[test]
    fn a_partial_panels_table_falls_back_per_missing_field() {
        let config = Config::from_sources(Some("[panels]\ncolumns = \"two\"\n"), None, None);
        assert_eq!(config.panels.columns, crate::panels::ColumnLayout::TwoPane);
        // show_hud/show_command_bar were not specified, so they keep their defaults.
        assert!(config.panels.show_hud);
        assert!(config.panels.show_command_bar);

        let config = Config::from_sources(Some("[panels]\nshow_hud = false\n"), None, None);
        assert_eq!(
            config.panels.columns,
            crate::panels::ColumnLayout::ThreePane
        );
        assert!(!config.panels.show_hud);
        assert!(config.panels.show_command_bar);
    }

    #[test]
    fn open_with_entries_keep_their_order_and_default_to_none() {
        let config = Config::from_sources(
            Some(
                "[[open_with]]\nname = \"Neovim\"\ncommand = \"nvim\"\n\
                 [[open_with]]\nname = \"VLC\"\ncommand = \"vlc {}\"\n",
            ),
            None,
            None,
        );
        let names: Vec<&str> = config.open_with.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["Neovim", "VLC"]);
        assert_eq!(config.open_with[1].command, "vlc {}");
        assert!(Config::from_sources(None, None, None).open_with.is_empty());
    }

    #[test]
    fn plugin_entries_keep_their_order_and_resolve_their_key_string() {
        let config = Config::from_sources(
            Some(
                "[[plugin]]\nname = \"git-blame\"\ncommand = \"python3\"\n\
                 args = [\"blame.py\"]\non_key = \"b\"\n\
                 [[plugin]]\nname = \"no-key\"\ncommand = \"true\"\n",
            ),
            None,
            None,
        );
        let names: Vec<&str> = config.plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["git-blame", "no-key"]);
        assert_eq!(config.plugins[0].command, "python3");
        assert_eq!(config.plugins[0].args, vec!["blame.py".to_string()]);
        assert_eq!(config.plugins[0].on_key, Some(KeyCode::Char('b')));
        assert_eq!(config.plugins[1].on_key, None);
        assert!(Config::from_sources(None, None, None).plugins.is_empty());
    }

    #[test]
    fn an_unrecognised_plugin_key_string_resolves_to_no_binding_rather_than_failing() {
        let config = Config::from_sources(
            Some("[[plugin]]\nname = \"x\"\ncommand = \"true\"\non_key = \"ctrl-nonsense\"\n"),
            None,
            None,
        );
        assert_eq!(config.plugins[0].on_key, None);
    }
}
