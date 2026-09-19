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

//! Font and color snippets for popular terminals, printed by `minuteman init-terminal <name>`.
//!
//! The UI's font is the terminal's to choose, so the way to make the whole thing look like one
//! design is to configure the terminal to match: a Nerd Font (so the `nerd` glyph set's icons and
//! Powerline arrows exist) and the neon palette (so anything drawn in the terminal's own 16 ANSI
//! colors matches the theme's hex colors). Snippets are only printed — never written to the
//! user's config files.

use theming::Font;

/// A terminal `init-terminal` can print a snippet for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terminal {
    Kitty,
    Alacritty,
    Wezterm,
}

impl Terminal {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "kitty" => Some(Self::Kitty),
            "alacritty" => Some(Self::Alacritty),
            "wezterm" => Some(Self::Wezterm),
            _ => None,
        }
    }

    /// The snippet for this terminal, with `font` (from `appearance.toml`'s `[font]`).
    pub fn snippet(self, font: &Font) -> String {
        match self {
            Self::Kitty => kitty(font),
            Self::Alacritty => alacritty(font),
            Self::Wezterm => wezterm(font),
        }
    }
}

/// A font size the way terminal configs write it: one decimal place (`12.0`, `13.5`).
fn size(font: &Font) -> String {
    format!("{:.1}", font.size)
}

const BACKGROUND: &str = "#0b0e1e";
const FOREGROUND: &str = "#c8ccff";
const CURSOR: &str = "#ff2bd6";
const SELECTION_BG: &str = "#2b1a4f";

/// The 16 ANSI colors, in order: black, red, green, yellow, blue, magenta, cyan, white — then the
/// bright row. Chosen to match the neon theme's own colors.
const NORMAL: [&str; 8] = [
    "#0b0e1e", "#ff3860", "#39ff88", "#ffd60a", "#4d7cff", "#ff2bd6", "#00f0ff", "#c8ccff",
];
const BRIGHT: [&str; 8] = [
    "#3d4270", "#ff6b86", "#7dffb0", "#ffe55c", "#7fa0ff", "#ff70e6", "#6dffff", "#ffffff",
];

fn kitty(font: &Font) -> String {
    let mut lines = vec![
        "# Add to ~/.config/kitty/kitty.conf".to_string(),
        format!("font_family      {}", font.family),
        format!("font_size        {}", size(font)),
        String::new(),
        "# Prefer to keep your current font? Leave font_family alone and add only this, which"
            .to_string(),
        "# takes the icons and arrows from a Symbols Nerd Font (install \"Symbols Nerd Font Mono\"):"
            .to_string(),
        "# symbol_map U+E0A0-U+E0D7,U+F000-U+F2E0 Symbols Nerd Font Mono".to_string(),
        String::new(),
        format!("background            {BACKGROUND}"),
        format!("foreground            {FOREGROUND}"),
        format!("cursor                {CURSOR}"),
        format!("selection_background  {SELECTION_BG}"),
    ];
    lines.extend(
        NORMAL
            .iter()
            .chain(BRIGHT.iter())
            .enumerate()
            .map(|(i, color)| format!("color{i:<2}  {color}")),
    );
    lines.join("\n") + "\n"
}

fn alacritty(font: &Font) -> String {
    const NAMES: [&str; 8] = [
        "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
    ];
    let mut lines = vec![
        "# Add to ~/.config/alacritty/alacritty.toml".to_string(),
        "[font]".to_string(),
        format!("size = {}", size(font)),
        String::new(),
        "[font.normal]".to_string(),
        format!("family = \"{}\"", font.family.replace('"', "\\\"")),
        String::new(),
        "[colors.primary]".to_string(),
        format!("background = \"{BACKGROUND}\""),
        format!("foreground = \"{FOREGROUND}\""),
        String::new(),
        "[colors.cursor]".to_string(),
        format!("cursor = \"{CURSOR}\""),
        format!("text = \"{BACKGROUND}\""),
        String::new(),
        "[colors.selection]".to_string(),
        format!("background = \"{SELECTION_BG}\""),
        "text = \"CellForeground\"".to_string(),
    ];
    for (table, colors) in [("normal", &NORMAL), ("bright", &BRIGHT)] {
        lines.push(String::new());
        lines.push(format!("[colors.{table}]"));
        lines.extend(
            NAMES
                .iter()
                .zip(colors.iter())
                .map(|(name, color)| format!("{name} = \"{color}\"")),
        );
    }
    lines.join("\n") + "\n"
}

fn wezterm(font: &Font) -> String {
    let list = |colors: &[&str; 8]| {
        colors
            .iter()
            .map(|c| format!("'{c}'"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    [
        "-- Add to ~/.config/wezterm/wezterm.lua, before `return config`".to_string(),
        format!(
            "config.font = wezterm.font('{}')",
            font.family.replace('\'', "\\'")
        ),
        format!("config.font_size = {}", size(font)),
        "config.colors = {".to_string(),
        format!("  foreground = '{FOREGROUND}',"),
        format!("  background = '{BACKGROUND}',"),
        format!("  cursor_bg = '{CURSOR}',"),
        format!("  cursor_border = '{CURSOR}',"),
        format!("  selection_bg = '{SELECTION_BG}',"),
        format!("  ansi = {{ {} }},", list(&NORMAL)),
        format!("  brights = {{ {} }},", list(&BRIGHT)),
        "}".to_string(),
    ]
    .join("\n")
        + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use theming::appearance::DEFAULT_FONT_FAMILY;

    const ALL: [Terminal; 3] = [Terminal::Kitty, Terminal::Alacritty, Terminal::Wezterm];

    #[test]
    fn parses_only_supported_terminals() {
        assert_eq!(Terminal::parse("kitty"), Some(Terminal::Kitty));
        assert_eq!(Terminal::parse("alacritty"), Some(Terminal::Alacritty));
        assert_eq!(Terminal::parse("wezterm"), Some(Terminal::Wezterm));
        assert_eq!(Terminal::parse("xterm"), None);
    }

    #[test]
    fn every_snippet_sets_the_mono_nerd_font_and_all_sixteen_colors() {
        for terminal in ALL {
            let text = terminal.snippet(&Font::default());
            assert!(
                text.contains(DEFAULT_FONT_FAMILY),
                "{terminal:?} lacks the font"
            );
            for color in NORMAL.iter().chain(BRIGHT.iter()) {
                assert!(text.contains(color), "{terminal:?} lacks {color}");
            }
            assert!(text.contains(BACKGROUND) && text.contains(FOREGROUND));
        }
    }

    #[test]
    fn snippets_have_no_stray_indentation_and_end_in_one_newline() {
        for terminal in ALL {
            let text = terminal.snippet(&Font::default());
            assert!(
                text.ends_with('\n') && !text.ends_with("\n\n"),
                "{terminal:?}"
            );
            for line in text.lines() {
                // Only wezterm's table body is meant to be indented, by exactly two spaces.
                let indent = line.len() - line.trim_start().len();
                assert!(
                    indent == 0 || (terminal == Terminal::Wezterm && indent == 2),
                    "{terminal:?} has an oddly indented line: {line:?}"
                );
            }
        }
    }

    #[test]
    fn a_configured_font_replaces_the_default_in_every_snippet() {
        let font = Font {
            family: "Iosevka Term".into(),
            size: 13.5,
        };
        for terminal in ALL {
            let text = terminal.snippet(&font);
            assert!(text.contains("Iosevka Term"), "{terminal:?}: {text}");
            assert!(text.contains("13.5"), "{terminal:?} lacks the size");
            assert!(
                !text.contains(DEFAULT_FONT_FAMILY),
                "{terminal:?} kept the default"
            );
        }
    }

    #[test]
    fn font_names_with_quotes_cannot_break_the_snippet() {
        let font = Font {
            family: "Odd\"Name".into(),
            size: 12.0,
        };
        assert!(Terminal::Alacritty.snippet(&font).contains("Odd\\\"Name"));
        let font = Font {
            family: "Odd'Name".into(),
            size: 12.0,
        };
        assert!(Terminal::Wezterm.snippet(&font).contains("Odd\\'Name"));
    }

    #[test]
    fn the_kitty_snippet_offers_the_symbol_map_fallback() {
        assert!(
            Terminal::Kitty
                .snippet(&Font::default())
                .contains("symbol_map")
        );
    }

    #[test]
    fn the_ansi_palette_matches_the_themes_own_neon_colors() {
        let theme = theming::Theme::default();
        assert_eq!(NORMAL[1], theme.danger_fg);
        assert_eq!(NORMAL[2], theme.source_fg);
        assert_eq!(NORMAL[3], theme.config_fg);
        assert_eq!(NORMAL[5], theme.accent_fg);
        assert_eq!(NORMAL[6], theme.border_focused_fg);
        assert_eq!(NORMAL[7], theme.file_fg);
        assert_eq!(SELECTION_BG, theme.selection_bg);
    }
}
