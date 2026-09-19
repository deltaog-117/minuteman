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

//! Everything that turns the theme's strings into what actually gets drawn: color parsing (basic
//! names, `#rrggbb` hex, and a 256-color fallback for terminals without truecolor), border
//! styles, the shared pane frame, and the file-type colors.

use std::path::Path;
use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};
use shared::DirEntryInfo;
use theming::{Config, Theme};

/// Whether the terminal advertises 24-bit color. Read once — `COLORTERM` can't change under a
/// running process, and this is consulted for every colored cell.
pub fn truecolor() -> bool {
    static TRUECOLOR: OnceLock<bool> = OnceLock::new();
    *TRUECOLOR.get_or_init(|| {
        std::env::var("COLORTERM")
            .is_ok_and(|v| v.eq_ignore_ascii_case("truecolor") || v.eq_ignore_ascii_case("24bit"))
    })
}

/// Resolves a theme color string. Anything unrecognised becomes `Color::Reset` (the terminal's
/// own default) rather than an error, so a typo in one field can't stop the app from starting.
pub fn parse_color(name: &str, truecolor: bool) -> Color {
    if let Some(hex) = name.strip_prefix('#') {
        return match parse_hex(hex) {
            Some((r, g, b)) if truecolor => Color::Rgb(r, g, b),
            Some((r, g, b)) => Color::Indexed(quantize_256(r, g, b)),
            None => Color::Reset,
        };
    }
    match name.to_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        _ => Color::Reset,
    }
}

/// `rrggbb` or the short `rgb` form (each digit doubled), without the leading `#`.
fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    if !hex.is_ascii() {
        return None;
    }
    let channel = |s: &str| u8::from_str_radix(s, 16).ok();
    match hex.len() {
        6 => Some((
            channel(&hex[0..2])?,
            channel(&hex[2..4])?,
            channel(&hex[4..6])?,
        )),
        3 => {
            let double = |i: usize| channel(&hex[i..=i]).map(|v| v * 17);
            Some((double(0)?, double(1)?, double(2)?))
        }
        _ => None,
    }
}

/// The xterm-256 index nearest an RGB color: the closer of the 6x6x6 color cube and the 24-step
/// grayscale ramp, so both saturated neon and muted greys survive the downgrade.
pub fn quantize_256(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [i32; 6] = [0, 95, 135, 175, 215, 255];
    let nearest_level = |v: u8| {
        (0..6)
            .min_by_key(|&i| (LEVELS[i] - v as i32).abs())
            .expect("the range 0..6 is never empty")
    };
    let (ri, gi, bi) = (nearest_level(r), nearest_level(g), nearest_level(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);

    let average = (r as i32 + g as i32 + b as i32) / 3;
    let gray_step = ((average - 8 + 5) / 10).clamp(0, 23);
    let gray_level = 8 + 10 * gray_step;

    let distance = |(cr, cg, cb): (i32, i32, i32)| {
        (cr - r as i32).pow(2) + (cg - g as i32).pow(2) + (cb - b as i32).pow(2)
    };
    if distance((gray_level, gray_level, gray_level)) < distance(cube) {
        232 + gray_step as u8
    } else {
        16 + (36 * ri + 6 * gi + bi) as u8
    }
}

pub fn color(name: &str) -> Color {
    parse_color(name, truecolor())
}

pub fn border_type(name: &str) -> BorderType {
    match name.to_lowercase().as_str() {
        "plain" => BorderType::Plain,
        "double" => BorderType::Double,
        "thick" => BorderType::Thick,
        _ => BorderType::Rounded,
    }
}

/// The frame every pane shares. `focused` marks the pane keystrokes currently act on: it gets
/// the bright border and an accent-colored bold title, so the active pane reads as lit up
/// against the dimmer rest.
pub fn themed_block(config: &Config, title: &str, focused: bool) -> Block<'static> {
    let theme = &config.theme;
    let (border, title_style) = if focused {
        (
            color(&theme.border_focused_fg),
            Style::default()
                .fg(color(&theme.accent_fg))
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            color(&theme.border_fg),
            Style::default().fg(color(&theme.title_fg)),
        )
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
        .title(Line::from(Span::styled(format!(" {title} "), title_style)));
    if crate::glyphs::of(config).ascii_borders {
        block.border_set(crate::glyphs::ASCII_BORDER)
    } else {
        block.border_type(border_type(&theme.border_type))
    }
}

/// Broad file categories, each with its own theme color so a directory listing can be read by
/// color at a glance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Dir,
    Source,
    Config,
    Doc,
    Archive,
    Media,
    Other,
}

impl FileKind {
    pub fn classify(entry: &DirEntryInfo) -> Self {
        if entry.is_dir {
            return Self::Dir;
        }
        // Dotfiles (`.gitignore`, `.zshrc`) are configuration whatever follows the dot.
        if entry.name.starts_with('.') {
            return Self::Config;
        }
        let ext = Path::new(&entry.name)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        match ext.as_deref() {
            Some(
                "rs" | "py" | "js" | "mjs" | "ts" | "tsx" | "jsx" | "c" | "h" | "cc" | "cpp"
                | "hpp" | "go" | "java" | "kt" | "lua" | "rb" | "php" | "sh" | "bash" | "zsh"
                | "fish" | "swift" | "cs" | "scala" | "hs" | "ml" | "zig" | "nim" | "sql" | "css"
                | "scss" | "html" | "htm" | "vue" | "svelte",
            ) => Self::Source,
            Some(
                "toml" | "yaml" | "yml" | "json" | "ini" | "conf" | "cfg" | "env" | "lock" | "xml",
            ) => Self::Config,
            Some(
                "md" | "txt" | "rst" | "pdf" | "doc" | "docx" | "odt" | "tex" | "org" | "epub",
            ) => Self::Doc,
            Some(
                "zip" | "tar" | "gz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "tgz" | "deb" | "rpm"
                | "iso",
            ) => Self::Archive,
            Some(
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "tiff" | "mp3"
                | "flac" | "wav" | "ogg" | "opus" | "mp4" | "mkv" | "webm" | "avi" | "mov",
            ) => Self::Media,
            _ => match entry.name.as_str() {
                "Makefile" | "Dockerfile" | "LICENSE" => Self::Config,
                _ => Self::Other,
            },
        }
    }

    /// This kind's color string in `theme`.
    pub fn theme_color(self, theme: &Theme) -> &str {
        match self {
            Self::Dir => &theme.dir_fg,
            Self::Source => &theme.source_fg,
            Self::Config => &theme.config_fg,
            Self::Doc => &theme.doc_fg,
            Self::Archive => &theme.archive_fg,
            Self::Media => &theme.media_fg,
            Self::Other => &theme.file_fg,
        }
    }
}

/// The color to paint the selected row's text, or `None` when the theme says `"keep"` — the
/// entry then stays in its own file-type color and only the row's background changes.
pub fn selection_fg(theme: &Theme) -> Option<Color> {
    (!theme.selection_fg.eq_ignore_ascii_case("keep")).then(|| color(&theme.selection_fg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, is_dir: bool) -> DirEntryInfo {
        DirEntryInfo {
            name: name.into(),
            path: name.into(),
            is_dir,
            size: 0,
            modified: None,
            mode: None,
        }
    }

    #[test]
    fn basic_names_and_typos_resolve_like_before() {
        assert_eq!(parse_color("cyan", true), Color::Cyan);
        assert_eq!(parse_color("GREY", true), Color::Gray);
        assert_eq!(parse_color("not-a-color", true), Color::Reset);
    }

    #[test]
    fn hex_becomes_rgb_on_truecolor_terminals() {
        assert_eq!(parse_color("#00f0ff", true), Color::Rgb(0, 240, 255));
        assert_eq!(parse_color("#0af", true), Color::Rgb(0, 170, 255));
    }

    #[test]
    fn hex_is_quantized_without_truecolor() {
        assert_eq!(parse_color("#000000", false), Color::Indexed(16));
        assert_eq!(parse_color("#ffffff", false), Color::Indexed(231));
        // Pure cyan-ish neon lands in the color cube, not the grayscale ramp.
        assert!(matches!(
            parse_color("#00f0ff", false),
            Color::Indexed(16..=231)
        ));
    }

    #[test]
    fn malformed_hex_resets_instead_of_failing() {
        for bad in ["#", "#12", "#12345", "#gggggg", "#1234567", "#é12345"] {
            assert_eq!(parse_color(bad, true), Color::Reset, "{bad}");
        }
    }

    #[test]
    fn quantize_sends_greys_to_the_ramp_and_colors_to_the_cube() {
        // A mid grey has no exact cube entry, so the 24-step ramp is the closer match.
        assert!(matches!(quantize_256(128, 128, 128), 232..=255));
        assert!(matches!(quantize_256(255, 43, 214), 16..=231));
    }

    #[test]
    fn quantize_never_leaves_the_256_color_palette_for_any_input() {
        for r in (0..=255).step_by(15) {
            for g in (0..=255).step_by(15) {
                for b in (0..=255).step_by(15) {
                    assert!(quantize_256(r, g, b) >= 16, "({r},{g},{b})");
                }
            }
        }
    }

    #[test]
    fn border_type_falls_back_to_rounded() {
        assert_eq!(border_type("double"), BorderType::Double);
        assert_eq!(border_type("PLAIN"), BorderType::Plain);
        assert_eq!(border_type("thick"), BorderType::Thick);
        assert_eq!(border_type("wobbly"), BorderType::Rounded);
    }

    #[test]
    fn classify_picks_a_kind_by_extension_case_insensitively() {
        assert_eq!(FileKind::classify(&file("src", true)), FileKind::Dir);
        assert_eq!(
            FileKind::classify(&file("main.RS", false)),
            FileKind::Source
        );
        assert_eq!(
            FileKind::classify(&file("Cargo.toml", false)),
            FileKind::Config
        );
        assert_eq!(FileKind::classify(&file("README.md", false)), FileKind::Doc);
        assert_eq!(FileKind::classify(&file("a.tar", false)), FileKind::Archive);
        assert_eq!(FileKind::classify(&file("cat.png", false)), FileKind::Media);
        assert_eq!(FileKind::classify(&file("mystery", false)), FileKind::Other);
    }

    #[test]
    fn dotfiles_and_well_known_names_are_config() {
        assert_eq!(
            FileKind::classify(&file(".gitignore", false)),
            FileKind::Config
        );
        assert_eq!(
            FileKind::classify(&file("Makefile", false)),
            FileKind::Config
        );
    }

    #[test]
    fn keep_leaves_the_selected_row_in_its_own_color() {
        let mut theme = Theme::default();
        assert_eq!(selection_fg(&theme), None);
        theme.selection_fg = "white".into();
        assert_eq!(selection_fg(&theme), Some(Color::White));
    }
}
