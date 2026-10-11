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

//! Draws the atlas (`atlas`) over the whole body in place of the columns, and says where its map
//! sits so a key or a click is mapped onto the tiles that were drawn.
//!
//! Where things sit (`layout`), how a tile is coloured and labelled (`volume_spec`, `row_spec`,
//! `small_spec`) and which text is readable on which colour (`is_light`, `readable_on`) are plain
//! functions so they can be tested without a terminal; `render` only paints them. The map is the
//! `atlas_layout` treemap: a tile is a block of background colour with its label written over it.
//! A volume's block is filled from the bottom by how full the volume is.

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use shared::devices::{Device, Kind as DeviceKind};
use theming::Config;

use crate::atlas::AtlasState;
use crate::atlas_layout::{Item, Tile};
use crate::disk_usage::{Kind, Measure, Row};
use crate::disk_usage_view::{elide_left, header_text, share_percent};
use crate::glyphs;
use crate::hud::{fit_width, format_size};
use crate::style;

/// A volume this full (as a whole percent) is drawn in the danger colour.
const FULL_PERCENT: u8 = 90;

/// The regions of the view within the area it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// The frame drawn around everything.
    pub frame: Rect,
    /// The line of totals.
    pub header: Rect,
    /// Where the tiles go.
    pub map: Rect,
    /// The key hints.
    pub footer: Rect,
}

/// Where everything sits in `area`: a frame around all of it, a line of totals at the top, a line
/// of hints at the bottom (when there is room for one), and the map between.
pub fn layout(area: Rect) -> Layout {
    let inner = area.inner(Margin::new(1, 1));
    let header = Rect {
        height: inner.height.min(1),
        ..inner
    };
    let footer_height = u16::from(inner.height >= 4);
    let footer = Rect {
        y: inner.bottom().saturating_sub(footer_height),
        height: footer_height,
        ..inner
    };
    let map = Rect {
        y: inner.y + header.height,
        height: inner
            .height
            .saturating_sub(header.height)
            .saturating_sub(footer_height),
        ..inner
    };
    Layout {
        frame: area,
        header,
        map,
        footer,
    }
}

/// Whether text on `color` should be dark: true for the light colours. Colours that cannot be
/// resolved to a brightness (the terminal's own default) count as dark.
pub fn is_light(color: Color) -> bool {
    const CUBE: [u32; 6] = [0, 95, 135, 175, 215, 255];
    let luma = |r: u32, g: u32, b: u32| (299 * r + 587 * g + 114 * b) / 1000 > 140;
    match color {
        Color::Rgb(r, g, b) => luma(u32::from(r), u32::from(g), u32::from(b)),
        Color::Indexed(n @ 16..=231) => {
            let i = usize::from(n - 16);
            luma(CUBE[i / 36], CUBE[(i / 6) % 6], CUBE[i % 6])
        }
        Color::Indexed(n @ 232..=255) => 8 + 10 * u32::from(n - 232) > 140,
        Color::Indexed(n) => matches!(n, 2 | 3 | 6 | 7 | 10 | 11 | 14 | 15),
        Color::Yellow
        | Color::Cyan
        | Color::Gray
        | Color::LightRed
        | Color::LightGreen
        | Color::LightYellow
        | Color::LightBlue
        | Color::LightMagenta
        | Color::LightCyan
        | Color::White => true,
        Color::Reset
        | Color::Black
        | Color::Red
        | Color::Green
        | Color::Blue
        | Color::Magenta
        | Color::DarkGray => false,
    }
}

/// The text colour to write on `background`.
pub fn readable_on(background: Color) -> Color {
    if is_light(background) {
        Color::Black
    } else {
        Color::White
    }
}

/// `color` darkened by `amount` (0 to 1). A colour with no RGB value to darken (a basic name, or
/// a 256-colour index) becomes dark gray, the one darker shade that is always there.
pub fn darkened(color: Color, amount: f64) -> Color {
    style::blend_rgb(color, Color::Rgb(0, 0, 0), amount).unwrap_or(Color::DarkGray)
}

/// The rectangle of a tile that is painted: the right column and bottom row are left bare when
/// the tile is big enough to spare them, so neighbouring tiles of one colour stay apart.
pub fn painted(rect: Rect) -> Rect {
    Rect {
        width: if rect.width >= 3 {
            rect.width - 1
        } else {
            rect.width
        },
        height: if rect.height >= 3 {
            rect.height - 1
        } else {
            rect.height
        },
        ..rect
    }
}

/// How a tile looks: its colours and the lines written on it, the first in bold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    /// The colour of the top part of the tile, and of all of it unless `split` says otherwise.
    pub top: Color,
    /// The colour of the bottom part.
    pub bottom: Color,
    /// How many of the painted rows, counting from the top, are `top`.
    pub split: u16,
    pub lines: Vec<String>,
}

/// How many of `rows` painted rows a volume `percent` full leaves free, at the top: the used part
/// grows up from the bottom.
pub fn free_rows(rows: u16, percent: u8) -> u16 {
    let used = (u32::from(rows) * u32::from(percent.min(100)) + 50) / 100;
    rows - u16::try_from(used).unwrap_or(rows).min(rows)
}

fn pin_mark(pinned: bool, g: &glyphs::Glyphs) -> String {
    match (pinned, g.marked.is_empty()) {
        (false, _) => String::new(),
        (true, true) => "* ".to_string(),
        (true, false) => format!("{} ", g.marked),
    }
}

fn device_icon(kind: DeviceKind, ascii: bool) -> &'static str {
    match (kind, ascii) {
        (DeviceKind::Internal, false) => "▣",
        (DeviceKind::Removable, false) => "⏏",
        (DeviceKind::Network, false) => "⇄",
        (DeviceKind::Internal, true) => "[]",
        (DeviceKind::Removable, true) => "^",
        (DeviceKind::Network, true) => "<>",
    }
}

/// A volume's block: filled from the bottom by how full it is, red when nearly full, and dimmed
/// when it is not mounted (nothing to look into).
pub fn volume_spec(
    device: &Device,
    pinned: bool,
    rows: u16,
    width: usize,
    config: &Config,
) -> Spec {
    let (theme, g) = (&config.theme, glyphs::of(config));
    let ascii = g.gauge_on.is_ascii();
    let percent = device.used_percent().unwrap_or(0);
    let mut lines = vec![format!(
        "{}{} {}",
        pin_mark(pinned, g),
        device_icon(device.kind, ascii),
        device.title()
    )];
    lines.push(match device.used_percent() {
        Some(p) => format!("{p}% of {}", format_size(device.total)),
        None => format_size(device.total),
    });
    lines.push(match &device.mount {
        Some(mount) => elide_left(&mount.display().to_string(), width),
        None => "not mounted".to_string(),
    });
    lines.push(format!(
        "{} · {} free",
        device.fs.as_deref().unwrap_or("?"),
        format_size(device.free)
    ));
    if !device.is_mounted() {
        let dim = darkened(style::color(&theme.status_fg), 0.4);
        return Spec {
            top: dim,
            bottom: dim,
            split: rows,
            lines,
        };
    }
    let used = if percent >= FULL_PERCENT {
        style::color(&theme.danger_fg)
    } else {
        style::color(&theme.accent_fg)
    };
    Spec {
        top: darkened(used, 0.7),
        bottom: used,
        split: free_rows(rows, percent),
        lines,
    }
}

/// An entry of the folder zoomed into: coloured by what it is, named, sized, and given its share
/// of the folder.
pub fn row_spec(
    row: &Row,
    pinned: bool,
    measure: Measure,
    total: u64,
    rows: u16,
    config: &Config,
) -> Spec {
    let (theme, g) = (&config.theme, glyphs::of(config));
    let size = row.sizes.of(measure);
    let color = match row.kind {
        Kind::Dir => style::color(&theme.dir_fg),
        Kind::File => style::color(&theme.file_fg),
        Kind::Link | Kind::Other | Kind::OtherFilesystem => {
            darkened(style::color(&theme.status_fg), 0.2)
        }
        Kind::Rest => darkened(style::color(&theme.status_fg), 0.5),
    };
    let name = match row.kind {
        Kind::Dir => format!("{}/", row.name),
        Kind::OtherFilesystem => format!("{}/ (other filesystem)", row.name),
        Kind::Link => format!("{} (link)", row.name),
        Kind::File | Kind::Other | Kind::Rest => row.name.clone(),
    };
    let flag = if row.partial { " !" } else { "" };
    let mut lines = vec![
        format!("{}{name}", pin_mark(pinned, g)),
        format!(
            "{} · {}%{flag}",
            format_size(size),
            share_percent(size, total)
        ),
    ];
    if row.kind == Kind::Dir {
        lines.push(format!("{} items", row.items));
    }
    Spec {
        top: color,
        bottom: color,
        split: rows,
        lines,
    }
}

/// The block that stands for entries too small to have their own.
pub fn small_spec(count: usize, rows: u16, config: &Config) -> Spec {
    let color = darkened(style::color(&config.theme.status_fg), 0.6);
    Spec {
        top: color,
        bottom: color,
        split: rows,
        lines: vec![format!(
            "+{count} smaller {}",
            if count == 1 { "entry" } else { "entries" }
        )],
    }
}

/// Paints one tile: its colour, its label inset by one cell (so a frame fits round it), and, if
/// it is the chosen one, that frame.
fn paint(frame: &mut Frame<'_>, rect: Rect, spec: &Spec, chosen: bool, config: &Config) {
    let body = painted(rect);
    if body.width == 0 || body.height == 0 {
        return;
    }
    let colour_of = |row: u16| {
        if row < spec.split {
            spec.top
        } else {
            spec.bottom
        }
    };
    let buf = frame.buffer_mut();
    for row in 0..body.height {
        let line = Rect::new(body.x, body.y + row, body.width, 1);
        buf.set_style(line, Style::default().bg(colour_of(row)));
    }
    let room = if body.width >= 3 && body.height >= 3 {
        body.inner(Margin::new(1, 1))
    } else {
        body
    };
    if room.width >= 3 {
        for (i, line) in spec.lines.iter().enumerate().take(usize::from(room.height)) {
            let row = u16::try_from(i).unwrap_or(0);
            let background = colour_of(room.y + row - body.y);
            let mut text = Style::default().fg(readable_on(background)).bg(background);
            if i == 0 {
                text = text.add_modifier(Modifier::BOLD);
            }
            buf.set_stringn(
                room.x,
                room.y + row,
                fit_width(line, usize::from(room.width)),
                usize::from(room.width),
                text,
            );
        }
    }
    if !chosen {
        return;
    }
    if body.width >= 3 && body.height >= 3 {
        let set = if glyphs::of(config).ascii_borders {
            glyphs::ASCII_BORDER
        } else {
            border::THICK
        };
        let edge = Style::default()
            .fg(readable_on(spec.top))
            .add_modifier(Modifier::BOLD);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_set(set)
                .border_style(edge),
            body,
        );
    } else {
        // Too small for a frame: swap the colours instead.
        frame
            .buffer_mut()
            .set_style(body, Style::default().add_modifier(Modifier::REVERSED));
    }
}

/// What to say instead of a map when there are no tiles.
fn empty_note(state: &AtlasState) -> &'static str {
    match state.zoomed() {
        None if !state.has_scanned() => "looking for volumes…",
        None => "no volume with a known size",
        Some(view) => match view.end() {
            None => "scanning…",
            Some(crate::disk_usage::End::Unreadable) => "cannot read this folder",
            Some(_) => "nothing here takes any space",
        },
    }
}

pub fn render(frame: &mut Frame<'_>, area: Rect, state: &AtlasState, here: &Path, config: &Config) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let (theme, g) = (&config.theme, glyphs::of(config));
    let places = layout(area);
    let dim = Style::default().fg(style::color(&theme.status_fg));
    let accent = Style::default().fg(style::color(&theme.accent_fg));

    frame.render_widget(Clear, area);
    let title = match (state.zoomed(), state.zoomed_device()) {
        (Some(view), device) => format!(
            "atlas: {}{}",
            device.map_or(String::new(), |d| format!("{} ", d.title())),
            elide_left(
                &view.path().display().to_string(),
                usize::from(area.width).saturating_sub(20)
            )
        ),
        (None, _) => "atlas".to_string(),
    };
    frame.render_widget(style::themed_block(config, &title, true), area);

    let width = usize::from(places.header.width);
    let header = match state.zoomed() {
        Some(view) => header_text(view, g.divider),
        None => {
            let capacity: u64 = state.devices().iter().map(|d| d.total).sum();
            let free: u64 = state
                .devices()
                .iter()
                .filter(|d| d.is_mounted())
                .map(|d| d.free)
                .sum();
            format!(
                "{} {} {} {} in all {} {} free",
                state.devices().len(),
                if state.devices().len() == 1 {
                    "volume"
                } else {
                    "volumes"
                },
                g.divider,
                format_size(capacity),
                g.divider,
                format_size(free)
            )
        }
    };
    frame.render_widget(
        Paragraph::new(Span::styled(fit_width(&header, width), accent)),
        places.header,
    );

    let tiles = state.tiles(places.map);
    if tiles.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(empty_note(state), dim)),
            places.map,
        );
    } else {
        paint_map(frame, &tiles, state, here, config);
    }

    if places.footer.height > 0 {
        let toggle = match state.zoomed().map(|view| view.measure()) {
            Some(Measure::OnDisk) => "  a apparent size",
            Some(Measure::Apparent) => "  a size on disk",
            None => "",
        };
        let hints = format!(
            "arrows move  Enter zoom in  Backspace out  o open  Tab next{toggle}  r rescan  {} you are here",
            pin_mark(true, g).trim_end()
        );
        frame.render_widget(
            Paragraph::new(Span::styled(fit_width(&hints, width), dim)),
            places.footer,
        );
    }
}

fn paint_map(
    frame: &mut Frame<'_>,
    tiles: &[Tile],
    state: &AtlasState,
    here: &Path,
    config: &Config,
) {
    let (chosen, pin) = (state.selected(), state.pin(here));
    for tile in tiles {
        let rows = painted(tile.rect).height;
        let width = usize::from(painted(tile.rect).width.saturating_sub(2));
        let (spec, is_chosen) = match tile.item {
            Item::Small(count) => (small_spec(count, rows, config), false),
            Item::Entry(i) => {
                let pinned = pin == Some(i);
                let spec = match state.zoomed() {
                    None => volume_spec(&state.devices()[i], pinned, rows, width, config),
                    Some(view) => row_spec(
                        &view.rows()[i],
                        pinned,
                        view.measure(),
                        view.total(),
                        rows,
                        config,
                    ),
                };
                (spec, chosen == Some(i))
            }
        };
        paint(frame, tile.rect, &spec, is_chosen, config);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use proptest::prelude::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use shared::LocalVfs;

    use super::*;
    use crate::atlas_layout::treemap;

    fn config() -> Config {
        Config::from_sources(None, None, None)
    }

    fn volume(node: &str, mount: Option<&str>, total: u64, free: u64) -> Device {
        Device {
            node: node.into(),
            disk: node.trim_end_matches(char::is_numeric).into(),
            label: Some(node.to_uppercase()),
            fs: Some("ext4".into()),
            mount: mount.map(PathBuf::from),
            total,
            free,
            kind: DeviceKind::Internal,
        }
    }

    fn screen(width: u16, height: u16, state: &AtlasState, here: &Path) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state, here, &config()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(usize::from(width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect()
    }

    fn state(devices: Vec<Device>) -> (tokio::runtime::Runtime, AtlasState) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let state = AtlasState::with_devices(rt.handle().clone(), Arc::new(LocalVfs), devices);
        (rt, state)
    }

    #[test]
    fn the_map_sits_between_the_totals_line_and_the_hints_inside_a_frame() {
        let places = layout(Rect::new(0, 0, 80, 24));
        assert_eq!(places.frame, Rect::new(0, 0, 80, 24));
        assert_eq!(places.header, Rect::new(1, 1, 78, 1));
        assert_eq!(places.map, Rect::new(1, 2, 78, 20));
        assert_eq!(places.footer, Rect::new(1, 22, 78, 1));
    }

    #[test]
    fn a_short_window_drops_the_hints_before_the_map() {
        let places = layout(Rect::new(0, 0, 80, 5));
        assert_eq!(places.footer.height, 0);
        assert_eq!(places.map.height, 2);
    }

    proptest! {
        /// Whatever the window, the regions stay inside it and do not overlap.
        #[test]
        fn the_regions_never_overlap_or_leave_the_window(w in 0u16..200, h in 0u16..60) {
            let area = Rect::new(2, 1, w, h);
            let p = layout(area);
            for part in [p.header, p.map, p.footer] {
                prop_assert!(part.right() <= area.right() && part.bottom() <= area.bottom());
                prop_assert!(part.width == 0 || part.height == 0 || (part.x >= area.x && part.y >= area.y));
            }
            prop_assert!(p.header.bottom() <= p.map.y || p.map.height == 0 || p.header.height == 0);
            prop_assert!(p.map.bottom() <= p.footer.y || p.footer.height == 0 || p.map.height == 0);
        }

        /// The free part of a volume's block shrinks as the volume fills, and never leaves the
        /// block.
        #[test]
        fn the_free_part_shrinks_as_the_volume_fills(rows in 0u16..80, a in 0u8..=100, b in 0u8..=100) {
            let (lo, hi) = (a.min(b), a.max(b));
            prop_assert!(free_rows(rows, hi) <= free_rows(rows, lo));
            prop_assert!(free_rows(rows, lo) <= rows);
            prop_assert_eq!(free_rows(rows, 100), 0);
            prop_assert_eq!(free_rows(rows, 0), rows);
        }

        /// Drawing never panics, whatever size the window is or how many volumes there are, and
        /// every character it writes stays inside the window.
        #[test]
        fn drawing_survives_any_window_and_any_volumes(
            w in 0u16..140,
            h in 0u16..45,
            totals in prop::collection::vec(0u64..4_000_000_000_000, 0..14),
        ) {
            let devices: Vec<Device> = totals
                .iter()
                .enumerate()
                .map(|(i, &t)| volume(&format!("sd{i}"), (i % 3 != 0).then_some("/mnt/x"), t, t / 3))
                .collect();
            let (_rt, mut atlas) = state(devices);
            atlas.poll(Instant::now(), Path::new("/mnt/x"));
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|frame| render(frame, frame.area(), &atlas, Path::new("/mnt/x"), &config()))
                .unwrap();
        }
    }

    #[test]
    fn light_colours_get_dark_text_and_dark_ones_light_text() {
        assert_eq!(readable_on(Color::Rgb(250, 250, 250)), Color::Black);
        assert_eq!(readable_on(Color::Rgb(10, 10, 40)), Color::White);
        assert_eq!(readable_on(Color::Yellow), Color::Black);
        assert_eq!(readable_on(Color::Blue), Color::White);
        assert_eq!(readable_on(Color::Indexed(15)), Color::Black);
        assert_eq!(readable_on(Color::Indexed(4)), Color::White);
        assert_eq!(readable_on(Color::Indexed(231)), Color::Black);
        assert_eq!(readable_on(Color::Indexed(16)), Color::White);
        assert_eq!(readable_on(Color::Indexed(255)), Color::Black);
        assert_eq!(readable_on(Color::Indexed(232)), Color::White);
        assert_eq!(readable_on(Color::Reset), Color::White);
    }

    #[test]
    fn darkening_works_on_rgb_and_falls_back_to_gray_otherwise() {
        assert_eq!(
            darkened(Color::Rgb(200, 100, 0), 0.5),
            Color::Rgb(100, 50, 0)
        );
        assert_eq!(darkened(Color::Rgb(200, 100, 0), 1.0), Color::Rgb(0, 0, 0));
        assert_eq!(darkened(Color::Red, 0.5), Color::DarkGray);
    }

    #[test]
    fn a_tile_leaves_a_gutter_only_when_it_has_room_for_one() {
        assert_eq!(painted(Rect::new(4, 4, 10, 5)), Rect::new(4, 4, 9, 4));
        assert_eq!(painted(Rect::new(4, 4, 2, 5)), Rect::new(4, 4, 2, 4));
        assert_eq!(painted(Rect::new(4, 4, 10, 2)), Rect::new(4, 4, 9, 2));
        assert_eq!(painted(Rect::new(4, 4, 1, 1)), Rect::new(4, 4, 1, 1));
    }

    #[test]
    fn a_full_volume_is_red_a_mounted_one_is_filled_from_the_bottom_and_an_unmounted_one_is_dim() {
        let config = config();
        let theme = &config.theme;
        let roomy = volume("a", Some("/a"), 1000, 600);
        let spec = volume_spec(&roomy, false, 10, 20, &config);
        assert_eq!(spec.bottom, style::color(&theme.accent_fg));
        assert_eq!(spec.split, 6, "60% free of ten rows");
        let tight = volume("b", Some("/b"), 1000, 20);
        assert_eq!(
            volume_spec(&tight, false, 10, 20, &config).bottom,
            style::color(&theme.danger_fg)
        );
        let off = volume("c", None, 1000, 500);
        let spec = volume_spec(&off, false, 10, 20, &config);
        assert_eq!((spec.top, spec.bottom), (spec.bottom, spec.top));
        assert_ne!(spec.bottom, style::color(&theme.accent_fg));
        assert!(spec.lines.iter().any(|l| l == "not mounted"));
    }

    #[test]
    fn a_volume_block_says_what_it_is_how_full_and_where() {
        let config = config();
        let spec = volume_spec(
            &volume("sda2", Some("/home"), 1 << 40, 1 << 38),
            true,
            6,
            30,
            &config,
        );
        assert!(spec.lines[0].contains("SDA2"), "{:?}", spec.lines);
        assert!(spec.lines[0].starts_with("◆"), "the pin: {:?}", spec.lines);
        assert_eq!(spec.lines[1], "75% of 1.0T");
        assert_eq!(spec.lines[2], "/home");
        assert!(spec.lines[3].contains("256G free"), "{:?}", spec.lines);
    }

    fn sample_row(name: &str, kind: Kind, size: u64) -> Row {
        use crate::disk_usage::Sizes;
        Row {
            name: name.into(),
            path: PathBuf::from("/v").join(name),
            kind,
            sizes: Sizes {
                disk: size,
                apparent: size,
            },
            items: 7,
            partial: false,
        }
    }

    #[test]
    fn a_folder_block_names_sizes_and_shares_it_and_counts_what_is_inside() {
        let config = config();
        let row = sample_row("logs", Kind::Dir, 250);
        let spec = row_spec(&row, false, Measure::OnDisk, 1000, 5, &config);
        assert_eq!(spec.lines[0], "logs/");
        assert_eq!(spec.lines[1], "250B · 25%");
        assert_eq!(spec.lines[2], "7 items");
        let mut partial = sample_row("x", Kind::File, 10);
        partial.partial = true;
        let spec = row_spec(&partial, false, Measure::OnDisk, 10, 5, &config);
        assert!(spec.lines[1].ends_with('!'), "{:?}", spec.lines);
        assert_eq!(spec.lines.len(), 2, "a file has no item count");
        let theme = &config.theme;
        assert_eq!(
            row_spec(&row, false, Measure::OnDisk, 1000, 5, &config).top,
            style::color(&theme.dir_fg)
        );
    }

    #[test]
    fn the_small_block_counts_what_it_folds() {
        let config = config();
        assert_eq!(small_spec(1, 3, &config).lines, ["+1 smaller entry"]);
        assert_eq!(small_spec(40, 3, &config).lines, ["+40 smaller entries"]);
    }

    #[test]
    fn the_screen_shows_every_volume_by_name_with_the_choice_framed() {
        let (_rt, mut atlas) = state(vec![
            volume("sda2", Some("/"), 900_000_000_000, 100_000_000_000),
            volume("sdb1", Some("/mnt/usb"), 100_000_000_000, 90_000_000_000),
        ]);
        atlas.poll(Instant::now(), Path::new("/home/me"));
        let out = screen(100, 24, &atlas, Path::new("/home/me"));
        let all = out.join("\n");
        assert!(out[0].contains("atlas"), "{out:?}");
        assert!(out[1].contains("2 volumes"), "{}", out[1]);
        let (_rt, one) = state(vec![volume("sda2", Some("/"), 900, 100)]);
        let out = screen(100, 24, &one, Path::new("/"));
        assert!(
            out[1].contains("1 volume ") && !out[1].contains("volumes"),
            "{}",
            out[1]
        );
        assert!(all.contains("SDA2") && all.contains("SDB1"), "{all}");
        assert!(all.contains("88% of 838G"), "{all}");
        assert!(all.contains("you are here"), "{all}");
        assert!(
            all.contains('┏') && all.contains('┛'),
            "the chosen block has a frame: {all}"
        );
    }

    #[test]
    fn the_big_volume_gets_the_bigger_block() {
        let (_rt, atlas) = state(vec![
            volume("big", Some("/"), 800, 100),
            volume("small", Some("/s"), 200, 100),
        ]);
        let map = layout(Rect::new(0, 0, 100, 30)).map;
        let tiles = atlas.tiles(map);
        let area = |item| {
            let r = tiles.iter().find(|t| t.item == item).unwrap().rect;
            u32::from(r.width) * u32::from(r.height)
        };
        let (big, small) = (area(Item::Entry(0)), area(Item::Entry(1)));
        assert!(big > 3 * small && big < 5 * small, "{big} vs {small}");
        assert_eq!(tiles, treemap(&atlas.weights(), map));
    }

    #[test]
    fn with_nothing_to_show_it_says_so() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let atlas = AtlasState::new(rt.handle().clone(), Arc::new(LocalVfs));
        let out = screen(80, 12, &atlas, Path::new("/"));
        assert!(out.join("\n").contains("looking for volumes"), "{out:?}");
        let (_rt, atlas) = state(vec![volume("zero", Some("/z"), 0, 0)]);
        let out = screen(80, 12, &atlas, Path::new("/"));
        assert!(
            out.join("\n").contains("no volume with a known size"),
            "{out:?}"
        );
    }

    #[test]
    fn zoomed_in_it_shows_the_folders_rows_as_blocks_and_the_title_names_the_volume() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root =
            std::env::temp_dir().join(format!("minuteman-atlas-view-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("logs")).unwrap();
        std::fs::write(root.join("logs/big.log"), vec![b'x'; 300_000]).unwrap();
        std::fs::write(root.join("notes.txt"), vec![b'x'; 120_000]).unwrap();
        let mut atlas = AtlasState::with_devices(
            rt.handle().clone(),
            Arc::new(LocalVfs),
            vec![volume("sda1", root.to_str(), 10_000_000, 5_000_000)],
        );
        atlas.poll(Instant::now(), &root.join("logs"));
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        atlas.key(enter, layout(Rect::new(0, 0, 100, 24)).map);
        let deadline = Instant::now() + Duration::from_secs(10);
        while atlas.zoomed().is_none_or(|view| view.end().is_none()) {
            atlas.poll(Instant::now(), &root.join("logs"));
            assert!(Instant::now() < deadline, "the scan never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
        let out = screen(100, 24, &atlas, &root.join("logs"));
        let all = out.join("\n");
        assert!(out[0].contains("atlas: SDA1"), "{}", out[0]);
        assert!(all.contains("logs/") && all.contains("notes.txt"), "{all}");
        assert!(all.contains("1 items") || all.contains("2 items"), "{all}");
        assert!(
            all.contains("◆ logs/"),
            "the pin is on the folder browsed: {all}"
        );
        assert!(all.contains("a apparent size"), "{all}");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
