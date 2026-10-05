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

//! Paints a two-color gradient over a rect's border and title — once around it from the top-left
//! corner, around and back, left to right, or top to bottom (`theming::GradientFlow`) — part of
//! the cinematic layer, alongside `anim`'s time-based curves.
//!
//! This deliberately recolors every cell along the perimeter, title text included, rather than
//! trying to tell a border glyph from a title glyph: the effect is meant to read as one ribbon of
//! color running around the whole frame, not a solid border with an oddly-excluded title. `Block`
//! (and its title) must already be rendered into the buffer before [`paint`] runs.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use theming::GradientFlow;

use crate::style::blend_rgb;

/// `index`'s position along a run of `total` cells, from `0.0` (the first) to `1.0` (the last).
/// `total <= 1` has nowhere to gradient across, so every index reads as the start.
pub fn fraction(index: usize, total: usize) -> f64 {
    if total <= 1 {
        return 0.0;
    }
    (index as f64 / (total - 1) as f64).clamp(0.0, 1.0)
}

/// Every cell on `area`'s perimeter, in order starting at the top-left corner: the top edge left
/// to right, the right edge top to bottom, the bottom edge right to left, the left edge bottom to
/// top — each corner counted once. Empty for an area narrower or shorter than 2 cells, which has
/// no perimeter distinct from its interior.
pub fn perimeter_cells(area: Rect) -> Vec<(u16, u16)> {
    if area.width < 2 || area.height < 2 {
        return Vec::new();
    }
    let (left, top) = (area.x, area.y);
    let right = area.x + area.width - 1;
    let bottom = area.y + area.height - 1;

    let mut cells = Vec::with_capacity(perimeter_len(area));
    cells.extend((left..=right).map(|x| (x, top)));
    cells.extend((top + 1..=bottom).map(|y| (right, y)));
    if bottom > top {
        cells.extend((left..right).rev().map(|x| (x, bottom)));
    }
    if right > left {
        cells.extend((top + 1..bottom).rev().map(|y| (left, y)));
    }
    cells
}

/// How many cells [`perimeter_cells`] returns for `area`, without building the vec — `2*(w+h-2)`
/// for any area at least 2x2, and `0` below that.
fn perimeter_len(area: Rect) -> usize {
    if area.width < 2 || area.height < 2 {
        return 0;
    }
    2 * (area.width as usize + area.height as usize - 2)
}

/// How far along the gradient the `index`th of `total` perimeter cells, at `(x, y)` in `area`, is:
/// `0.0` is `from`, `1.0` is `to`.
fn position(flow: GradientFlow, index: usize, total: usize, (x, y): (u16, u16), area: Rect) -> f64 {
    match flow {
        GradientFlow::Around => fraction(index, total),
        // Up to `to` at the cell halfway round, then back, so the first and last cells match.
        GradientFlow::Mirror => 1.0 - (2.0 * fraction(index, total) - 1.0).abs(),
        GradientFlow::Across => fraction(usize::from(x - area.x), usize::from(area.width)),
        GradientFlow::Down => fraction(usize::from(y - area.y), usize::from(area.height)),
    }
}

/// Recolors `area`'s perimeter in `buf` as a gradient from `from` to `to`, running as `flow` says.
/// A no-op wherever [`blend_rgb`] can't blend the two colors (anything but two resolved
/// `Color::Rgb` values) — the caller's flat border color is left exactly as `Block` already drew
/// it.
pub fn paint(buf: &mut Buffer, area: Rect, from: Color, to: Color, flow: GradientFlow) {
    let cells = perimeter_cells(area);
    let total = cells.len();
    for (i, cell) in cells.into_iter().enumerate() {
        if !buf
            .area
            .contains(ratatui::layout::Position::new(cell.0, cell.1))
        {
            continue;
        }
        if let Some(color) = blend_rgb(from, to, position(flow, i, total, cell, area)) {
            buf[cell].set_fg(color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_reaches_both_ends_and_never_divides_by_zero() {
        assert_eq!(fraction(0, 5), 0.0);
        assert_eq!(fraction(4, 5), 1.0);
        assert_eq!(fraction(2, 5), 0.5);
        assert_eq!(fraction(0, 1), 0.0);
        assert_eq!(fraction(0, 0), 0.0);
    }

    #[test]
    fn perimeter_is_empty_below_two_by_two() {
        assert!(perimeter_cells(Rect::new(0, 0, 1, 5)).is_empty());
        assert!(perimeter_cells(Rect::new(0, 0, 5, 1)).is_empty());
        assert!(perimeter_cells(Rect::new(0, 0, 0, 0)).is_empty());
    }

    #[test]
    fn perimeter_counts_each_corner_once_and_starts_top_left() {
        let area = Rect::new(2, 3, 4, 3);
        let cells = perimeter_cells(area);
        assert_eq!(cells.len(), perimeter_len(area));
        assert_eq!(cells.len(), 2 * (4 + 3 - 2));
        assert_eq!(cells[0], (2, 3));
        // Every cell appears exactly once.
        let unique: std::collections::HashSet<_> = cells.iter().collect();
        assert_eq!(unique.len(), cells.len(), "{cells:?}");
    }

    #[test]
    fn a_two_by_two_area_is_its_own_four_corners() {
        let cells = perimeter_cells(Rect::new(0, 0, 2, 2));
        let mut sorted = cells.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, [(0, 0), (0, 1), (1, 0), (1, 1)]);
    }

    #[test]
    fn every_returned_cell_lies_on_the_areas_edge() {
        let area = Rect::new(5, 5, 10, 6);
        let (left, top) = (area.x, area.y);
        let (right, bottom) = (area.x + area.width - 1, area.y + area.height - 1);
        for (x, y) in perimeter_cells(area) {
            assert!(
                x == left || x == right || y == top || y == bottom,
                "({x},{y}) is not on the edge of {area:?}"
            );
            assert!(area.contains(ratatui::layout::Position::new(x, y)));
        }
    }

    #[test]
    fn paint_leaves_the_buffer_untouched_when_colors_cannot_blend() {
        let area = Rect::new(0, 0, 4, 4);
        let mut buf = Buffer::empty(area);
        for cell in buf.content.iter_mut() {
            cell.set_fg(Color::Reset);
        }
        paint(
            &mut buf,
            area,
            Color::White,
            Color::Rgb(0, 0, 0),
            GradientFlow::Around,
        );
        for cell in buf.content.iter() {
            assert_eq!(cell.fg, Color::Reset);
        }
    }

    #[test]
    fn paint_reaches_both_endpoint_colors_at_the_start_and_end_of_the_sweep() {
        let area = Rect::new(0, 0, 6, 4);
        let mut buf = Buffer::empty(area);
        let (from, to) = (Color::Rgb(0, 0, 0), Color::Rgb(255, 200, 100));
        paint(&mut buf, area, from, to, GradientFlow::Around);
        let cells = perimeter_cells(area);
        let first = cells[0];
        let last = *cells.last().expect("area is at least 2x2");
        assert_eq!(buf[first].fg, from);
        assert_eq!(
            buf[last].fg,
            blend_rgb(from, to, fraction(cells.len() - 1, cells.len())).unwrap()
        );
    }

    const FROM: Color = Color::Rgb(0, 0, 0);
    const TO: Color = Color::Rgb(200, 100, 50);

    fn painted(area: Rect, flow: GradientFlow) -> Buffer {
        let mut buf = Buffer::empty(area);
        paint(&mut buf, area, FROM, TO, flow);
        buf
    }

    #[test]
    fn across_runs_from_the_left_edge_to_the_right_edge() {
        let area = Rect::new(3, 2, 8, 5);
        let buf = painted(area, GradientFlow::Across);
        for y in area.top()..area.bottom() {
            assert_eq!(buf[(area.left(), y)].fg, FROM);
            assert_eq!(buf[(area.right() - 1, y)].fg, TO);
        }
        assert_eq!(
            buf[(5, 2)].fg,
            buf[(5, 6)].fg,
            "a column is one color, top and bottom"
        );
    }

    #[test]
    fn down_runs_from_the_top_edge_to_the_bottom_edge() {
        let area = Rect::new(0, 0, 7, 4);
        let buf = painted(area, GradientFlow::Down);
        for x in area.left()..area.right() {
            assert_eq!(buf[(x, 0)].fg, FROM);
            assert_eq!(buf[(x, 3)].fg, TO);
        }
    }

    #[test]
    fn mirror_reaches_the_end_color_halfway_and_comes_back_without_a_seam() {
        let area = Rect::new(0, 0, 6, 4); // 16 perimeter cells
        let buf = painted(area, GradientFlow::Mirror);
        let cells = perimeter_cells(area);
        assert_eq!(buf[cells[0]].fg, FROM);
        assert_eq!(
            buf[*cells.last().unwrap()].fg,
            FROM,
            "both ends of the sweep match"
        );
        let middle = cells.len() / 2;
        assert!(
            [buf[cells[middle - 1]].fg, buf[cells[middle]].fg]
                .iter()
                .all(|&c| c != FROM),
            "the far side is toward the end color"
        );
    }

    proptest::proptest! {
        /// Every flow keeps each painted cell between the two colors, channel by channel, and
        /// never paints outside the frame's edge.
        #[test]
        fn every_flow_stays_between_its_two_colors_and_on_the_edge(
            w in 0u16..30, h in 0u16..20, flow in 0usize..4,
        ) {
            let area = Rect::new(1, 1, w, h);
            let screen = Rect::new(0, 0, 40, 30);
            let flow = GradientFlow::parse(GradientFlow::NAMES[flow]).unwrap();
            let mut buf = Buffer::empty(screen);
            paint(&mut buf, area, FROM, TO, flow);
            let edge: std::collections::HashSet<_> = perimeter_cells(area).into_iter().collect();
            for pos in screen.positions() {
                let fg = buf[pos].fg;
                if !edge.contains(&(pos.x, pos.y)) {
                    proptest::prop_assert_eq!(fg, Color::Reset);
                } else if let Color::Rgb(r, g, b) = fg {
                    proptest::prop_assert!(r <= 200 && g <= 100 && b <= 50);
                } else {
                    proptest::prop_assert!(false, "an edge cell was left unpainted: {fg:?}");
                }
            }
        }

        #[test]
        fn perimeter_never_leaves_the_area_or_panics(
            x in 0u16..50, y in 0u16..50, w in 0u16..30, h in 0u16..30,
        ) {
            let area = Rect::new(x, y, w, h);
            let cells = perimeter_cells(area);
            proptest::prop_assert_eq!(cells.len(), perimeter_len(area));
            for (cx, cy) in cells {
                proptest::prop_assert!(area.contains(ratatui::layout::Position::new(cx, cy)));
            }
        }

        #[test]
        fn fraction_never_leaves_zero_to_one(index in 0usize..10_000, total in 0usize..10_000) {
            proptest::prop_assert!((0.0..=1.0).contains(&fraction(index, total)));
        }
    }
}
