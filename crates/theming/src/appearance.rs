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

//! The parts of the appearance that aren't colors or glyphs: text styles (bold, italic, ...) per
//! interface element, and the font the user wants their terminal to use.
//!
//! Both are read from `appearance.toml` alongside the colors (`[theme]`) and glyph set (`[ui]`),
//! so one file holds the whole look. The font can't be *applied* by Minuteman — the terminal owns
//! it — so `[font]` exists to feed `minuteman init-terminal`, which prints a terminal config with
//! it.

use serde::Deserialize;

/// Text attributes for one element. A set of flags rather than a list of names so an invalid
/// combination can't be represented and the renderer never re-parses strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub bold: bool,
    pub italic: bool,
    pub dim: bool,
    pub underline: bool,
    pub reverse: bool,
    pub crossed_out: bool,
}

impl Mods {
    /// Builds the flags from config names: `bold`, `italic`, `dim`, `underline`, `reverse`,
    /// `strikethrough` (case-insensitive, with a few aliases). Unrecognised names are skipped so
    /// a typo in one entry can't stop the app starting.
    pub fn parse<S: AsRef<str>>(names: &[S]) -> Self {
        let mut mods = Self::default();
        for name in names {
            match name.as_ref().trim().to_lowercase().as_str() {
                "bold" => mods.bold = true,
                "italic" | "italics" => mods.italic = true,
                "dim" | "faint" => mods.dim = true,
                "underline" | "underlined" => mods.underline = true,
                "reverse" | "reversed" | "inverse" => mods.reverse = true,
                "strikethrough" | "crossed_out" | "crossed-out" => mods.crossed_out = true,
                _ => {}
            }
        }
        mods
    }
}

/// Declares, once, everything that needs a per-element entry: the resolved `Styles` struct, its
/// deserializable `RawStyles` twin, the defaults, the layering of two files, and the list of
/// element names. Each line is `element: [default modifiers]`.
macro_rules! element_styles {
    ($($(#[$doc:meta])* $field:ident : [$($default:ident),*]),* $(,)?) => {
        /// Resolved text attributes for every styled element of the interface.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct Styles {
            $($(#[$doc])* pub $field: Mods,)*
        }

        impl Default for Styles {
            fn default() -> Self {
                Self {
                    $($field: Mods::parse::<&str>(&[$(stringify!($default)),*]),)*
                }
            }
        }

        /// `[style]` as written in the file: each element is an optional list of modifier names.
        /// A list *replaces* that element's default (so `dir = []` turns its bold off).
        #[derive(Debug, Clone, Default, Deserialize)]
        #[serde(default)]
        pub struct RawStyles {
            $(pub $field: Option<Vec<String>>,)*
        }

        impl RawStyles {
            /// `top` wins wherever it sets an element; otherwise `self` shows through.
            pub fn overlay(self, top: RawStyles) -> RawStyles {
                RawStyles { $($field: top.$field.or(self.$field),)* }
            }
        }

        impl From<RawStyles> for Styles {
            fn from(raw: RawStyles) -> Self {
                let base = Styles::default();
                Self {
                    $($field: raw.$field.map_or(base.$field, |names| Mods::parse(&names)),)*
                }
            }
        }

        /// Every element name accepted under `[style]`.
        pub const STYLE_ELEMENTS: &[&str] = &[$(stringify!($field)),*];
    };
}

element_styles! {
    /// Directory names.
    dir: [bold],
    source: [],
    config: [],
    doc: [],
    archive: [],
    media: [],
    other: [],
    /// Added on top of a file's kind style when it has an execute bit.
    executable: [bold],
    /// The selected row's name.
    selection: [bold],
    /// The mark in front of a marked row, and a marked row's name.
    mark: [bold],
    /// Pane titles other than the active pane's.
    title: [],
    title_focused: [bold],
    /// The size and age columns.
    columns: [],
    /// Breadcrumb segments above the current directory.
    breadcrumb: [],
    breadcrumb_current: [bold],
    /// The header's status pills (marks, clipboard, running job).
    pill: [],
    /// The status bar's mode pill.
    mode: [bold],
    /// The selected file's name in the status bar.
    status_name: [bold],
    /// The rest of the status bar's segments.
    status: [],
    hint_key: [],
    hint_label: [],
    /// A transient status message.
    message: [],
}

/// The font `minuteman init-terminal` puts in the snippet it prints.
#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    pub family: String,
    pub size: f64,
}

/// The "Mono" Nerd Font variant keeps every icon one cell wide.
pub const DEFAULT_FONT_FAMILY: &str = "JetBrainsMono Nerd Font Mono";
pub const DEFAULT_FONT_SIZE: f64 = 12.0;

impl Default for Font {
    fn default() -> Self {
        Self {
            family: DEFAULT_FONT_FAMILY.into(),
            size: DEFAULT_FONT_SIZE,
        }
    }
}

/// `[font]` as written in the file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RawFont {
    pub family: Option<String>,
    pub size: Option<f64>,
}

impl RawFont {
    pub fn overlay(self, top: RawFont) -> RawFont {
        RawFont {
            family: top.family.or(self.family),
            size: top.size.or(self.size),
        }
    }
}

impl From<RawFont> for Font {
    fn from(raw: RawFont) -> Self {
        let defaults = Font::default();
        Self {
            family: raw
                .family
                .map(|f| f.trim().to_string())
                .filter(|f| !f.is_empty())
                .unwrap_or(defaults.family),
            // A nonsense size (zero, negative, NaN, absurd) falls back rather than producing a
            // terminal config that can't load.
            size: raw
                .size
                .filter(|s| s.is_finite() && (4.0..=96.0).contains(s))
                .unwrap_or(defaults.size),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(toml_text: &str) -> RawStyles {
        toml::from_str(toml_text).expect("valid test toml")
    }

    #[test]
    fn modifier_names_parse_case_insensitively_with_aliases() {
        let mods = Mods::parse(&[
            "Bold",
            "ITALIC",
            "faint",
            "underlined",
            "inverse",
            "strikethrough",
        ]);
        assert!(mods.bold && mods.italic && mods.dim);
        assert!(mods.underline && mods.reverse && mods.crossed_out);
    }

    #[test]
    fn unknown_modifier_names_are_skipped_not_fatal() {
        let mods = Mods::parse(&["bold", "sparkly", ""]);
        assert!(mods.bold);
        assert!(!mods.italic && !mods.dim);
        assert_eq!(Mods::parse::<&str>(&[]), Mods::default());
    }

    #[test]
    fn defaults_make_directories_and_executables_bold_but_not_plain_files() {
        let styles = Styles::default();
        assert!(styles.dir.bold);
        assert!(styles.executable.bold);
        assert!(styles.selection.bold && styles.title_focused.bold);
        assert_eq!(styles.doc, Mods::default());
        assert_eq!(styles.other, Mods::default());
    }

    #[test]
    fn a_style_list_replaces_the_default_instead_of_adding_to_it() {
        let styles: Styles = raw("dir = [\"italic\"]").into();
        assert!(styles.dir.italic);
        assert!(!styles.dir.bold, "the default bold must be replaced");
    }

    #[test]
    fn an_empty_list_turns_a_default_off() {
        let styles: Styles = raw("dir = []").into();
        assert_eq!(styles.dir, Mods::default());
        // Elements the file doesn't mention keep their defaults.
        assert!(styles.selection.bold);
    }

    #[test]
    fn overlay_lets_the_top_file_win_per_element() {
        let base = raw("dir = [\"italic\"]\ndoc = [\"dim\"]");
        let top = raw("dir = [\"bold\"]");
        let styles: Styles = base.overlay(top).into();
        assert!(styles.dir.bold && !styles.dir.italic);
        assert!(
            styles.doc.dim,
            "an element only the base sets still shows through"
        );
    }

    #[test]
    fn every_element_name_is_accepted_under_style() {
        assert!(STYLE_ELEMENTS.contains(&"dir") && STYLE_ELEMENTS.contains(&"message"));
        let all_bold: String = STYLE_ELEMENTS
            .iter()
            .map(|e| format!("{e} = [\"bold\"]\n"))
            .collect();
        let styles: Styles = raw(&all_bold).into();
        assert!(styles.doc.bold && styles.other.bold && styles.hint_key.bold);
    }

    #[test]
    fn font_defaults_to_the_mono_nerd_font() {
        let font: Font = RawFont::default().into();
        assert_eq!(font, Font::default());
        assert_eq!(font.family, DEFAULT_FONT_FAMILY);
    }

    #[test]
    fn font_overrides_apply_and_bad_values_fall_back() {
        let font: Font = RawFont {
            family: Some("  Iosevka  ".into()),
            size: Some(13.5),
        }
        .into();
        assert_eq!((font.family.as_str(), font.size), ("Iosevka", 13.5));

        for bad in [0.0, -3.0, f64::NAN, f64::INFINITY, 500.0] {
            let font: Font = RawFont {
                family: Some("   ".into()),
                size: Some(bad),
            }
            .into();
            assert_eq!(font, Font::default(), "size {bad}");
        }
    }
}
