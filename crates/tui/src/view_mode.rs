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

//! The browser's switchable views (`V`) and the pure geometry of the filmstrip one: where the big
//! preview and the strip of neighbour thumbnails go, which entries the strip shows, and which
//! cell a click landed on. Like `browser_mouse` and `context_menu`, nothing here touches a
//! terminal, so the rectangles `draw` paints and the ones a click is tested against come from one
//! function and cannot disagree.

use std::ops::Range;

use ratatui::layout::{Margin, Position, Rect};

/// How the browsed folder is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// The usual name list in miller columns.
    #[default]
    List,
    /// The selected file as large as the window allows, with a strip of its neighbours below.
    Filmstrip,
}

impl ViewMode {
    /// The view `V` switches to; the last one wraps round to the first.
    #[must_use]
    pub fn cycled(self) -> Self {
        match self {
            Self::List => Self::Filmstrip,
            Self::Filmstrip => Self::List,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Filmstrip => "filmstrip",
        }
    }
}

/// Below this the strip is dropped and the preview takes the whole body: a strip needs room for a
/// readable thumbnail and the preview above it would be left with a sliver.
const MIN_BODY_HEIGHT: u16 = 12;
const MIN_BODY_WIDTH: u16 = 24;
const STRIP_MIN_HEIGHT: u16 = 5;
const STRIP_MAX_HEIGHT: u16 = 9;
/// Blank columns between two thumbnails.
const CELL_GAP: u16 = 1;
/// A terminal cell is about twice as tall as wide, and photographs are about 3:2, so a thumbnail
/// `h` rows tall is drawn `3 * h` columns wide.
const CELL_ASPECT: u16 = 3;
const CELL_MIN_WIDTH: u16 = 8;

/// Where everything of the filmstrip view goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilmstripLayout {
    /// The selected entry's preview.
    pub big: Rect,
    /// The strip's frame; empty when the body is too small to hold one.
    pub strip: Rect,
    /// One rectangle per thumbnail slot, left to right, each a thumbnail with its name row at the
    /// bottom. Never more than fit inside `strip`.
    pub cells: Vec<Rect>,
}

impl FilmstripLayout {
    /// Splits `body` (the area the three columns would have filled).
    pub fn split(body: Rect) -> Self {
        if body.height < MIN_BODY_HEIGHT || body.width < MIN_BODY_WIDTH {
            return Self {
                big: body,
                strip: Rect::new(body.x, body.bottom(), body.width, 0),
                cells: Vec::new(),
            };
        }
        let strip_height = (body.height / 4).clamp(STRIP_MIN_HEIGHT, STRIP_MAX_HEIGHT);
        let big = Rect::new(body.x, body.y, body.width, body.height - strip_height);
        let strip = Rect::new(body.x, big.bottom(), body.width, strip_height);
        let inner = strip.inner(Margin::new(1, 1));
        let cell_height = inner.height;
        // The bottom row of a cell is its name; the rest is the picture.
        let picture_height = cell_height.saturating_sub(1);
        let cell_width = (picture_height * CELL_ASPECT).max(CELL_MIN_WIDTH);
        let mut capacity = (inner.width + CELL_GAP) / (cell_width + CELL_GAP);
        // An odd count keeps the selection in the middle of the strip when it can be.
        if capacity > 1 && capacity.is_multiple_of(2) {
            capacity -= 1;
        }
        let used = capacity * cell_width + capacity.saturating_sub(1) * CELL_GAP;
        let start_x = inner.x + inner.width.saturating_sub(used) / 2;
        let cells = (0..capacity)
            .map(|i| {
                Rect::new(
                    start_x + i * (cell_width + CELL_GAP),
                    inner.y,
                    cell_width,
                    cell_height,
                )
            })
            .collect();
        Self { big, strip, cells }
    }

    /// The part of a cell that holds the picture (everything above its name row).
    pub fn picture(cell: Rect) -> Rect {
        Rect::new(cell.x, cell.y, cell.width, cell.height.saturating_sub(1))
    }

    /// The row of a cell that holds the entry's name.
    pub fn label(cell: Rect) -> Rect {
        Rect::new(
            cell.x,
            cell.bottom().saturating_sub(1),
            cell.width,
            u16::from(cell.height > 0),
        )
    }

    /// The entries the strip shows when `selected` is the cursor in a listing of `len`: a window
    /// of at most `slots` entries that keeps the cursor in the middle, sliding to the edge of the
    /// listing rather than showing empty slots.
    pub fn window(selected: usize, len: usize, slots: usize) -> Range<usize> {
        let slots = slots.min(len);
        let start = selected
            .saturating_sub(slots / 2)
            .min(len.saturating_sub(slots));
        start..start + slots
    }

    /// The entry index under `pos`, given the `window` the strip was drawn with.
    pub fn entry_at(&self, window: &Range<usize>, pos: Position) -> Option<usize> {
        let slot = self.cells.iter().position(|cell| cell.contains(pos))?;
        let index = window.start + slot;
        window.contains(&index).then_some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_view_cycle_comes_back_to_where_it_began() {
        assert_eq!(ViewMode::List.cycled(), ViewMode::Filmstrip);
        assert_eq!(ViewMode::List.cycled().cycled(), ViewMode::List);
    }

    #[test]
    fn a_small_body_gets_no_strip_and_the_preview_keeps_all_of_it() {
        let body = Rect::new(0, 1, 80, 8);
        let layout = FilmstripLayout::split(body);
        assert_eq!(layout.big, body);
        assert!(layout.cells.is_empty());
        assert_eq!(layout.strip.height, 0);
    }

    #[test]
    fn an_ordinary_terminal_gets_an_odd_strip_centred_on_the_cursor() {
        let layout = FilmstripLayout::split(Rect::new(0, 1, 120, 38));
        assert!(layout.cells.len() > 1);
        assert_eq!(layout.cells.len() % 2, 1);
        assert_eq!(
            FilmstripLayout::window(50, 100, layout.cells.len()).len(),
            layout.cells.len()
        );
    }

    #[test]
    fn the_window_slides_to_the_ends_instead_of_leaving_empty_slots() {
        assert_eq!(FilmstripLayout::window(0, 20, 5), 0..5);
        assert_eq!(FilmstripLayout::window(19, 20, 5), 15..20);
        assert_eq!(FilmstripLayout::window(10, 20, 5), 8..13);
        assert_eq!(FilmstripLayout::window(1, 3, 5), 0..3);
        assert_eq!(FilmstripLayout::window(0, 0, 5), 0..0);
    }

    #[test]
    fn a_click_maps_to_the_entry_drawn_in_that_cell() {
        let layout = FilmstripLayout::split(Rect::new(0, 1, 120, 38));
        let window = FilmstripLayout::window(30, 100, layout.cells.len());
        let cell = layout.cells[2];
        assert_eq!(
            layout.entry_at(&window, Position::new(cell.x + 1, cell.y + 1)),
            Some(window.start + 2)
        );
        assert_eq!(layout.entry_at(&window, Position::new(0, 0)), None);
    }

    #[test]
    fn a_short_listing_leaves_the_unused_slots_unclickable() {
        let layout = FilmstripLayout::split(Rect::new(0, 1, 120, 38));
        let window = FilmstripLayout::window(0, 2, layout.cells.len());
        let cell = layout.cells[3];
        assert_eq!(
            layout.entry_at(&window, Position::new(cell.x, cell.y)),
            None
        );
    }

    proptest! {
        /// Whatever the terminal size, the pieces tile the body without overlapping, and every
        /// thumbnail lies inside the strip's frame.
        #[test]
        fn the_layout_stays_inside_the_body_for_any_size(
            x in 0u16..50, y in 0u16..50, w in 0u16..400, h in 0u16..120,
        ) {
            let body = Rect::new(x, y, w, h);
            let layout = FilmstripLayout::split(body);
            prop_assert!(body.contains(Position::new(layout.big.x, layout.big.y)) || layout.big.is_empty());
            prop_assert_eq!(layout.big.union(layout.strip).intersection(body), layout.big.union(layout.strip));
            prop_assert!(layout.big.intersection(layout.strip).is_empty());
            let inner = layout.strip.inner(Margin::new(1, 1));
            for (i, cell) in layout.cells.iter().enumerate() {
                prop_assert_eq!(cell.intersection(inner), *cell);
                for other in &layout.cells[i + 1..] {
                    prop_assert!(cell.intersection(*other).is_empty());
                }
            }
        }

        /// The cursor is always in the window, the window never runs past the listing, and it
        /// never holds more than the strip has slots.
        #[test]
        fn the_window_always_holds_the_cursor(len in 1usize..500, slots in 0usize..40, sel in 0usize..500) {
            let selected = sel % len;
            let window = FilmstripLayout::window(selected, len, slots);
            prop_assert!(window.end <= len);
            prop_assert!(window.len() <= slots);
            prop_assert_eq!(window.len(), slots.min(len));
            if slots > 0 {
                prop_assert!(window.contains(&selected));
            }
        }

        /// A point resolves to an entry only when it is inside a cell, and the entry is one the
        /// window holds.
        #[test]
        fn a_click_never_resolves_to_an_entry_outside_the_window(
            len in 0usize..200, sel in 0usize..200, px in 0u16..200, py in 0u16..60,
        ) {
            let layout = FilmstripLayout::split(Rect::new(0, 1, 120, 38));
            let selected = if len == 0 { 0 } else { sel % len };
            let window = FilmstripLayout::window(selected, len, layout.cells.len());
            if let Some(index) = layout.entry_at(&window, Position::new(px, py)) {
                prop_assert!(window.contains(&index));
                prop_assert!(index < len);
            }
        }
    }
}
