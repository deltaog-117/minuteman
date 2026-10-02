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

//! The browser's switchable views (`V`) and their pure geometry: the tile size the grid and the
//! filmstrip's strip share, where the filmstrip's big preview and strip go, how the grid lays tiles
//! out and scrolls, which tile a click landed on, and the one layout function the drawing code and
//! the mouse code both ask. Like `browser_mouse` and `context_menu`, nothing here touches a
//! terminal, so the rectangles `draw` paints and the ones a click is tested against come from one
//! function and cannot disagree.

use std::ops::Range;

use ratatui::layout::{Margin, Position, Rect, Size};
use theming::ColumnLayout;

use crate::browser_mouse::BrowserLayout;

/// How the browsed folder is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// The usual name list in miller columns.
    #[default]
    List,
    /// Tiles with a thumbnail and a name each, over the whole window, like a graphical file
    /// manager's icon view.
    Grid,
    /// One flat table over the whole window: name, size, exact modified time and permissions.
    Details,
    /// The selected file as large as the window allows, with a strip of its neighbours below.
    Filmstrip,
}

impl ViewMode {
    /// The view `V` switches to; the last one wraps round to the first.
    #[must_use]
    pub fn cycled(self) -> Self {
        match self {
            Self::List => Self::Grid,
            Self::Grid => Self::Details,
            Self::Details => Self::Filmstrip,
            Self::Filmstrip => Self::List,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Grid => "grid",
            Self::Details => "details",
            Self::Filmstrip => "filmstrip",
        }
    }
}

/// The layout every view's mouse handling and drawing agree on. Everything but the details view
/// is the ordinary three-column split; the details view puts one full-width list in place of the
/// columns (the parent and preview get zero-width rectangles, which nothing hit-tests or draws),
/// with a single row above it for the column titles.
pub fn browser_layout(
    area: Rect,
    columns: ColumnLayout,
    show_hud: bool,
    mode: ViewMode,
) -> BrowserLayout {
    let base = BrowserLayout::split(area, columns, show_hud);
    if mode != ViewMode::Details {
        return base;
    }
    let body = body_of(&base);
    let title_row = u16::from(body.height > 2);
    BrowserLayout {
        parent: Rect::new(body.x, body.y, 0, body.height),
        current: Rect::new(
            body.x,
            body.y + title_row,
            body.width,
            body.height - title_row,
        ),
        preview: Rect::new(body.right(), body.y, 0, body.height),
        ..base
    }
}

/// The area the three columns fill, between the header and the status bar.
pub fn body_of(layout: &BrowserLayout) -> Rect {
    layout.parent.union(layout.current).union(layout.preview)
}

/// The details table's title row: the line just above its list.
pub fn details_title_row(layout: &BrowserLayout) -> Rect {
    Rect::new(
        layout.current.x,
        layout.current.y.saturating_sub(1),
        layout.current.width,
        u16::from(layout.current.y > 0),
    )
}

const PICTURE_ASPECT: u16 = 3;
const TILE_MIN_WIDTH: u16 = 10;
/// Blank columns between two tiles of a row.
const TILE_GAP: u16 = 1;

/// The size of a tile whose picture is `picture_rows` tall: a frame round a picture with the
/// entry's name in a row under it. A terminal cell is about twice as tall as wide and photographs
/// are about 3:2, so a picture `h` rows tall is drawn `3 * h` columns wide.
pub fn tile_size(picture_rows: u16) -> Size {
    Size::new(
        (picture_rows * PICTURE_ASPECT + 2).max(TILE_MIN_WIDTH),
        picture_rows + 3,
    )
}

/// The part of a tile that holds the picture: inside the frame, above the name row.
pub fn tile_picture(tile: Rect) -> Rect {
    let inner = tile.inner(Margin::new(1, 1));
    Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(1),
    )
}

/// The row of a tile that holds the entry's name.
pub fn tile_label(tile: Rect) -> Rect {
    let inner = tile.inner(Margin::new(1, 1));
    Rect::new(
        inner.x,
        inner.bottom().saturating_sub(1),
        inner.width,
        u16::from(inner.height > 0),
    )
}

/// Below this the strip is dropped and the preview takes the whole body: a strip needs room for a
/// readable thumbnail and the preview above it would be left with a sliver.
const MIN_BODY_HEIGHT: u16 = 14;
const MIN_BODY_WIDTH: u16 = 24;
const STRIP_MIN_HEIGHT: u16 = 8;
const STRIP_MAX_HEIGHT: u16 = 12;

/// Where everything of the filmstrip view goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilmstripLayout {
    /// The selected entry's preview.
    pub big: Rect,
    /// The strip's frame; empty when the body is too small to hold one.
    pub strip: Rect,
    /// One tile per slot, left to right. Never more than fit inside `strip`.
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
        let tile = tile_size(inner.height.saturating_sub(3));
        let mut capacity = (inner.width + TILE_GAP) / (tile.width + TILE_GAP);
        // An odd count keeps the selection in the middle of the strip when it can be.
        if capacity > 1 && capacity.is_multiple_of(2) {
            capacity -= 1;
        }
        let used = capacity * tile.width + capacity.saturating_sub(1) * TILE_GAP;
        let start_x = inner.x + inner.width.saturating_sub(used) / 2;
        let cells = (0..capacity)
            .map(|i| {
                Rect::new(
                    start_x + i * (tile.width + TILE_GAP),
                    inner.y,
                    tile.width,
                    inner.height,
                )
            })
            .collect();
        Self { big, strip, cells }
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

/// A move of the cursor across the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Left,
    Right,
    Up,
    Down,
}

const GRID_MIN_TILE_HEIGHT: u16 = 7;
const GRID_MAX_TILE_HEIGHT: u16 = 11;

/// Where the grid's tiles go: as many columns as fit, rows from the top one down, the whole block
/// centred across the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridLayout {
    body: Rect,
    /// Tiles per row, at least one.
    pub columns: usize,
    /// Rows that fit in the body, at least one.
    pub rows: usize,
    tile: Size,
    left: u16,
}

impl GridLayout {
    pub fn split(body: Rect) -> Self {
        let tile_height = (body.height / 4).clamp(GRID_MIN_TILE_HEIGHT, GRID_MAX_TILE_HEIGHT);
        let mut tile = tile_size(tile_height - 3);
        // A body smaller than one tile still gets one, clipped, rather than none.
        tile.width = tile.width.min(body.width.max(1));
        tile.height = tile.height.min(body.height.max(1));
        let columns = (usize::from(body.width) + usize::from(TILE_GAP))
            / (usize::from(tile.width) + usize::from(TILE_GAP));
        let columns = columns.max(1);
        let rows = (usize::from(body.height) / usize::from(tile.height)).max(1);
        let used = columns as u16 * tile.width + (columns as u16 - 1) * TILE_GAP;
        Self {
            body,
            columns,
            rows,
            tile,
            left: body.x + body.width.saturating_sub(used) / 2,
        }
    }

    /// Tiles on screen at once.
    pub fn capacity(&self) -> usize {
        self.columns * self.rows
    }

    /// The row `index` is in.
    pub fn row_of(&self, index: usize) -> usize {
        index / self.columns
    }

    /// The first row to draw so that `selected` is on screen, moving as little as it must from
    /// `top` (the row drawn first last time) and never past the last row that fills the body.
    pub fn follow(&self, top: usize, selected: usize, len: usize) -> usize {
        let last_top = len.div_ceil(self.columns).saturating_sub(self.rows);
        let row = self.row_of(selected);
        let top = if row < top {
            row
        } else if row >= top + self.rows {
            row + 1 - self.rows
        } else {
            top
        };
        top.min(last_top)
    }

    /// The entries drawn when row `top` is first.
    pub fn visible(&self, top: usize, len: usize) -> Range<usize> {
        let start = (top * self.columns).min(len);
        start..(start + self.capacity()).min(len)
    }

    /// Where entry `index` is drawn when row `top` is first; `None` if it is off screen.
    pub fn rect_of(&self, index: usize, top: usize) -> Option<Rect> {
        let row = self.row_of(index).checked_sub(top)?;
        if row >= self.rows {
            return None;
        }
        let column = index % self.columns;
        let rect = Rect::new(
            self.left + column as u16 * (self.tile.width + TILE_GAP),
            self.body.y + row as u16 * self.tile.height,
            self.tile.width,
            self.tile.height,
        );
        Some(rect.intersection(self.body))
    }

    /// The entry under `pos`.
    pub fn entry_at(&self, top: usize, len: usize, pos: Position) -> Option<usize> {
        self.visible(top, len)
            .find(|&index| self.rect_of(index, top).is_some_and(|r| r.contains(pos)))
    }

    /// Where the cursor goes after a move: sideways by one entry (wrapping onto the next or
    /// previous row at the ends, as reading order does), up or down by a whole row, and never off
    /// the listing. Down from a short last row lands on its last entry.
    pub fn step(&self, selected: usize, len: usize, step: Step) -> usize {
        if len == 0 {
            return 0;
        }
        let last = len - 1;
        match step {
            Step::Left => selected.saturating_sub(1),
            Step::Right => (selected + 1).min(last),
            Step::Up if selected >= self.columns => selected - self.columns,
            Step::Up => selected,
            Step::Down if selected + self.columns <= last => selected + self.columns,
            Step::Down if self.row_of(selected) < self.row_of(last) => last,
            Step::Down => selected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_view_cycle_visits_every_view_and_comes_back() {
        let mut seen = vec![ViewMode::List];
        let mut mode = ViewMode::List.cycled();
        while mode != ViewMode::List {
            seen.push(mode);
            mode = mode.cycled();
        }
        assert_eq!(
            seen,
            [
                ViewMode::List,
                ViewMode::Grid,
                ViewMode::Details,
                ViewMode::Filmstrip
            ]
        );
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
    fn an_ordinary_terminal_gets_an_odd_strip_of_framed_tiles() {
        let layout = FilmstripLayout::split(Rect::new(0, 1, 120, 38));
        assert!(layout.cells.len() > 1);
        assert_eq!(layout.cells.len() % 2, 1);
        // Every tile has room for a picture and a name inside its frame.
        let tile = layout.cells[0];
        assert!(tile_picture(tile).height >= 3);
        assert_eq!(tile_label(tile).height, 1);
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

    #[test]
    fn the_details_layout_is_one_full_width_list_under_a_title_row() {
        let area = Rect::new(0, 0, 120, 40);
        let list = browser_layout(area, ColumnLayout::ThreePane, true, ViewMode::List);
        let details = browser_layout(area, ColumnLayout::ThreePane, true, ViewMode::Details);
        let body = body_of(&list);
        assert_eq!(details.current.width, body.width);
        assert_eq!(details.current.bottom(), body.bottom());
        assert_eq!(details.current.y, body.y + 1);
        assert_eq!(
            details_title_row(&details),
            Rect::new(body.x, body.y, body.width, 1)
        );
        assert_eq!(details.parent.width, 0);
        assert_eq!(details.preview.width, 0);
        // Header and status stay where they were.
        assert_eq!(details.header, list.header);
        assert_eq!(details.status, list.status);
    }

    #[test]
    fn the_other_views_keep_the_three_columns() {
        let area = Rect::new(0, 0, 120, 40);
        let base = BrowserLayout::split(area, ColumnLayout::ThreePane, true);
        for mode in [ViewMode::List, ViewMode::Grid, ViewMode::Filmstrip] {
            assert_eq!(
                browser_layout(area, ColumnLayout::ThreePane, true, mode),
                base
            );
        }
    }

    #[test]
    fn a_120x38_body_gets_a_grid_of_several_columns_and_rows() {
        let grid = GridLayout::split(Rect::new(0, 1, 120, 38));
        assert!(grid.columns >= 4, "{} columns", grid.columns);
        assert!(grid.rows >= 3, "{} rows", grid.rows);
    }

    #[test]
    fn the_grid_scrolls_only_when_the_cursor_leaves_the_screen() {
        let grid = GridLayout::split(Rect::new(0, 1, 120, 38));
        let (cols, rows) = (grid.columns, grid.rows);
        let len = cols * rows * 4;
        assert_eq!(grid.follow(0, cols * (rows - 1), len), 0);
        assert_eq!(grid.follow(0, cols * rows, len), 1);
        assert_eq!(grid.follow(3, 0, len), 0);
        assert_eq!(grid.follow(2, cols * 3, len), 2);
    }

    #[test]
    fn the_cursor_moves_by_rows_and_stops_at_the_ends() {
        let grid = GridLayout::split(Rect::new(0, 1, 120, 38));
        let c = grid.columns;
        assert_eq!(grid.step(0, 100, Step::Left), 0);
        assert_eq!(grid.step(99, 100, Step::Right), 99);
        assert_eq!(grid.step(0, 100, Step::Up), 0);
        assert_eq!(grid.step(c + 1, 100, Step::Up), 1);
        assert_eq!(grid.step(1, 100, Step::Down), c + 1);
        // A short last row: down lands on its last entry, and once there stays put.
        let len = c * 2 + 1;
        assert_eq!(grid.step(c + 1, len, Step::Down), len - 1);
        assert_eq!(grid.step(len - 1, len, Step::Down), len - 1);
        assert_eq!(grid.step(0, 0, Step::Down), 0);
    }

    #[test]
    fn a_click_on_a_tile_selects_that_entry_and_blank_space_selects_none() {
        let grid = GridLayout::split(Rect::new(0, 1, 120, 38));
        let tile = grid.rect_of(7, 0).expect("entry 7 is on the first screen");
        assert_eq!(
            grid.entry_at(0, 50, Position::new(tile.x + 2, tile.y + 2)),
            Some(7)
        );
        // Past the last entry of a short listing.
        let tile = grid.rect_of(3, 0).expect("entry 3 is on the first screen");
        assert_eq!(
            grid.entry_at(0, 3, Position::new(tile.x + 2, tile.y + 2)),
            None
        );
    }

    proptest! {
        /// Whatever the terminal size, the pieces tile the body without overlapping, and every
        /// tile lies inside the strip's frame.
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

        /// For any body, every drawn tile is inside it, no two overlap, and there is always at
        /// least one column and one row.
        #[test]
        fn grid_tiles_stay_inside_the_body_and_never_overlap(
            x in 0u16..40, y in 0u16..40, w in 0u16..300, h in 0u16..100, top in 0usize..6, len in 0usize..300,
        ) {
            let body = Rect::new(x, y, w, h);
            let grid = GridLayout::split(body);
            prop_assert!(grid.columns >= 1 && grid.rows >= 1);
            let visible = grid.visible(top, len);
            prop_assert!(visible.len() <= grid.capacity());
            let rects: Vec<Rect> = visible
                .clone()
                .filter_map(|i| grid.rect_of(i, top))
                .collect();
            for (i, rect) in rects.iter().enumerate() {
                prop_assert_eq!(rect.intersection(body), *rect);
                for other in &rects[i + 1..] {
                    prop_assert!(rect.intersection(*other).is_empty());
                }
            }
        }

        /// After `follow`, the cursor's row is on screen and the first row is never past the point
        /// where the body would be left half empty.
        #[test]
        fn following_always_keeps_the_cursor_on_screen(
            w in 20u16..300, h in 10u16..100, top in 0usize..40, len in 1usize..2000, sel in 0usize..2000,
        ) {
            let grid = GridLayout::split(Rect::new(0, 1, w, h));
            let selected = sel % len;
            let top = grid.follow(top, selected, len);
            prop_assert!(grid.visible(top, len).contains(&selected));
            prop_assert!(top <= len.div_ceil(grid.columns).saturating_sub(grid.rows));
        }

        /// A cursor move never leaves the listing, and a move then its opposite on a full row
        /// comes back.
        #[test]
        fn cursor_steps_stay_inside_the_listing(
            w in 20u16..300, h in 10u16..100, len in 1usize..500, sel in 0usize..500, which in 0usize..4,
        ) {
            let grid = GridLayout::split(Rect::new(0, 1, w, h));
            let selected = sel % len;
            let step = [Step::Left, Step::Right, Step::Up, Step::Down][which];
            prop_assert!(grid.step(selected, len, step) < len);
            if len > grid.columns * 2 && selected >= grid.columns && selected + grid.columns < len {
                let down = grid.step(selected, len, Step::Down);
                prop_assert_eq!(grid.step(down, len, Step::Up), selected);
            }
        }

        /// A click resolves to an entry only when it is inside that entry's tile.
        #[test]
        fn a_grid_click_resolves_only_inside_a_visible_tile(
            len in 0usize..300, top in 0usize..5, px in 0u16..200, py in 0u16..60,
        ) {
            let grid = GridLayout::split(Rect::new(0, 1, 120, 38));
            let pos = Position::new(px, py);
            if let Some(index) = grid.entry_at(top, len, pos) {
                prop_assert!(index < len);
                prop_assert!(grid.rect_of(index, top).is_some_and(|r| r.contains(pos)));
            }
        }
    }
}
