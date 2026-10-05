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

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};
use shared::DirEntryInfo;
use theming::{Config, Mods, Styles, Theme, TitleAlign};

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

/// The ratatui modifiers for a set of configured text attributes.
pub fn modifiers(mods: Mods) -> Modifier {
    [
        (mods.bold, Modifier::BOLD),
        (mods.italic, Modifier::ITALIC),
        (mods.dim, Modifier::DIM),
        (mods.underline, Modifier::UNDERLINED),
        (mods.reverse, Modifier::REVERSED),
        (mods.crossed_out, Modifier::CROSSED_OUT),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .fold(Modifier::empty(), |acc, (_, m)| acc | m)
}

/// `base` with the configured attributes added on top.
pub fn styled(base: Style, mods: Mods) -> Style {
    base.add_modifier(modifiers(mods))
}

pub fn color(name: &str) -> Color {
    parse_color(name, truecolor())
}

/// The ratatui frame for one of `theming::BORDER_STYLES`; anything else, `none` included (it has
/// no `BorderType`, see [`frame_set`]), is drawn rounded.
pub fn border_type(name: &str) -> BorderType {
    match name.to_lowercase().as_str() {
        "plain" => BorderType::Plain,
        "double" => BorderType::Double,
        "thick" => BorderType::Thick,
        "dashed" => BorderType::LightDoubleDashed,
        "thick-dashed" => BorderType::HeavyDoubleDashed,
        "block" => BorderType::QuadrantOutside,
        "block-inside" => BorderType::QuadrantInside,
        _ => BorderType::Rounded,
    }
}

/// The symbols every frame is drawn with. `none` is blanks rather than no border at all, so a
/// pane keeps its one-cell frame and its rows stay exactly where the mouse code looks for them.
pub fn frame_set(config: &Config) -> border::Set<'static> {
    let name = &config.theme.border_type;
    if name.eq_ignore_ascii_case("none") {
        border::EMPTY
    } else if crate::glyphs::of(config).ascii_borders {
        crate::glyphs::ASCII_BORDER
    } else {
        border_type(name).to_border_set()
    }
}

fn title_alignment(align: TitleAlign) -> Alignment {
    match align {
        TitleAlign::Left => Alignment::Left,
        TitleAlign::Center => Alignment::Center,
        TitleAlign::Right => Alignment::Right,
    }
}

/// The border gradient's two ends, with the `accent`/`focused` keywords resolved.
pub fn gradient_ends(theme: &Theme) -> (Color, Color) {
    (
        color(theme.resolve(&theme.gradient_from)),
        color(theme.resolve(&theme.gradient_to)),
    )
}

/// Paints the border gradient over `area`'s frame, in the theme's colors and direction. Call it
/// once the frame is drawn; whether this frame gets one at all is the caller's to check against
/// `[shape] gradient`.
pub fn paint_gradient(buf: &mut Buffer, area: Rect, config: &Config) {
    let (from, to) = gradient_ends(&config.theme);
    crate::gradient::paint(buf, area, from, to, config.theme.shape.gradient_flow);
}

/// [`paint_gradient`] for a menu or popup: only when `[shape] gradient` reaches floating panels.
pub fn paint_floating_gradient(buf: &mut Buffer, area: Rect, config: &Config) {
    if config.theme.shape.gradient.on_floating() {
        paint_gradient(buf, area, config);
    }
}

/// Dims every cell of `area`, for `[shape] dim_inactive`. Cells an image protocol owns are left
/// alone: their content is an escape sequence, not text a terminal attribute could fade.
pub fn dim(buf: &mut Buffer, area: Rect) {
    for pos in area.intersection(buf.area).positions() {
        let cell = &mut buf[pos];
        if cell.diff_option == CellDiffOption::None {
            cell.modifier.insert(Modifier::DIM);
        }
    }
}

/// The shadow a floating panel casts on what is already drawn, when `[shape] shadow` is on: two
/// columns down its right side and one row under it, both offset by one, as if lit from the top
/// left. Call it once whatever is under the panel has been drawn; it never touches the panel's own
/// cells.
pub fn cast_shadow(buf: &mut Buffer, panel: Rect, config: &Config) {
    if !config.theme.shape.shadow {
        return;
    }
    // Black from outside the 16 themed slots: a terminal that sets slot 0 to its own background
    // would otherwise make the shadow invisible.
    let shade = if truecolor() {
        Color::Rgb(0, 0, 0)
    } else {
        Color::Indexed(16)
    };
    let right = Rect::new(panel.right(), panel.y.saturating_add(1), 2, panel.height);
    let below = Rect::new(panel.x.saturating_add(2), panel.bottom(), panel.width, 1);
    for rect in [right, below] {
        for pos in rect.intersection(buf.area).positions() {
            let cell = &mut buf[pos];
            if cell.diff_option == CellDiffOption::None {
                cell.bg = shade;
                cell.modifier.insert(Modifier::DIM);
            }
        }
    }
}

/// The frame every pane shares. `focused` marks the pane keystrokes currently act on: it gets
/// the bright border and an accent-colored title (bold by default), so the active pane reads as
/// lit up against the dimmer rest.
pub fn themed_block(config: &Config, title: &str, focused: bool) -> Block<'static> {
    let theme = &config.theme;
    let border = if focused {
        color(&theme.border_focused_fg)
    } else {
        color(&theme.border_fg)
    };
    themed_block_with_border(config, title, focused, border)
}

/// Like [`themed_block`], but with the border color already resolved by the caller instead of
/// derived from `focused` — for a shell pane mid-focus-transition (see
/// `shell_layout::render_tree`), whose border isn't simply `border_fg` or `border_focused_fg` but
/// a blend of the two (see [`blend_rgb`]). The title still switches the instant focus does; only
/// the border itself animates, this cycle.
pub fn themed_block_with_border(
    config: &Config,
    title: &str,
    focused: bool,
    border: Color,
) -> Block<'static> {
    let theme = &config.theme;
    let styles = &config.styles;
    let title_style = if focused {
        styled(
            Style::default().fg(color(&theme.accent_fg)),
            styles.title_focused,
        )
    } else {
        styled(Style::default().fg(color(&theme.title_fg)), styles.title)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
        .border_set(frame_set(config))
        .title_alignment(title_alignment(theme.shape.title_align));
    // A frame with no name (a grid tile) stays a plain line rather than gaining two blanks.
    if title.is_empty() {
        block
    } else {
        block.title(Line::from(Span::styled(format!(" {title} "), title_style)))
    }
}

/// Linearly interpolates two resolved colors by `t` (clamped to `[0.0, 1.0]`) — only ever
/// meaningful between two `Color::Rgb` values (a truecolor terminal resolving a hex theme color).
/// Any other resolved variant (a basic ANSI name, or a hex color quantized down to a 256-color
/// index) has no smooth path between two swatches, so this returns `None` rather than guess an
/// in-between shade; the caller falls back to an instant switch between `from` and `to`.
pub fn blend_rgb(from: Color, to: Color, t: f64) -> Option<Color> {
    let (Color::Rgb(fr, fg, fb), Color::Rgb(tr, tg, tb)) = (from, to) else {
        return None;
    };
    let t = t.clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lerp = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8;
    Some(Color::Rgb(lerp(fr, tr), lerp(fg, tg), lerp(fb, tb)))
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

    /// This kind's text attributes in `styles`.
    pub fn mods(self, styles: &Styles) -> Mods {
        match self {
            Self::Dir => styles.dir,
            Self::Source => styles.source,
            Self::Config => styles.config,
            Self::Doc => styles.doc,
            Self::Archive => styles.archive,
            Self::Media => styles.media,
            Self::Other => styles.other,
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

/// Whether `entry` is a file with any execute bit set — styled on top of its kind, like Ranger's
/// bold executables. Always false where the backend has no permission bits.
pub fn is_executable(entry: &DirEntryInfo) -> bool {
    !entry.is_dir && entry.mode.is_some_and(|mode| mode & 0o111 != 0)
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

    fn config_with(border: &str, shape: theming::RawShape) -> Config {
        let mut config = Config::from_sources(None, None, None);
        config.theme.border_type = border.to_string();
        config.theme.shape = config.theme.shape.overlay_raw(&shape);
        config
    }

    #[test]
    fn every_listed_frame_style_draws_differently() {
        let sets: Vec<border::Set<'static>> = theming::BORDER_STYLES
            .iter()
            .map(|name| frame_set(&config_with(name, Default::default())))
            .collect();
        for (i, a) in sets.iter().enumerate() {
            for b in &sets[i + 1..] {
                assert_ne!(a, b, "two styles share one frame");
            }
        }
        assert_eq!(sets.last(), Some(&border::EMPTY), "none is listed last");
    }

    #[test]
    fn a_frame_drawn_without_lines_keeps_its_rows_in_place() {
        let config = config_with("none", Default::default());
        let block = themed_block(&config, "pane", false);
        let area = Rect::new(0, 0, 20, 6);
        assert_eq!(block.inner(area), Rect::new(1, 1, 18, 4));
    }

    #[test]
    fn titles_follow_the_shape_alignment() {
        let shape = theming::RawShape {
            title_align: Some("right".into()),
            ..Default::default()
        };
        let config = config_with("rounded", shape);
        let area = Rect::new(0, 0, 20, 3);
        let mut buf = Buffer::empty(area);
        ratatui::widgets::Widget::render(themed_block(&config, "ab", false), area, &mut buf);
        let top: String = (0..20).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(top.ends_with(" ab ╮"), "{top:?}");
    }

    #[test]
    fn a_shadow_falls_beside_and_under_the_panel_only_when_switched_on() {
        let area = Rect::new(0, 0, 12, 8);
        let panel = Rect::new(2, 1, 5, 3);
        let mut buf = Buffer::empty(area);
        cast_shadow(&mut buf, panel, &config_with("rounded", Default::default()));
        assert_eq!(buf, Buffer::empty(area), "off by default");

        cast_shadow(&mut buf, panel, &config_with("rounded", shape_on()));
        let shaded = |x: u16, y: u16| buf[(x, y)].modifier.contains(Modifier::DIM);
        assert!(shaded(7, 2) && shaded(8, 4), "right of the panel");
        assert!(shaded(4, 4) && shaded(6, 4), "under the panel");
        assert!(!shaded(7, 1), "the top right corner stays lit");
        assert!(
            !shaded(2, 4) && !shaded(3, 4),
            "the bottom left corner stays lit"
        );
        assert!(
            panel
                .positions()
                .all(|pos| !buf[pos].modifier.contains(Modifier::DIM)),
            "never on the panel itself"
        );
    }

    #[test]
    fn a_shadow_at_the_screen_edge_is_clipped_not_a_panic() {
        let area = Rect::new(0, 0, 10, 5);
        let config = config_with("rounded", shape_on());
        let mut buf = Buffer::empty(area);
        cast_shadow(&mut buf, area, &config);
        cast_shadow(&mut buf, Rect::new(8, 3, 2, 2), &config);
        dim(&mut buf, Rect::new(5, 2, 50, 50));
        assert!(buf[(9, 4)].modifier.contains(Modifier::DIM));
    }

    #[test]
    fn floating_panels_get_the_gradient_only_when_the_scope_reaches_them() {
        if !truecolor() {
            return; // hex colors are quantized and cannot blend; nothing to see
        }
        let area = Rect::new(0, 0, 10, 4);
        for (scope, painted) in [("focused", false), ("panes", false), ("all", true)] {
            let shape = theming::RawShape {
                gradient: Some(scope.into()),
                ..Default::default()
            };
            let config = config_with("rounded", shape);
            let mut buf = Buffer::empty(area);
            paint_floating_gradient(&mut buf, area, &config);
            assert_eq!(buf[(0, 0)].fg != Color::Reset, painted, "{scope}");
        }
    }

    #[test]
    fn the_gradient_ends_follow_the_keywords() {
        let mut config = config_with("rounded", Default::default());
        config.theme.accent_fg = "red".into();
        config.theme.border_focused_fg = "blue".into();
        assert_eq!(gradient_ends(&config.theme), (Color::Red, Color::Blue));
        config.theme.gradient_from = "green".into();
        assert_eq!(gradient_ends(&config.theme).0, Color::Green);
    }

    fn shape_on() -> theming::RawShape {
        theming::RawShape {
            shadow: Some(true),
            ..Default::default()
        }
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
    fn modifiers_map_each_configured_attribute() {
        assert_eq!(modifiers(Mods::default()), Modifier::empty());
        let all = Mods::parse(&[
            "bold",
            "italic",
            "dim",
            "underline",
            "reverse",
            "strikethrough",
        ]);
        assert_eq!(
            modifiers(all),
            Modifier::BOLD
                | Modifier::ITALIC
                | Modifier::DIM
                | Modifier::UNDERLINED
                | Modifier::REVERSED
                | Modifier::CROSSED_OUT
        );
        assert_eq!(modifiers(Mods::parse(&["italic"])), Modifier::ITALIC);
    }

    #[test]
    fn each_file_kind_reads_its_own_style() {
        let styles = Styles::default();
        assert!(FileKind::Dir.mods(&styles).bold);
        assert!(!FileKind::Doc.mods(&styles).bold);
    }

    #[test]
    fn only_non_directories_with_an_execute_bit_are_executable() {
        let mut e = file("run.sh", false);
        e.mode = Some(0o755);
        assert!(is_executable(&e));
        e.mode = Some(0o644);
        assert!(!is_executable(&e));
        e.mode = None;
        assert!(!is_executable(&e));
        let mut dir = file("bin", true);
        dir.mode = Some(0o755);
        assert!(
            !is_executable(&dir),
            "a directory's x bit means searchable, not executable"
        );
    }

    #[test]
    fn keep_leaves_the_selected_row_in_its_own_color() {
        let mut theme = Theme::default();
        assert_eq!(selection_fg(&theme), None);
        theme.selection_fg = "white".into();
        assert_eq!(selection_fg(&theme), Some(Color::White));
    }

    #[test]
    fn blend_rgb_reaches_both_endpoints() {
        let from = Color::Rgb(0, 0, 0);
        let to = Color::Rgb(255, 100, 50);
        assert_eq!(blend_rgb(from, to, 0.0), Some(from));
        assert_eq!(blend_rgb(from, to, 1.0), Some(to));
    }

    #[test]
    fn blend_rgb_refuses_to_guess_across_named_or_indexed_colors() {
        assert_eq!(blend_rgb(Color::White, Color::Rgb(0, 0, 0), 0.5), None);
        assert_eq!(
            blend_rgb(Color::Indexed(16), Color::Indexed(200), 0.5),
            None
        );
    }

    proptest::proptest! {
        /// Every blended channel stays within the (possibly reversed) span its two endpoints
        /// bound — a blend can brighten or dim a channel, but never overshoot past either swatch.
        #[test]
        fn blend_rgb_never_overshoots_either_endpoint(
            fr in 0u8..=255, fg in 0u8..=255, fb in 0u8..=255,
            tr in 0u8..=255, tg in 0u8..=255, tb in 0u8..=255,
            t in 0.0f64..=1.0,
        ) {
            let Some(Color::Rgb(r, g, b)) = blend_rgb(Color::Rgb(fr, fg, fb), Color::Rgb(tr, tg, tb), t) else {
                unreachable!("both endpoints are Color::Rgb");
            };
            let in_span = |v: u8, a: u8, b: u8| v >= a.min(b) && v <= a.max(b);
            proptest::prop_assert!(in_span(r, fr, tr));
            proptest::prop_assert!(in_span(g, fg, tg));
            proptest::prop_assert!(in_span(b, fb, tb));
        }
    }
}
