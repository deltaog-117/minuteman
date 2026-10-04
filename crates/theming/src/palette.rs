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

//! Builds a [`Theme`] out of the colors a terminal reports for itself, so the file manager
//! matches whatever the user's wallpaper or theming tool has already pushed into the terminal —
//! on any window manager, desktop or OS.

use crate::theme::Theme;

pub type Rgb = (u8, u8, u8);

/// The terminal's live colors: its background, foreground and the 16 ANSI slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalPalette {
    pub background: Rgb,
    /// `None` when the terminal answered for the background but not the foreground.
    pub foreground: Option<Rgb>,
    pub ansi: [Rgb; 16],
}

// WCAG-style contrast floors. Text must stay readable; frames and comments only need to be seen.
const TEXT_CONTRAST: f64 = 3.5;
const COMMENT_CONTRAST: f64 = 2.2;
const FRAME_CONTRAST: f64 = 1.8;

impl Theme {
    /// Maps a terminal's own palette onto every theme field. Panel and bar backgrounds stay
    /// `reset` so a translucent or wallpaper-backed terminal shows through; only the selection
    /// row gets a solid color, tinted from the background toward the foreground.
    pub fn from_palette(palette: &TerminalPalette) -> Self {
        let bg = palette.background;
        let fg = palette.foreground.unwrap_or(palette.ansi[7]);
        let ansi = |i: usize| palette.ansi[i];
        let text = |c: Rgb| hex(ensure_contrast(c, bg, fg, TEXT_CONTRAST));

        // The most colorful of green/yellow/blue/magenta/cyan leads: it is what the wallpaper's
        // palette is actually built around. Red is left out because it means "danger" here.
        let accent = [2, 3, 4, 5, 6]
            .into_iter()
            .map(ansi)
            .max_by(|a, b| saturation(*a).total_cmp(&saturation(*b)))
            .unwrap_or_else(|| ansi(4));

        let comment = ensure_contrast(mix(bg, fg, 0.45), bg, fg, COMMENT_CONTRAST);
        let frame = ensure_contrast(ansi(8), bg, fg, FRAME_CONTRAST);

        Self {
            selection_bg: hex(mix(bg, fg, 0.16)),
            selection_fg: "keep".into(),
            border_fg: hex(frame),
            border_focused_fg: text(accent),
            title_fg: text(mix(accent, fg, 0.4)),
            accent_fg: text(accent),
            dir_fg: text(ansi(4)),
            file_fg: text(fg),
            source_fg: text(ansi(2)),
            config_fg: text(ansi(3)),
            doc_fg: text(ansi(5)),
            archive_fg: text(ansi(1)),
            media_fg: text(ansi(13)),
            syntax_keyword_fg: text(ansi(5)),
            syntax_string_fg: text(ansi(2)),
            syntax_comment_fg: hex(comment),
            syntax_number_fg: text(ansi(3)),
            syntax_function_fg: text(ansi(4)),
            syntax_type_fg: text(ansi(6)),
            status_fg: text(mix(bg, fg, 0.7)),
            bar_bg: "reset".into(),
            danger_fg: text(ansi(1)),
            border_type: "rounded".into(),
            separator: "auto".into(),
        }
    }
}

fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn channel(value: f64) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// Linear blend from `a` (`t = 0`) to `b` (`t = 1`).
fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let lerp = |x: u8, y: u8| channel(f64::from(x) + (f64::from(y) - f64::from(x)) * t);
    (lerp(a.0, b.0), lerp(a.1, b.1), lerp(a.2, b.2))
}

fn saturation((r, g, b): Rgb) -> f64 {
    let max = f64::from(r.max(g).max(b));
    let min = f64::from(r.min(g).min(b));
    if max == 0.0 { 0.0 } else { (max - min) / max }
}

fn luminance((r, g, b): Rgb) -> f64 {
    let linear = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Moves `color` toward `toward` (the foreground, which contrasts with the background by
/// construction) until it reaches `min` contrast against `bg`; unchanged if it already does.
fn ensure_contrast(color: Rgb, bg: Rgb, toward: Rgb, min: f64) -> Rgb {
    if contrast(color, bg) >= min {
        return color;
    }
    // Pure white or black is the farthest `toward` can be pushed when the foreground itself is
    // too close to the background to reach `min`.
    let limit = if luminance(bg) < 0.5 {
        (255, 255, 255)
    } else {
        (0, 0, 0)
    };
    let target = if contrast(toward, bg) >= min {
        toward
    } else {
        limit
    };
    (1..=20)
        .map(|step| mix(color, target, f64::from(step) / 20.0))
        .find(|candidate| contrast(*candidate, bg) >= min)
        .unwrap_or(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A muted, earthy wallpaper-derived palette on a near-black background.
    fn earthy() -> TerminalPalette {
        let mut ansi = [(0, 0, 0); 16];
        ansi[0] = (0x0f, 0x10, 0x12);
        ansi[1] = (0x97, 0x68, 0x49);
        ansi[2] = (0x5b, 0x87, 0x78);
        ansi[3] = (0xaa, 0x93, 0x62);
        ansi[4] = (0x6e, 0x6e, 0x6e);
        ansi[5] = (0x62, 0x61, 0x5c);
        ansi[6] = (0xa2, 0xa3, 0xa1);
        ansi[7] = (0xdf, 0xda, 0xd1);
        for i in 8..16 {
            ansi[i] = ansi[i - 8];
        }
        TerminalPalette {
            background: (0x0f, 0x10, 0x12),
            foreground: Some((0xdf, 0xda, 0xd1)),
            ansi,
        }
    }

    fn rgb(value: &str) -> Rgb {
        let n = u32::from_str_radix(value.trim_start_matches('#'), 16).unwrap();
        ((n >> 16) as u8, (n >> 8) as u8, n as u8)
    }

    #[test]
    fn every_text_color_is_readable_against_the_background() {
        let palette = earthy();
        let theme = Theme::from_palette(&palette);
        for color in [
            &theme.file_fg,
            &theme.dir_fg,
            &theme.accent_fg,
            &theme.danger_fg,
            &theme.syntax_type_fg,
            &theme.status_fg,
        ] {
            assert!(
                contrast(rgb(color), palette.background) >= TEXT_CONTRAST - 0.05,
                "{color} is too dim"
            );
        }
    }

    #[test]
    fn a_low_contrast_ansi_color_is_lifted_not_kept() {
        // ANSI 5 (#62615c) is barely above the background.
        let palette = earthy();
        let theme = Theme::from_palette(&palette);
        assert_ne!(theme.doc_fg, hex(palette.ansi[5]));
    }

    #[test]
    fn backgrounds_stay_transparent_except_the_selection() {
        let theme = Theme::from_palette(&earthy());
        assert_eq!(theme.bar_bg, "reset");
        assert_eq!(theme.selection_fg, "keep");
        assert!(theme.selection_bg.starts_with('#'));
    }

    #[test]
    fn the_accent_is_the_most_colorful_non_red_slot() {
        let palette = earthy();
        let theme = Theme::from_palette(&palette);
        // Sand (ANSI 3) is the most saturated of slots 2–6 here.
        assert_eq!(theme.accent_fg, theme.config_fg);
    }

    #[test]
    fn a_light_terminal_gets_dark_text() {
        let mut palette = earthy();
        palette.background = (0xf5, 0xf0, 0xe6);
        palette.foreground = Some((0x2a, 0x2a, 0x2a));
        let theme = Theme::from_palette(&palette);
        assert!(luminance(rgb(&theme.file_fg)) < 0.3);
        assert!(contrast(rgb(&theme.dir_fg), palette.background) >= TEXT_CONTRAST - 0.05);
    }

    #[test]
    fn a_missing_foreground_falls_back_to_ansi_white() {
        let mut palette = earthy();
        palette.foreground = None;
        let theme = Theme::from_palette(&palette);
        assert_eq!(theme.file_fg, hex(palette.ansi[7]));
    }
}
