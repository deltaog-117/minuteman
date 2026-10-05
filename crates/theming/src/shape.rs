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

//! The shape of the interface, apart from its colors: the space between the panes and around the
//! screen's edge, where pane titles sit, and the shadow and dimming effects — the `[shape]` table
//! of `appearance.toml`.
//!
//! A shape is part of a [`Theme`](crate::Theme) (its `shape` field), so a saved theme brings its
//! shape back with its colors. The frame style itself (`border_type`) and the status bar's
//! `separator` predate this table and stay in `[theme]`, where the built-in palettes set them.
//!
//! A terminal draws whole cells, so there is no pixel rounding, blur or transparency: gaps are
//! counted in cells, and the effects are drawn with cell attributes.

use serde::{Deserialize, Serialize};

/// Every frame style `Theme::border_type` accepts, in the order the appearance popup cycles them.
/// An unrecognised name is drawn as `rounded`.
pub const BORDER_STYLES: [&str; 9] = [
    "rounded",
    "plain",
    "double",
    "thick",
    "dashed",
    "thick-dashed",
    "block",
    "block-inside",
    "none",
];

/// The widest gap allowed between two panes. Wider would leave the side columns too narrow to
/// read on an ordinary 80-column terminal.
pub const MAX_GAPS_IN: u8 = 4;
/// The widest gap allowed to the screen's edge, for the same reason.
pub const MAX_GAPS_OUT: u8 = 8;

/// A number of blank cells, never more than `MAX`. Built only through [`Gap::clamped`], so an
/// out-of-range value from a hand-edited file or a key held too long can't reach the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Gap<const MAX: u8>(u8);

impl<const MAX: u8> Gap<MAX> {
    /// `cells`, pulled into `0..=MAX`: a value outside the range is moved to the nearest end
    /// rather than rejected, so a typo in the file still gives a usable layout.
    pub fn clamped(cells: i64) -> Self {
        // In `0..=MAX` after the clamp, so it always fits a `u8`.
        Self(u8::try_from(cells.clamp(0, i64::from(MAX))).unwrap_or(MAX))
    }

    pub fn cells(self) -> u16 {
        u16::from(self.0)
    }

    /// `self` moved by `delta` cells, stopping at either end of the range.
    #[must_use]
    pub fn stepped(self, delta: i64) -> Self {
        Self::clamped(i64::from(self.0).saturating_add(delta))
    }
}

/// Where a pane's title sits on its top border.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TitleAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl TitleAlign {
    /// The names `parse` accepts, in the order the appearance popup cycles them.
    pub const NAMES: [&'static str; 3] = ["left", "center", "right"];

    /// Parses a config value (case-insensitive). `None` for anything unrecognised, so the caller
    /// can fall back rather than fail startup on a typo.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "left" => Some(Self::Left),
            "center" | "centre" => Some(Self::Center),
            "right" => Some(Self::Right),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

/// The resolved shape: every field set, every number in range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Shape {
    /// Blank cells between two side-by-side panes.
    pub gaps_in: Gap<MAX_GAPS_IN>,
    /// Blank cells between the interface and the left and right edges of the screen; half as
    /// many rows at the top and bottom, since a cell is about twice as tall as it is wide.
    pub gaps_out: Gap<MAX_GAPS_OUT>,
    pub title_align: TitleAlign,
    /// Whether floating panels (menus, popups, the shell box) cast a shadow on what is under
    /// them.
    pub shadow: bool,
    /// Whether the panes that don't have focus are drawn dimmed, so the active one stands out.
    pub dim_inactive: bool,
}

impl Shape {
    /// Layers `overrides` on top of `self`, field by field — the same shape as
    /// `Theme::overlay_raw`, for the appearance popup's live edits.
    #[must_use]
    pub fn overlay_raw(&self, overrides: &RawShape) -> Shape {
        Shape {
            gaps_in: overrides.gaps_in.map_or(self.gaps_in, Gap::clamped),
            gaps_out: overrides.gaps_out.map_or(self.gaps_out, Gap::clamped),
            title_align: overrides
                .title_align
                .as_deref()
                .and_then(TitleAlign::parse)
                .unwrap_or(self.title_align),
            shadow: overrides.shadow.unwrap_or(self.shadow),
            dim_inactive: overrides.dim_inactive.unwrap_or(self.dim_inactive),
        }
    }
}

/// `[shape]` as written in the file. Every field is optional, so a partial table changes only
/// what it mentions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct RawShape {
    /// Wider than the range on purpose: an out-of-range number is clamped, not a parse error.
    pub gaps_in: Option<i64>,
    pub gaps_out: Option<i64>,
    pub title_align: Option<String>,
    pub shadow: Option<bool>,
    pub dim_inactive: Option<bool>,
}

impl RawShape {
    /// `top` wins wherever it sets a field; otherwise `self` shows through.
    #[must_use]
    pub fn overlay(self, top: RawShape) -> RawShape {
        RawShape {
            gaps_in: top.gaps_in.or(self.gaps_in),
            gaps_out: top.gaps_out.or(self.gaps_out),
            title_align: top.title_align.or(self.title_align),
            shadow: top.shadow.or(self.shadow),
            dim_inactive: top.dim_inactive.or(self.dim_inactive),
        }
    }

    /// Whether no field is set — so `local.toml` can leave the table out entirely.
    pub fn is_empty(&self) -> bool {
        *self == RawShape::default()
    }

    /// Every field set from `shape`: what a saved theme stores, so it comes back exactly as it
    /// was saved whatever the files around it say later.
    pub fn from_shape(shape: &Shape) -> RawShape {
        RawShape {
            gaps_in: Some(i64::from(shape.gaps_in.cells())),
            gaps_out: Some(i64::from(shape.gaps_out.cells())),
            title_align: Some(shape.title_align.name().to_string()),
            shadow: Some(shape.shadow),
            dim_inactive: Some(shape.dim_inactive),
        }
    }
}

impl From<RawShape> for Shape {
    fn from(raw: RawShape) -> Self {
        Shape::default().overlay_raw(&raw)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn the_default_shape_is_the_look_minuteman_always_had() {
        let shape = Shape::default();
        assert_eq!((shape.gaps_in.cells(), shape.gaps_out.cells()), (0, 0));
        assert_eq!(shape.title_align, TitleAlign::Left);
        assert!(!shape.shadow && !shape.dim_inactive);
    }

    #[test]
    fn a_partial_table_changes_only_what_it_mentions() {
        let raw: RawShape = toml::from_str("gaps_in = 2\nshadow = true").expect("valid toml");
        let shape: Shape = raw.into();
        assert_eq!(shape.gaps_in.cells(), 2);
        assert!(shape.shadow);
        assert_eq!(shape.gaps_out, Shape::default().gaps_out);
        assert_eq!(shape.title_align, TitleAlign::Left);
    }

    #[test]
    fn out_of_range_gaps_and_unknown_alignments_fall_back_instead_of_failing() {
        let raw: RawShape = toml::from_str("gaps_in = -3\ngaps_out = 900\ntitle_align = \"up\"")
            .expect("numbers outside the range still parse");
        let shape: Shape = raw.into();
        assert_eq!(shape.gaps_in.cells(), 0);
        assert_eq!(shape.gaps_out.cells(), u16::from(MAX_GAPS_OUT));
        assert_eq!(shape.title_align, TitleAlign::Left);
    }

    #[test]
    fn title_align_names_parse_back_to_themselves() {
        for name in TitleAlign::NAMES {
            let align = TitleAlign::parse(name).expect("every listed name parses");
            assert_eq!(align.name(), name);
        }
        assert_eq!(TitleAlign::parse(" CENTRE "), Some(TitleAlign::Center));
    }

    #[test]
    fn an_empty_raw_shape_serializes_to_nothing() {
        assert!(RawShape::default().is_empty());
        assert_eq!(toml::to_string(&RawShape::default()).unwrap(), "");
    }

    fn any_shape() -> impl Strategy<Value = Shape> {
        (
            any::<i64>(),
            any::<i64>(),
            0..TitleAlign::NAMES.len(),
            any::<bool>(),
            any::<bool>(),
        )
            .prop_map(|(gaps_in, gaps_out, align, shadow, dim_inactive)| Shape {
                gaps_in: Gap::clamped(gaps_in),
                gaps_out: Gap::clamped(gaps_out),
                title_align: TitleAlign::parse(TitleAlign::NAMES[align])
                    .expect("listed names parse"),
                shadow,
                dim_inactive,
            })
    }

    fn any_raw() -> impl Strategy<Value = RawShape> {
        (
            proptest::option::of(any::<i64>()),
            proptest::option::of(any::<i64>()),
            proptest::option::of("[a-zA-Z]{0,8}"),
            proptest::option::of(any::<bool>()),
            proptest::option::of(any::<bool>()),
        )
            .prop_map(
                |(gaps_in, gaps_out, title_align, shadow, dim_inactive)| RawShape {
                    gaps_in,
                    gaps_out,
                    title_align,
                    shadow,
                    dim_inactive,
                },
            )
    }

    proptest! {
        #[test]
        fn a_gap_is_always_in_range_and_keeps_an_in_range_value(cells in any::<i64>()) {
            let gap = Gap::<MAX_GAPS_OUT>::clamped(cells);
            prop_assert!(gap.cells() <= u16::from(MAX_GAPS_OUT));
            if (0..=i64::from(MAX_GAPS_OUT)).contains(&cells) {
                prop_assert_eq!(i64::from(gap.cells()), cells);
            }
        }

        #[test]
        fn stepping_stays_in_range_and_undoes_itself_when_nothing_was_clamped(
            start in 0..=i64::from(MAX_GAPS_IN),
            delta in -10i64..=10,
        ) {
            let gap = Gap::<MAX_GAPS_IN>::clamped(start);
            let moved = gap.stepped(delta);
            prop_assert!(moved.cells() <= u16::from(MAX_GAPS_IN));
            if (0..=i64::from(MAX_GAPS_IN)).contains(&(start + delta)) {
                prop_assert_eq!(moved.stepped(-delta), gap);
            }
        }

        #[test]
        fn a_saved_shape_resolves_back_to_exactly_itself(shape in any_shape()) {
            let saved = RawShape::from_shape(&shape);
            prop_assert_eq!(Shape::from(saved.clone()), shape);
            // Whatever it is layered over: a saved shape sets every field.
            prop_assert_eq!(Shape { shadow: !shape.shadow, ..shape }.overlay_raw(&saved), shape);
        }

        #[test]
        fn the_top_table_wins_per_field(base in any_raw(), top in any_raw()) {
            let merged = base.clone().overlay(top.clone());
            prop_assert_eq!(merged.gaps_in, top.gaps_in.or(base.gaps_in));
            prop_assert_eq!(merged.title_align, top.title_align.or(base.title_align));
            prop_assert_eq!(merged.shadow, top.shadow.or(base.shadow));
        }

        #[test]
        fn a_raw_shape_survives_a_trip_through_toml(raw in any_raw()) {
            let text = toml::to_string(&raw).expect("serializes");
            let parsed: RawShape = toml::from_str(&text).expect("parses back");
            prop_assert_eq!(parsed, raw);
        }
    }
}
