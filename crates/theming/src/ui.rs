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

use serde::Deserialize;

/// Which set of symbols the interface draws with. The font is the terminal's, not the app's, so
/// this is how the UI matches what the user's font can actually show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlyphSet {
    /// Box-drawing and geometric symbols found in nearly every modern font. The default.
    #[default]
    Unicode,
    /// Adds file-type icons and Powerline arrows; needs a Nerd Font (or a Symbols Nerd Font
    /// fallback) in the terminal.
    Nerd,
    /// Plain ASCII chrome, for a Linux console or a terminal with a very limited font.
    Ascii,
}

impl GlyphSet {
    /// Parses a config value (case-insensitive). `None` for anything unrecognised, so the caller
    /// can fall back rather than fail startup on a typo.
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "unicode" => Some(Self::Unicode),
            "nerd" => Some(Self::Nerd),
            "ascii" => Some(Self::Ascii),
            _ => None,
        }
    }
}

/// Resolved `[ui]` settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ui {
    pub glyphs: GlyphSet,
}

/// Deserialized `[ui]` config; every field optional, like the rest of the config.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RawUi {
    pub glyphs: Option<String>,
}

impl From<RawUi> for Ui {
    fn from(raw: RawUi) -> Self {
        Self {
            glyphs: raw
                .glyphs
                .as_deref()
                .and_then(GlyphSet::parse)
                .unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_sets_parse_case_insensitively() {
        assert_eq!(GlyphSet::parse("nerd"), Some(GlyphSet::Nerd));
        assert_eq!(GlyphSet::parse("ASCII"), Some(GlyphSet::Ascii));
        assert_eq!(GlyphSet::parse("Unicode"), Some(GlyphSet::Unicode));
        assert_eq!(GlyphSet::parse("emoji"), None);
    }

    #[test]
    fn default_is_unicode_and_a_typo_falls_back_to_it() {
        assert_eq!(Ui::default().glyphs, GlyphSet::Unicode);
        let typo: Ui = RawUi {
            glyphs: Some("nerdd".into()),
        }
        .into();
        assert_eq!(typo.glyphs, GlyphSet::Unicode);
        let nerd: Ui = RawUi {
            glyphs: Some("nerd".into()),
        }
        .into();
        assert_eq!(nerd.glyphs, GlyphSet::Nerd);
    }
}
