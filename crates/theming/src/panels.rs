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

//! Which panels the browser draws and how many file columns it splits into — the `[panels]`
//! table of `config.toml`. Like `[ui]`'s `glyphs`, an unrecognised `columns` value falls back to
//! the default rather than failing to start.

use serde::{Deserialize, Serialize};

/// How many of the miller columns are drawn. `TwoPane` drops the left (parent-directory) column
/// and gives its width to `current`/`preview` instead of just blanking it, so it reads as the
/// column being removed rather than left empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnLayout {
    /// Parent | current | preview, 20/40/40 — the layout Minuteman has always drawn.
    #[default]
    ThreePane,
    /// Current | preview, 50/50 — for a narrower terminal or a simpler view.
    TwoPane,
}

impl ColumnLayout {
    /// Parses a config value (case-insensitive). `None` for anything unrecognised, so the caller
    /// can fall back rather than fail startup on a typo.
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "three" | "three_pane" => Some(Self::ThreePane),
            "two" | "two_pane" => Some(Self::TwoPane),
            _ => None,
        }
    }

    /// The other layout — what the settings popup cycles to.
    #[must_use]
    pub fn cycled(self) -> Self {
        match self {
            Self::ThreePane => Self::TwoPane,
            Self::TwoPane => Self::ThreePane,
        }
    }
}

/// Resolved `[panels]` settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelsConfig {
    pub columns: ColumnLayout,
    /// Whether the header row (breadcrumb, marks/clipboard pills) is drawn. Hiding it reclaims
    /// its row rather than leaving it blank.
    pub show_hud: bool,
    /// Whether the status bar's idle chrome (mode pill, selected-file details, key hints) is
    /// drawn. The row itself is never reclaimed and a prompt or a transient message is never
    /// suppressed by this — only the passive display while nothing else is happening.
    pub show_command_bar: bool,
}

impl Default for PanelsConfig {
    fn default() -> Self {
        Self {
            columns: ColumnLayout::default(),
            show_hud: true,
            show_command_bar: true,
        }
    }
}

impl PanelsConfig {
    /// Layers `raw` on top of `self`, field by field — for live, in-session edits (the settings
    /// popup, persisted to `local.toml`) rather than the `config.toml`/`appearance.toml` parse.
    /// A field left `None` in `raw` keeps `self`'s value; an unrecognised `columns` keeps it too,
    /// rather than falling back to the type's own default the way a fresh parse would.
    pub fn overlay_raw(&self, raw: &RawPanels) -> PanelsConfig {
        PanelsConfig {
            columns: raw
                .columns
                .as_deref()
                .and_then(ColumnLayout::parse)
                .unwrap_or(self.columns),
            show_hud: raw.show_hud.unwrap_or(self.show_hud),
            show_command_bar: raw.show_command_bar.unwrap_or(self.show_command_bar),
        }
    }
}

/// Deserialized `[panels]` config; every field optional, like the rest of the config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct RawPanels {
    pub columns: Option<String>,
    pub show_hud: Option<bool>,
    pub show_command_bar: Option<bool>,
}

impl RawPanels {
    /// `top` wins wherever it sets a field; otherwise `self` shows through — the same rule
    /// `RawTheme::overlay`/`RawUi::overlay` use for layering `appearance.toml` over `config.toml`.
    pub fn overlay(self, top: RawPanels) -> RawPanels {
        RawPanels {
            columns: top.columns.or(self.columns),
            show_hud: top.show_hud.or(self.show_hud),
            show_command_bar: top.show_command_bar.or(self.show_command_bar),
        }
    }
}

impl From<RawPanels> for PanelsConfig {
    fn from(raw: RawPanels) -> Self {
        Self {
            columns: raw
                .columns
                .as_deref()
                .and_then(ColumnLayout::parse)
                .unwrap_or_default(),
            show_hud: raw.show_hud.unwrap_or(true),
            show_command_bar: raw.show_command_bar.unwrap_or(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn columns_parse_case_insensitively() {
        assert_eq!(ColumnLayout::parse("two"), Some(ColumnLayout::TwoPane));
        assert_eq!(ColumnLayout::parse("TWO_PANE"), Some(ColumnLayout::TwoPane));
        assert_eq!(ColumnLayout::parse("three"), Some(ColumnLayout::ThreePane));
        assert_eq!(ColumnLayout::parse("nonsense"), None);
    }

    #[test]
    fn cycled_toggles_between_the_two_layouts() {
        assert_eq!(ColumnLayout::ThreePane.cycled(), ColumnLayout::TwoPane);
        assert_eq!(ColumnLayout::TwoPane.cycled(), ColumnLayout::ThreePane);
    }

    #[test]
    fn default_is_three_pane_shown_hud_and_shown_command_bar() {
        let config = PanelsConfig::default();
        assert_eq!(config.columns, ColumnLayout::ThreePane);
        assert!(config.show_hud);
        assert!(config.show_command_bar);
    }

    #[test]
    fn an_absent_table_resolves_to_the_default() {
        let config: PanelsConfig = RawPanels::default().into();
        assert_eq!(config, PanelsConfig::default());
    }

    #[test]
    fn a_typo_in_columns_falls_back_to_the_default_rather_than_failing() {
        let raw = RawPanels {
            columns: Some("thre".into()),
            ..Default::default()
        };
        let config: PanelsConfig = raw.into();
        assert_eq!(config.columns, ColumnLayout::ThreePane);
    }

    #[test]
    fn overlay_raw_layers_onto_an_existing_config_not_the_type_default() {
        let base = PanelsConfig {
            columns: ColumnLayout::TwoPane,
            show_hud: false,
            show_command_bar: true,
        };
        let layered = base.overlay_raw(&RawPanels {
            show_hud: Some(true),
            ..Default::default()
        });
        assert!(layered.show_hud);
        // Untouched fields keep the base's value, not `PanelsConfig::default()`'s.
        assert_eq!(layered.columns, ColumnLayout::TwoPane);
        assert!(layered.show_command_bar);

        assert_eq!(base.overlay_raw(&RawPanels::default()), base);
    }

    #[test]
    fn raw_overlay_lets_top_win_field_by_field() {
        let bottom = RawPanels {
            columns: Some("two".into()),
            show_hud: Some(false),
            show_command_bar: None,
        };
        let top = RawPanels {
            columns: None,
            show_hud: Some(true),
            show_command_bar: Some(false),
        };
        let merged = bottom.overlay(top);
        assert_eq!(merged.columns.as_deref(), Some("two"), "top left it unset");
        assert_eq!(merged.show_hud, Some(true), "top wins where it sets one");
        assert_eq!(merged.show_command_bar, Some(false));
    }

    proptest! {
        /// Whatever subset of the three keys is present, with whatever valid values, parsing
        /// never panics and every absent field resolves to exactly the documented default —
        /// the same guarantee `Config::from_sources` relies on for every other table.
        #[test]
        fn any_partial_raw_panels_resolves_without_panicking(
            columns in proptest::option::of("[a-z_]{0,12}"),
            show_hud in proptest::option::of(any::<bool>()),
            show_command_bar in proptest::option::of(any::<bool>()),
        ) {
            let raw = RawPanels { columns: columns.clone(), show_hud, show_command_bar };
            let config: PanelsConfig = raw.into();
            if let Some(name) = columns.as_deref().and_then(ColumnLayout::parse) {
                prop_assert_eq!(config.columns, name);
            } else {
                prop_assert_eq!(config.columns, ColumnLayout::default());
            }
            prop_assert_eq!(config.show_hud, show_hud.unwrap_or(true));
            prop_assert_eq!(config.show_command_bar, show_command_bar.unwrap_or(true));
        }
    }
}
