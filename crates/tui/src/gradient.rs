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

//! Paints a two-color gradient around a rect's border and title, in one continuous sweep starting
//! at the top-left corner — part of the cinematic layer, alongside `anim`'s time-based curves.
//!
//! This deliberately recolors every cell along the perimeter, title text included, rather than
//! trying to tell a border glyph from a title glyph: the effect is meant to read as one ribbon of
//! color running around the whole frame, not a solid border with an oddly-excluded title. `Block`
//! (and its title) must already be rendered into the buffer before [`paint`] runs.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

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

/// Recolors `area`'s perimeter in `buf` as a gradient from `from` to `to`. A no-op wherever
/// [`blend_rgb`] can't blend the two colors (anything but two resolved `Color::Rgb` values) — the
/// caller's flat border color is left exactly as `Block` already drew it.
pub fn paint(buf: &mut Buffer, area: Rect, from: Color, to: Color) {
    let cells = perimeter_cells(area);
    let total = cells.len();
    for (i, (x, y)) in cells.into_iter().enumerate() {
        if !buf.area.contains(ratatui::layout::Position::new(x, y)) {
            continue;
        }
        if let Some(color) = blend_rgb(from, to, fraction(i, total)) {
            buf[(x, y)].set_fg(color);
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
        paint(&mut buf, area, Color::White, Color::Rgb(0, 0, 0));
        for cell in buf.content.iter() {
            assert_eq!(cell.fg, Color::Reset);
        }
    }

    #[test]
    fn paint_reaches_both_endpoint_colors_at_the_start_and_end_of_the_sweep() {
        let area = Rect::new(0, 0, 6, 4);
        let mut buf = Buffer::empty(area);
        let (from, to) = (Color::Rgb(0, 0, 0), Color::Rgb(255, 200, 100));
        paint(&mut buf, area, from, to);
        let cells = perimeter_cells(area);
        let first = cells[0];
        let last = *cells.last().expect("area is at least 2x2");
        assert_eq!(buf[first].fg, from);
        assert_eq!(
            buf[last].fg,
            blend_rgb(from, to, fraction(cells.len() - 1, cells.len())).unwrap()
        );
    }

    proptest::proptest! {
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
