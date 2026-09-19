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

//! The symbols the interface draws with, per `[ui] glyphs` set. Every place that used to
//! hardcode a box-drawing character or symbol takes it from here instead, so one setting makes
//! the whole UI match what the user's terminal font can show.

use ratatui::symbols::border;
use theming::{Config, GlyphSet};

use crate::style::FileKind;

/// File-type icons — Font Awesome codepoints, which Nerd Fonts v2 and v3 both keep in place
/// (unlike the Material Design range, which v3 moved).
pub struct Icons {
    pub dir: &'static str,
    pub source: &'static str,
    pub config: &'static str,
    pub doc: &'static str,
    pub archive: &'static str,
    pub media: &'static str,
    pub other: &'static str,
}

impl Icons {
    pub fn for_kind(&self, kind: FileKind) -> &'static str {
        match kind {
            FileKind::Dir => self.dir,
            FileKind::Source => self.source,
            FileKind::Config => self.config,
            FileKind::Doc => self.doc,
            FileKind::Archive => self.archive,
            FileKind::Media => self.media,
            FileKind::Other => self.other,
        }
    }
}

pub struct Glyphs {
    /// Marks the selected row.
    pub stripe: &'static str,
    /// Anchors the left edge of the header.
    pub header_prefix: &'static str,
    /// Between breadcrumb segments, spaces included.
    pub crumb_sep: &'static str,
    pub thumb: &'static str,
    pub track: &'static str,
    /// The text cursor at the end of a prompt.
    pub cursor: &'static str,
    /// Between status-bar segments that share a background, when arrows aren't used.
    pub divider: &'static str,
    pub gauge_on: &'static str,
    pub gauge_off: &'static str,
    /// Leading symbols of the header pills; empty when the set has none.
    pub yanked: &'static str,
    pub cut: &'static str,
    pub marked: &'static str,
    /// Stands in for a missing value (a directory's size, an unknown age).
    pub none: &'static str,
    pub arrow_right: &'static str,
    pub arrow_left: &'static str,
    /// Hollow arrow between neighbours that share a background.
    pub arrow_right_thin: &'static str,
    /// Whether the `auto` separator setting should pick arrows.
    pub powerline: bool,
    /// Draw pane frames with ASCII instead of box-drawing characters.
    pub ascii_borders: bool,
    pub icons: Option<Icons>,
}

const POWERLINE_RIGHT: &str = "\u{e0b0}";
const POWERLINE_LEFT: &str = "\u{e0b2}";
const POWERLINE_RIGHT_THIN: &str = "\u{e0b1}";

static UNICODE: Glyphs = Glyphs {
    stripe: "▌",
    header_prefix: " ▌ ",
    crumb_sep: " › ",
    thumb: "█",
    track: "│",
    cursor: "▏",
    divider: "│",
    gauge_on: "▰",
    gauge_off: "▱",
    yanked: "⧉",
    cut: "✂",
    marked: "◆",
    none: "—",
    arrow_right: POWERLINE_RIGHT,
    arrow_left: POWERLINE_LEFT,
    arrow_right_thin: POWERLINE_RIGHT_THIN,
    powerline: false,
    ascii_borders: false,
    icons: None,
};

static NERD: Glyphs = Glyphs {
    crumb_sep: " \u{e0b1} ",
    yanked: "\u{f0c5}",
    cut: "\u{f0c4}",
    marked: "\u{f00c}",
    powerline: true,
    icons: Some(Icons {
        dir: "\u{f07b}",
        source: "\u{f121}",
        config: "\u{f013}",
        doc: "\u{f15c}",
        archive: "\u{f1c6}",
        media: "\u{f1c5}",
        other: "\u{f016}",
    }),
    ..UNICODE
};

static ASCII: Glyphs = Glyphs {
    stripe: ">",
    header_prefix: " > ",
    crumb_sep: " > ",
    thumb: "#",
    track: "|",
    cursor: "_",
    divider: "|",
    gauge_on: "#",
    gauge_off: "-",
    yanked: "",
    cut: "",
    marked: "",
    none: "-",
    arrow_right: ">",
    arrow_left: "<",
    arrow_right_thin: ">",
    powerline: false,
    ascii_borders: true,
    icons: None,
};

/// The pane frame for `Glyphs::ascii_borders`.
pub const ASCII_BORDER: border::Set<'static> = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

impl Glyphs {
    pub fn for_set(set: GlyphSet) -> &'static Glyphs {
        match set {
            GlyphSet::Unicode => &UNICODE,
            GlyphSet::Nerd => &NERD,
            GlyphSet::Ascii => &ASCII,
        }
    }

    /// Cells a file-type icon takes in front of a name: the icon and a space, or nothing.
    pub fn icon_width(&self) -> usize {
        if self.icons.is_some() { 2 } else { 0 }
    }

    /// A header-pill label: the symbol (if this set has one) then the text.
    pub fn pill(&self, symbol: &str, text: &str) -> String {
        if symbol.is_empty() {
            text.to_string()
        } else {
            format!("{symbol} {text}")
        }
    }
}

pub fn of(config: &Config) -> &'static Glyphs {
    Glyphs::for_set(config.ui.glyphs)
}

/// What `minuteman glyphs` prints: every symbol the given set draws with, so someone can see
/// straight away whether their terminal font has them (missing ones show as boxes or `?`).
pub fn sample(set: GlyphSet) -> String {
    let g = Glyphs::for_set(set);
    let name = match set {
        GlyphSet::Unicode => "unicode",
        GlyphSet::Nerd => "nerd",
        GlyphSet::Ascii => "ascii",
    };
    let mut out = format!("[{name}]\n");
    out.push_str(&format!(
        "  frame        {}\n",
        if g.ascii_borders {
            "+--+ |  |".to_string()
        } else {
            "╭──╮ │  │ ╰──╯".to_string()
        }
    ));
    out.push_str(&format!(
        "  selection    {}   header {}   breadcrumb ~{}dev   scroll {}{}   gauge {}{}{}\n",
        g.stripe,
        g.header_prefix.trim(),
        g.crumb_sep,
        g.thumb,
        g.track,
        g.gauge_on,
        g.gauge_on,
        g.gauge_off,
    ));
    out.push_str(&format!(
        "  pills        {}   {}   {}\n",
        g.pill(g.yanked, "yanked"),
        g.pill(g.cut, "cut"),
        g.pill(g.marked, "marked"),
    ));
    out.push_str(&format!(
        "  arrows       {} {} {}\n",
        g.arrow_right, g.arrow_right_thin, g.arrow_left
    ));
    match &g.icons {
        Some(icons) => {
            out.push_str("  icons        ");
            for (label, icon) in [
                ("dir", icons.dir),
                ("source", icons.source),
                ("config", icons.config),
                ("doc", icons.doc),
                ("archive", icons.archive),
                ("media", icons.media),
                ("other", icons.other),
            ] {
                out.push_str(&format!("{icon} {label}   "));
            }
            out.push('\n');
        }
        None => out.push_str("  icons        (none in this set)\n"),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_symbol(g: &Glyphs) -> Vec<&'static str> {
        vec![
            g.stripe,
            g.header_prefix,
            g.crumb_sep,
            g.thumb,
            g.track,
            g.cursor,
            g.divider,
            g.gauge_on,
            g.gauge_off,
            g.yanked,
            g.cut,
            g.marked,
            g.none,
            g.arrow_right,
            g.arrow_left,
            g.arrow_right_thin,
        ]
    }

    #[test]
    fn the_ascii_set_is_pure_ascii() {
        let g = Glyphs::for_set(GlyphSet::Ascii);
        for symbol in every_symbol(g) {
            assert!(symbol.is_ascii(), "{symbol:?} is not ASCII");
        }
        assert!(g.ascii_borders);
        assert!(g.icons.is_none());
    }

    #[test]
    fn only_the_nerd_set_has_icons_and_powerline() {
        assert!(Glyphs::for_set(GlyphSet::Nerd).icons.is_some());
        assert!(Glyphs::for_set(GlyphSet::Nerd).powerline);
        assert!(Glyphs::for_set(GlyphSet::Unicode).icons.is_none());
        assert!(!Glyphs::for_set(GlyphSet::Unicode).powerline);
        assert_eq!(Glyphs::for_set(GlyphSet::Nerd).icon_width(), 2);
        assert_eq!(Glyphs::for_set(GlyphSet::Unicode).icon_width(), 0);
    }

    #[test]
    fn every_file_kind_has_a_distinct_enough_nerd_icon() {
        let icons = Glyphs::for_set(GlyphSet::Nerd)
            .icons
            .as_ref()
            .expect("nerd has icons");
        let kinds = [
            FileKind::Dir,
            FileKind::Source,
            FileKind::Config,
            FileKind::Doc,
            FileKind::Archive,
            FileKind::Media,
            FileKind::Other,
        ];
        let mut seen = std::collections::HashSet::new();
        for kind in kinds {
            let icon = icons.for_kind(kind);
            assert_eq!(icon.chars().count(), 1, "{kind:?} icon is one codepoint");
            assert!(seen.insert(icon), "{kind:?} reuses an icon");
        }
    }

    #[test]
    fn pills_only_gain_a_symbol_when_the_set_has_one() {
        assert_eq!(Glyphs::for_set(GlyphSet::Ascii).pill("", "cut"), "cut");
        assert_eq!(Glyphs::for_set(GlyphSet::Unicode).pill("✂", "cut"), "✂ cut");
    }

    #[test]
    fn the_sample_names_the_set_and_lists_icons_only_for_nerd() {
        assert!(sample(GlyphSet::Nerd).contains("[nerd]"));
        assert!(sample(GlyphSet::Nerd).contains("source"));
        assert!(sample(GlyphSet::Unicode).contains("(none in this set)"));
    }
}
