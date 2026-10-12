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

//! Draws a plugin's canvas (`plugin_api::Canvas`) into the body of a view. The plugin says what
//! each part means (a tone, a hue number, a mark); this module is the one place that turns that
//! into colors, from the user's theme, so a view served by a plugin looks exactly like one drawn
//! by the core. The sixteen hues are sixteen of the theme's own colors, in a fixed order, so
//! changing the theme recolors every view without the plugin knowing.
//!
//! A canvas is laid out for a size and the window can be a frame older than that size, so every
//! part is clipped to the area it is drawn in; a frame (which would lose its corners) or a block
//! that does not fit is simply not drawn until the plugin lays out again. Text takes its color
//! from what is under it: written over a filled block it turns black or white, whichever reads,
//! which is why a plugin never names a text color.

use plugin_api::{Canvas, Cells, HUES, Item, Mark, Paint, Tone};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};
use theming::{Config, Theme};

use crate::atlas_view::{darkened, readable_on};
use crate::glyphs;
use crate::style;

/// The theme color behind hue `n`, wrapping nothing: `n` is below [`HUES`] for a canvas that has
/// been checked, and a larger number is taken modulo [`HUES`] rather than trusted.
pub fn hue_color(theme: &Theme, n: u8) -> Color {
    let name = match n % HUES {
        0 => &theme.dir_fg,
        1 => &theme.file_fg,
        2 => &theme.source_fg,
        3 => &theme.config_fg,
        4 => &theme.doc_fg,
        5 => &theme.archive_fg,
        6 => &theme.media_fg,
        7 => &theme.accent_fg,
        8 => &theme.syntax_keyword_fg,
        9 => &theme.syntax_string_fg,
        10 => &theme.syntax_number_fg,
        11 => &theme.syntax_function_fg,
        12 => &theme.syntax_type_fg,
        13 => &theme.status_fg,
        14 => &theme.border_focused_fg,
        _ => &theme.danger_fg,
    };
    style::color(name)
}

fn tone_color(theme: &Theme, tone: Tone) -> Color {
    style::color(match tone {
        Tone::Normal => &theme.file_fg,
        Tone::Accent => &theme.accent_fg,
        Tone::Danger => &theme.danger_fg,
        Tone::Dim => &theme.status_fg,
    })
}

/// The color a paint stands for on its own: its hue if it has one, else its tone's.
fn paint_color(theme: &Theme, paint: Paint) -> Color {
    paint
        .hue
        .map_or_else(|| tone_color(theme, paint.tone), |n| hue_color(theme, n))
}

/// Whether a paint says nothing, so a frame or path takes the theme's border color.
fn is_plain(paint: Paint) -> bool {
    paint.hue.is_none() && paint.tone == Tone::Normal && paint.mark == Mark::None
}

/// The cells of `at` that lie inside `area` (whose origin the canvas is drawn from), or `None`
/// when none do.
fn clipped(area: Rect, at: Cells) -> Option<Rect> {
    let rect = Rect::new(
        area.x.saturating_add(at.x),
        area.y.saturating_add(at.y),
        at.w,
        at.h,
    );
    let rect = rect.intersection(area);
    (rect.width > 0 && rect.height > 0).then_some(rect)
}

/// What a path cell draws as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece {
    Horizontal,
    Vertical,
    /// Joins a vertical neighbour to a horizontal one: `up` says the vertical neighbour is above
    /// (else below), `left` that the horizontal one is to the left (else to the right).
    Corner {
        up: bool,
        left: bool,
    },
}

/// Every cell a path passes through, in order from its first point, each with what it draws as.
/// Consecutive points share a row or a column (a checked canvas guarantees it); a point that
/// repeats the last adds nothing, and a corner is a cell whose two neighbours lie on different
/// axes.
pub fn path_cells(points: &[(u16, u16)]) -> Vec<((u16, u16), Piece)> {
    let mut cells: Vec<(u16, u16)> = Vec::new();
    for pair in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        let run: Vec<(u16, u16)> = if y0 == y1 {
            let xs: Vec<u16> = (x0.min(x1)..=x0.max(x1)).collect();
            let xs = if x0 <= x1 {
                xs
            } else {
                xs.into_iter().rev().collect()
            };
            xs.into_iter().map(|x| (x, y0)).collect()
        } else {
            let ys: Vec<u16> = (y0.min(y1)..=y0.max(y1)).collect();
            let ys = if y0 <= y1 {
                ys
            } else {
                ys.into_iter().rev().collect()
            };
            ys.into_iter().map(|y| (x0, y)).collect()
        };
        for cell in run {
            if cells.last() != Some(&cell) {
                cells.push(cell);
            }
        }
    }
    (0..cells.len())
        .map(|i| {
            let (x, y) = cells[i];
            let mut up = false;
            let mut down = false;
            let mut left = false;
            let mut right = false;
            let neighbours = [i.checked_sub(1), Some(i + 1)]
                .into_iter()
                .flatten()
                .filter_map(|j| cells.get(j));
            for &(nx, ny) in neighbours {
                up |= nx == x && ny < y;
                down |= nx == x && ny > y;
                left |= ny == y && nx < x;
                right |= ny == y && nx > x;
            }
            let piece = match (up || down, left || right) {
                (true, true) => Piece::Corner { up, left },
                (false, true) => Piece::Horizontal,
                _ => Piece::Vertical,
            };
            ((x, y), piece)
        })
        .collect()
}

fn piece_glyph(piece: Piece, ascii: bool, rounded: bool) -> &'static str {
    match (piece, ascii) {
        (Piece::Horizontal, true) => "-",
        (Piece::Vertical, true) => "|",
        (Piece::Corner { .. }, true) => "+",
        (Piece::Horizontal, false) => "─",
        (Piece::Vertical, false) => "│",
        (
            Piece::Corner {
                up: true,
                left: true,
            },
            false,
        ) => {
            if rounded {
                "╯"
            } else {
                "┘"
            }
        }
        (
            Piece::Corner {
                up: true,
                left: false,
            },
            false,
        ) => {
            if rounded {
                "╰"
            } else {
                "└"
            }
        }
        (
            Piece::Corner {
                up: false,
                left: true,
            },
            false,
        ) => {
            if rounded {
                "╮"
            } else {
                "┐"
            }
        }
        (
            Piece::Corner {
                up: false,
                left: false,
            },
            false,
        ) => {
            if rounded {
                "╭"
            } else {
                "┌"
            }
        }
    }
}

/// Draws `canvas` into `area` of `buf`. `phase` counts animation steps (the travelling mark of a
/// `flow` path advances one cell per step).
pub fn paint(buf: &mut Buffer, area: Rect, canvas: &Canvas, config: &Config, phase: u64) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = &config.theme;
    let g = glyphs::of(config);
    let ascii = g.ascii_borders;
    let rounded = style::border_type(&theme.border_type) == ratatui::widgets::BorderType::Rounded;
    for item in &canvas.items {
        match item {
            Item::Block { at, paint, level } => {
                let Some(rect) = clipped(area, *at) else {
                    continue;
                };
                let block_bottom = u32::from(area.y) + u32::from(at.y) + u32::from(at.h);
                paint_block(buf, rect, *at, block_bottom, *paint, *level, theme);
            }
            Item::Frame { at, paint, title } => {
                // A frame cut by the window would lose its corners and read as damage.
                let whole = Rect::new(
                    area.x + at.x.min(area.width),
                    area.y + at.y.min(area.height),
                    at.w,
                    at.h,
                );
                if whole.intersection(area) != whole || at.w < 2 || at.h < 2 {
                    continue;
                }
                let border = if is_plain(*paint) {
                    style::color(&theme.border_fg)
                } else {
                    match paint.mark {
                        Mark::Here => style::color(&theme.border_focused_fg),
                        Mark::Chosen => style::color(&theme.accent_fg),
                        Mark::None => paint_color(theme, *paint),
                    }
                };
                let mut block = Block::default()
                    .borders(Borders::ALL)
                    .border_set(style::frame_set(config))
                    .border_style(Style::default().fg(border));
                if let Some(title) = title {
                    block = block.title(format!(" {title} "));
                }
                block.render(whole, buf);
            }
            Item::Text {
                x,
                y,
                width,
                text,
                paint,
                bold,
            } => {
                let Some(rect) = clipped(
                    area,
                    Cells {
                        x: *x,
                        y: *y,
                        w: *width,
                        h: 1,
                    },
                ) else {
                    continue;
                };
                let under = buf[(rect.x, rect.y)].bg;
                let fg = if under == Color::Reset {
                    paint_color(theme, *paint)
                } else if paint.tone == Tone::Dim {
                    darkened(readable_on(under), 0.35)
                } else {
                    readable_on(under)
                };
                let mut style = Style::default().fg(fg);
                if *bold || paint.mark == Mark::Chosen {
                    style = style.add_modifier(Modifier::BOLD);
                }
                buf.set_stringn(rect.x, rect.y, text, usize::from(rect.width), style);
            }
            Item::Path {
                points,
                paint,
                flow,
            } => {
                let color = if is_plain(*paint) {
                    style::color(&theme.border_fg)
                } else {
                    paint_color(theme, *paint)
                };
                let line = Style::default().fg(color);
                let cells = path_cells(points);
                for &((x, y), piece) in &cells {
                    let (px, py) = (area.x.saturating_add(x), area.y.saturating_add(y));
                    if px < area.right() && py < area.bottom() {
                        buf[(px, py)]
                            .set_symbol(piece_glyph(piece, ascii, rounded))
                            .set_style(line);
                    }
                }
                if *flow && !cells.is_empty() {
                    let at = usize::try_from(phase % cells.len() as u64).unwrap_or(0);
                    let ((x, y), _) = cells[at];
                    let (px, py) = (area.x.saturating_add(x), area.y.saturating_add(y));
                    if px < area.right() && py < area.bottom() {
                        let mark = if ascii { "*" } else { "●" };
                        buf[(px, py)]
                            .set_symbol(mark)
                            .set_style(Style::default().fg(style::color(&theme.accent_fg)));
                    }
                }
            }
        }
    }
}

/// A filled rectangle. `level` fills from the bottom: the cells above it are the same hue
/// darkened, so a volume reads as a gauge. `rect` is the visible part of the block, which ends at
/// screen row `block_bottom` (exclusive) whether or not its lower rows are visible.
fn paint_block(
    buf: &mut Buffer,
    rect: Rect,
    at: Cells,
    block_bottom: u32,
    paint: Paint,
    level: Option<u8>,
    theme: &Theme,
) {
    let base = match paint.mark {
        Mark::Chosen => style::color(&theme.selection_bg),
        _ => paint_color(theme, paint),
    };
    let dimmed = darkened(base, 0.55);
    let full_rows = level.map_or(u32::from(at.h), |level| {
        (u32::from(at.h) * u32::from(level.min(100))).div_ceil(100)
    });
    for y in rect.top()..rect.bottom() {
        let bg = if block_bottom - u32::from(y) <= full_rows {
            base
        } else {
            dimmed
        };
        for x in rect.left()..rect.right() {
            let cell = &mut buf[(x, y)];
            cell.set_symbol(" ");
            cell.set_style(Style::default().bg(bg));
        }
    }
}

#[cfg(test)]
mod tests {
    use plugin_api::MAX_PATH_POINTS;
    use proptest::prelude::*;

    use super::*;

    fn config() -> Config {
        Config::from_sources(None, None, None)
    }

    fn canvas(width: u16, height: u16, items: Vec<Item>) -> Canvas {
        Canvas {
            width,
            height,
            items,
            claims: Vec::new(),
            hint: None,
        }
    }

    fn block(x: u16, y: u16, w: u16, h: u16, hue: Option<u8>) -> Item {
        Item::Block {
            at: Cells { x, y, w, h },
            paint: Paint {
                hue,
                ..Paint::default()
            },
            level: None,
        }
    }

    fn drawn(area: Rect, canvas: &Canvas) -> Buffer {
        let mut buf = Buffer::empty(area);
        paint(&mut buf, area, canvas, &config(), 0);
        buf
    }

    #[test]
    fn a_block_paints_exactly_its_own_cells_in_its_hue() {
        let area = Rect::new(2, 1, 10, 5);
        let buf = drawn(area, &canvas(10, 5, vec![block(1, 1, 3, 2, Some(7))]));
        let want = hue_color(&config().theme, 7);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let inside = (3..6).contains(&x) && (2..4).contains(&y);
                let bg = buf[(x, y)].bg;
                assert_eq!(bg == want, inside, "({x},{y})");
            }
        }
    }

    #[test]
    fn a_level_fills_from_the_bottom_and_dims_the_rest() {
        let area = Rect::new(0, 0, 4, 4);
        let item = Item::Block {
            at: Cells {
                x: 0,
                y: 0,
                w: 4,
                h: 4,
            },
            paint: Paint {
                hue: Some(0),
                ..Paint::default()
            },
            level: Some(50),
        };
        let buf = drawn(area, &canvas(4, 4, vec![item]));
        let full = hue_color(&config().theme, 0);
        assert_eq!(buf[(0, 3)].bg, full);
        assert_eq!(buf[(0, 2)].bg, full);
        assert_ne!(buf[(0, 1)].bg, full);
        assert_ne!(buf[(0, 0)].bg, full);
        assert_eq!(buf[(0, 0)].bg, buf[(0, 1)].bg);
    }

    #[test]
    fn text_over_a_block_is_written_in_black_or_white_never_the_plugins_choice() {
        let area = Rect::new(0, 0, 8, 1);
        let text = Item::Text {
            x: 0,
            y: 0,
            width: 8,
            text: "usr".into(),
            paint: Paint::default(),
            bold: false,
        };
        let buf = drawn(area, &canvas(8, 1, vec![block(0, 0, 8, 1, Some(2)), text]));
        assert_eq!(buf[(0, 0)].symbol(), "u");
        assert!(matches!(buf[(0, 0)].fg, Color::Black | Color::White));
        assert_eq!(buf[(0, 0)].bg, hue_color(&config().theme, 2));
    }

    #[test]
    fn text_is_cut_to_its_width_and_to_the_area() {
        let area = Rect::new(0, 0, 6, 1);
        let text = |width| Item::Text {
            x: 1,
            y: 0,
            width,
            text: "abcdefgh".into(),
            paint: Paint::default(),
            bold: false,
        };
        let line = |buf: &Buffer| {
            (0..6)
                .map(|x| buf[(x, 0)].symbol().to_string())
                .collect::<String>()
        };
        assert_eq!(line(&drawn(area, &canvas(20, 1, vec![text(3)]))), " abc  ");
        assert_eq!(line(&drawn(area, &canvas(20, 1, vec![text(20)]))), " abcde");
    }

    #[test]
    fn a_frame_that_does_not_fit_the_area_is_not_drawn_at_all() {
        let area = Rect::new(0, 0, 6, 3);
        let frame = Item::Frame {
            at: Cells {
                x: 2,
                y: 0,
                w: 6,
                h: 3,
            },
            paint: Paint::default(),
            title: None,
        };
        let buf = drawn(area, &canvas(10, 3, vec![frame]));
        assert!(buf.content().iter().all(|cell| cell.symbol() == " "));
    }

    #[test]
    fn a_frame_that_fits_has_corners_and_its_title() {
        let area = Rect::new(0, 0, 8, 3);
        let frame = Item::Frame {
            at: Cells {
                x: 0,
                y: 0,
                w: 8,
                h: 3,
            },
            paint: Paint::default(),
            title: Some("sda".into()),
        };
        let buf = drawn(area, &canvas(8, 3, vec![frame]));
        assert_ne!(buf[(0, 0)].symbol(), " ");
        assert_ne!(buf[(7, 2)].symbol(), " ");
        let top: String = (0..8).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(top.contains("sda"), "{top}");
    }

    #[test]
    fn a_path_turns_corners_and_ends_straight() {
        let cells = path_cells(&[(0, 0), (2, 0), (2, 2)]);
        let pieces: Vec<Piece> = cells.iter().map(|(_, piece)| *piece).collect();
        assert_eq!(
            pieces,
            [
                Piece::Horizontal,
                Piece::Horizontal,
                Piece::Corner {
                    up: false,
                    left: true
                },
                Piece::Vertical,
                Piece::Vertical,
            ]
        );
        assert_eq!(cells.first().map(|c| c.0), Some((0, 0)));
        assert_eq!(cells.last().map(|c| c.0), Some((2, 2)));
    }

    #[test]
    fn a_path_going_back_up_and_left_has_the_matching_corner() {
        let cells = path_cells(&[(4, 3), (4, 1), (1, 1)]);
        let corner = cells.iter().find(|(at, _)| *at == (4, 1)).map(|(_, p)| *p);
        assert_eq!(
            corner,
            Some(Piece::Corner {
                up: false,
                left: true
            })
        );
        let cells = path_cells(&[(1, 1), (1, 3), (4, 3)]);
        let corner = cells.iter().find(|(at, _)| *at == (1, 3)).map(|(_, p)| *p);
        assert_eq!(
            corner,
            Some(Piece::Corner {
                up: true,
                left: false
            })
        );
    }

    #[test]
    fn a_flowing_path_carries_one_mark_that_moves_with_the_phase() {
        let area = Rect::new(0, 0, 6, 1);
        let path = Item::Path {
            points: vec![(0, 0), (5, 0)],
            paint: Paint::default(),
            flow: true,
        };
        let canvas = canvas(6, 1, vec![path]);
        let marks = |phase| {
            let mut buf = Buffer::empty(area);
            paint(&mut buf, area, &canvas, &config(), phase);
            (0..6)
                .filter(|&x| buf[(x, 0)].symbol() == "●")
                .collect::<Vec<_>>()
        };
        assert_eq!(marks(0), [0]);
        assert_eq!(marks(3), [3]);
        assert_eq!(marks(6), [0]);
    }

    #[test]
    fn every_hue_is_a_theme_color_and_a_larger_number_wraps_rather_than_panics() {
        let theme = config().theme;
        for n in 0..HUES {
            assert_eq!(hue_color(&theme, n), hue_color(&theme, n + HUES));
        }
    }

    fn arb_item() -> impl Strategy<Value = Item> {
        let cells =
            (0u16..30, 0u16..30, 0u16..30, 0u16..30).prop_map(|(x, y, w, h)| Cells { x, y, w, h });
        let paint = (0u8..HUES, any::<bool>()).prop_map(|(hue, hued)| Paint {
            hue: hued.then_some(hue),
            ..Paint::default()
        });
        prop_oneof![
            (
                cells.clone(),
                paint.clone(),
                proptest::option::of(0u8..=100)
            )
                .prop_map(|(at, paint, level)| Item::Block { at, paint, level }),
            (cells, paint.clone()).prop_map(|(at, paint)| Item::Frame {
                at,
                paint,
                title: Some("t".into())
            }),
            (0u16..30, 0u16..30, 0u16..30, ".{0,12}", paint.clone()).prop_map(
                |(x, y, width, text, paint)| {
                    Item::Text {
                        x,
                        y,
                        width,
                        text,
                        paint,
                        bold: false,
                    }
                }
            ),
            (
                proptest::collection::vec((0u16..30, 0u16..30), 2..MAX_PATH_POINTS),
                paint,
                any::<bool>()
            )
                .prop_map(|(points, paint, flow)| {
                    // Joined at right angles, whatever the points were.
                    let mut joined = vec![points[0]];
                    for (i, &(x, y)) in points.iter().enumerate().skip(1) {
                        let (px, py) = *joined.last().expect("one point at least");
                        joined.push(if i % 2 == 0 { (x, py) } else { (px, y) });
                    }
                    Item::Path {
                        points: joined,
                        paint,
                        flow,
                    }
                }),
        ]
    }

    proptest! {
        // Whatever a canvas holds and whatever room it is drawn in, drawing never panics and never
        // reaches outside the area: a `Buffer` indexed out of its area panics, so surviving this
        // is the proof, and a window that shrank under a canvas is the case it stands for.
        #[test]
        fn painting_any_canvas_in_any_area_stays_inside_it(
            items in proptest::collection::vec(arb_item(), 0..20),
            (ax, ay, aw, ah) in (0u16..5, 0u16..5, 0u16..40, 0u16..40),
            phase in any::<u64>(),
        ) {
            let area = Rect::new(ax, ay, aw, ah);
            let mut buf = Buffer::empty(Rect::new(0, 0, 50, 50));
            let before = buf.clone();
            paint(&mut buf, area, &canvas(30, 30, items), &config(), phase);
            for y in 0..50u16 {
                for x in 0..50u16 {
                    if !area.contains(ratatui::layout::Position::new(x, y)) {
                        prop_assert_eq!(&buf[(x, y)], &before[(x, y)], "({}, {}) is outside", x, y);
                    }
                }
            }
        }

        // A path visits each cell once, in steps of one cell, from its first point to its last.
        #[test]
        fn a_path_is_a_connected_walk_from_its_first_point_to_its_last(
            x in 0u16..40, y in 0u16..40, dx in 0u16..40, dy in 0u16..40,
        ) {
            let cells = path_cells(&[(x, y), (dx, y), (dx, dy)]);
            prop_assert_eq!(cells.first().map(|c| c.0), Some((x, y)));
            prop_assert_eq!(cells.last().map(|c| c.0), Some((dx, dy)));
            for pair in cells.windows(2) {
                let ((ax, ay), (bx, by)) = (pair[0].0, pair[1].0);
                prop_assert_eq!(u32::from(ax.abs_diff(bx)) + u32::from(ay.abs_diff(by)), 1);
            }
        }
    }
}
