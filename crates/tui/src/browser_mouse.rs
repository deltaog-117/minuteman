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

//! Mouse support for the three file columns: where a pointer lands, what a click or a wheel
//! notch then means, and the clock that tells a double-click from two single ones.
//!
//! Everything here is pure — no terminal, no event loop — so it can be tested (and fuzzed)
//! without either. `draw` and the mouse handler both take their rectangles from
//! [`BrowserLayout::split`], which is what keeps a click landing on the row the user was
//! looking at instead of drifting from what was drawn.

use std::time::{Duration, Instant};

use browser::BrowserState;
use crossterm::event::MouseEventKind;
use ratatui::layout::{Constraint, Direction, Layout, Margin, Position, Rect};
use shared::{Vfs, VfsError};

/// Two clicks on the same row this close together are a double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// How many entries one wheel notch moves the selection by. One would feel sluggish in a long
/// directory; a whole page would overshoot in a short one.
const WHEEL_STEP: usize = 3;

/// The screen split the browser is drawn in: header, the three columns, status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserLayout {
    pub header: Rect,
    pub parent: Rect,
    pub current: Rect,
    pub preview: Rect,
    pub status: Rect,
}

impl BrowserLayout {
    pub fn split(area: Rect) -> Self {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Percentage(40),
                Constraint::Percentage(40),
            ])
            .split(rows[1]);
        Self {
            header: rows[0],
            parent: columns[0],
            current: columns[1],
            preview: columns[2],
            status: rows[2],
        }
    }
}

/// Where the rows of a pane's list are drawn: inside the one-cell border every pane has.
fn list_area(pane: Rect) -> Rect {
    pane.inner(Margin::new(1, 1))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Parent,
    Current,
    Preview,
}

/// What the pointer is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// An entry in the left column, by index into the parent directory's listing.
    ParentRow(usize),
    /// An entry in the middle column, by index into the current directory's listing.
    CurrentRow(usize),
    /// Inside a pane but not on an entry: its border, or the blank space under the last row.
    Blank(Pane),
    /// The header, the status bar, or off the layout entirely.
    Elsewhere,
}

impl Hit {
    /// The pane the pointer is in, whether or not it is on an entry.
    pub fn pane(self) -> Option<Pane> {
        match self {
            Hit::ParentRow(_) => Some(Pane::Parent),
            Hit::CurrentRow(_) => Some(Pane::Current),
            Hit::Blank(pane) => Some(pane),
            Hit::Elsewhere => None,
        }
    }
}

/// How much of a list is on hand to hit-test: its entry count and the index of the first entry
/// on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listing {
    pub len: usize,
    pub offset: usize,
}

impl Listing {
    /// A list that is always drawn from its first entry, like the parent column.
    pub fn unscrolled(len: usize) -> Self {
        Self { len, offset: 0 }
    }
}

/// What `pos` is over. Only the parent and current columns have rows; the preview column is
/// never more than [`Hit::Blank`].
pub fn hit_test(layout: &BrowserLayout, pos: Position, parent: Listing, current: Listing) -> Hit {
    let lists = [
        (
            layout.parent,
            parent,
            Hit::ParentRow as fn(usize) -> Hit,
            Pane::Parent,
        ),
        (
            layout.current,
            current,
            Hit::CurrentRow as fn(usize) -> Hit,
            Pane::Current,
        ),
    ];
    for (pane_area, listing, row, pane) in lists {
        if !pane_area.contains(pos) {
            continue;
        }
        let rows = list_area(pane_area);
        if rows.contains(pos) {
            let index = listing.offset + usize::from(pos.y - rows.y);
            if index < listing.len {
                return row(index);
            }
        }
        return Hit::Blank(pane);
    }
    if layout.preview.contains(pos) {
        Hit::Blank(Pane::Preview)
    } else {
        Hit::Elsewhere
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    Single,
    Double,
}

/// Turns a stream of clicks into [`Click`]s. Time is passed in rather than read, so the
/// behaviour at the edges of the window can be tested exactly.
#[derive(Debug, Default)]
pub struct ClickTracker {
    last: Option<(Hit, Instant)>,
}

impl ClickTracker {
    /// Registers a click on `hit` at `now`. It is a double-click if the click before it landed on
    /// the same row within [`DOUBLE_CLICK`]; a double-click is then spent, so a third quick click
    /// starts over as a single instead of chaining into another double.
    pub fn register(&mut self, hit: Hit, now: Instant) -> Click {
        let is_double = self.last.is_some_and(|(previous, at)| {
            previous == hit && now.saturating_duration_since(at) <= DOUBLE_CLICK
        });
        self.last = if is_double { None } else { Some((hit, now)) };
        if is_double {
            Click::Double
        } else {
            Click::Single
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wheel {
    Up,
    Down,
}

impl Wheel {
    /// The wheel direction of a mouse event, or `None` for anything that isn't a wheel notch.
    pub fn of(kind: MouseEventKind) -> Option<Self> {
        match kind {
            MouseEventKind::ScrollUp => Some(Wheel::Up),
            MouseEventKind::ScrollDown => Some(Wheel::Down),
            _ => None,
        }
    }
}

/// The selection after one wheel notch, kept inside a list of `len` entries (`0` when empty).
pub fn wheel_target(selected: usize, len: usize, wheel: Wheel) -> usize {
    let last = len.saturating_sub(1);
    match wheel {
        Wheel::Down => selected.saturating_add(WHEEL_STEP).min(last),
        Wheel::Up => selected.saturating_sub(WHEEL_STEP).min(last),
    }
}

/// Carries out what a click on `hit` means. A click on a row in the middle column selects it and
/// a double-click opens it; a click in the left column goes up a directory and selects the entry
/// clicked.
///
/// # Errors
///
/// Returns the [`VfsError`] from listing a directory that cannot be entered or left.
pub fn apply_click(
    browser: &mut BrowserState,
    vfs: &dyn Vfs,
    hit: Hit,
    click: Click,
) -> Result<(), VfsError> {
    match (hit, click) {
        (Hit::CurrentRow(index), Click::Single) => browser.select_index(index),
        // `enter` does nothing on a file, which is what "directories only" needs until there is
        // somewhere to send a file to be opened.
        (Hit::CurrentRow(index), Click::Double) => {
            browser.select_index(index);
            browser.enter(vfs)?;
        }
        (Hit::ParentRow(index), Click::Single) => {
            let Some(target) = browser.parent_entries().get(index).map(|e| e.path.clone()) else {
                return Ok(());
            };
            browser.leave(vfs)?;
            if let Some(position) = browser
                .current_entries()
                .iter()
                .position(|e| e.path == target)
            {
                browser.select_index(position);
            }
        }
        // The first click already moved the columns, so the second landed on a different entry
        // than the one it was aimed at — acting on it would send the user up twice.
        (Hit::ParentRow(_), Click::Double) => {}
        (Hit::Blank(_) | Hit::Elsewhere, _) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use proptest::prelude::*;
    use ratatui::widgets::Block;
    use shared::LocalVfs;

    use super::*;

    fn at(x: u16, y: u16) -> Position {
        Position::new(x, y)
    }

    fn layout() -> BrowserLayout {
        BrowserLayout::split(Rect::new(0, 0, 100, 30))
    }

    #[test]
    fn split_gives_a_header_a_status_bar_and_20_40_40_columns() {
        let l = layout();
        assert_eq!(l.header, Rect::new(0, 0, 100, 1));
        assert_eq!(l.status, Rect::new(0, 29, 100, 1));
        assert_eq!(l.parent, Rect::new(0, 1, 20, 28));
        assert_eq!(l.current, Rect::new(20, 1, 40, 28));
        assert_eq!(l.preview, Rect::new(60, 1, 40, 28));
    }

    #[test]
    fn list_area_matches_what_a_bordered_block_leaves_inside() {
        let pane = Rect::new(20, 1, 40, 28);
        assert_eq!(list_area(pane), Block::bordered().inner(pane));
    }

    #[test]
    fn a_click_on_the_first_drawn_row_hits_the_entry_at_the_offset() {
        let current = Listing {
            len: 50,
            offset: 12,
        };
        // Column 20 is the pane's left border; the rows start one cell in, one cell down.
        assert_eq!(
            hit_test(&layout(), at(25, 2), Listing::unscrolled(5), current),
            Hit::CurrentRow(12)
        );
        assert_eq!(
            hit_test(&layout(), at(25, 5), Listing::unscrolled(5), current),
            Hit::CurrentRow(15)
        );
    }

    #[test]
    fn borders_blank_space_headers_and_the_preview_are_not_rows() {
        let parent = Listing::unscrolled(2);
        let current = Listing::unscrolled(3);
        let l = layout();
        // The top border, where the pane's title is drawn.
        assert_eq!(
            hit_test(&l, at(25, 1), parent, current),
            Hit::Blank(Pane::Current)
        );
        // Below the last of three rows.
        assert_eq!(
            hit_test(&l, at(25, 5), parent, current),
            Hit::Blank(Pane::Current)
        );
        assert_eq!(
            hit_test(&l, at(5, 4), parent, current),
            Hit::Blank(Pane::Parent)
        );
        assert_eq!(
            hit_test(&l, at(70, 3), parent, current),
            Hit::Blank(Pane::Preview)
        );
        assert_eq!(hit_test(&l, at(50, 0), parent, current), Hit::Elsewhere);
        assert_eq!(hit_test(&l, at(50, 29), parent, current), Hit::Elsewhere);
        assert_eq!(hit_test(&l, at(500, 500), parent, current), Hit::Elsewhere);
    }

    #[test]
    fn the_parent_column_is_hit_from_its_first_entry() {
        let l = layout();
        assert_eq!(
            hit_test(&l, at(3, 2), Listing::unscrolled(4), Listing::unscrolled(0)),
            Hit::ParentRow(0)
        );
        assert_eq!(
            hit_test(&l, at(3, 4), Listing::unscrolled(4), Listing::unscrolled(0)),
            Hit::ParentRow(2)
        );
    }

    #[test]
    fn a_second_click_on_the_same_row_inside_the_window_is_a_double_click() {
        let start = Instant::now();
        let mut clicks = ClickTracker::default();
        assert_eq!(clicks.register(Hit::CurrentRow(2), start), Click::Single);
        assert_eq!(
            clicks.register(Hit::CurrentRow(2), start + Duration::from_millis(400)),
            Click::Double
        );
    }

    #[test]
    fn a_slow_click_or_one_on_another_row_is_a_single_click() {
        let start = Instant::now();
        let mut clicks = ClickTracker::default();
        clicks.register(Hit::CurrentRow(2), start);
        assert_eq!(
            clicks.register(Hit::CurrentRow(2), start + Duration::from_millis(401)),
            Click::Single
        );
        clicks.register(Hit::CurrentRow(2), start + Duration::from_secs(5));
        assert_eq!(
            clicks.register(Hit::CurrentRow(3), start + Duration::from_secs(5)),
            Click::Single
        );
    }

    #[test]
    fn a_third_quick_click_starts_over_instead_of_chaining() {
        let start = Instant::now();
        let mut clicks = ClickTracker::default();
        clicks.register(Hit::CurrentRow(1), start);
        clicks.register(Hit::CurrentRow(1), start + Duration::from_millis(100));
        assert_eq!(
            clicks.register(Hit::CurrentRow(1), start + Duration::from_millis(200)),
            Click::Single
        );
    }

    #[test]
    fn the_wheel_moves_three_entries_and_stops_at_both_ends() {
        assert_eq!(wheel_target(10, 50, Wheel::Down), 13);
        assert_eq!(wheel_target(10, 50, Wheel::Up), 7);
        assert_eq!(wheel_target(48, 50, Wheel::Down), 49);
        assert_eq!(wheel_target(1, 50, Wheel::Up), 0);
        assert_eq!(wheel_target(0, 0, Wheel::Down), 0);
        assert_eq!(wheel_target(0, 0, Wheel::Up), 0);
    }

    #[test]
    fn only_scroll_events_are_wheel_notches() {
        assert_eq!(Wheel::of(MouseEventKind::ScrollUp), Some(Wheel::Up));
        assert_eq!(Wheel::of(MouseEventKind::ScrollDown), Some(Wheel::Down));
        assert_eq!(Wheel::of(MouseEventKind::Moved), None);
        assert_eq!(Wheel::of(MouseEventKind::ScrollLeft), None);
    }

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-browser-mouse-test-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `root/{alpha/inside.txt, beta.txt, sub/note.txt}`.
    fn tree(label: &str) -> PathBuf {
        let root = scratch_dir(label);
        std::fs::create_dir_all(root.join("alpha")).unwrap();
        std::fs::write(root.join("alpha/inside.txt"), "x").unwrap();
        std::fs::write(root.join("beta.txt"), "x").unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/note.txt"), "x").unwrap();
        root
    }

    fn index_of(entries: &[shared::DirEntryInfo], name: &str) -> usize {
        entries.iter().position(|e| e.name == name).unwrap()
    }

    #[test]
    fn clicking_a_row_selects_it() {
        let root = tree("select");
        let mut browser = BrowserState::new(&LocalVfs, root.clone()).unwrap();
        let row = index_of(browser.current_entries(), "beta.txt");

        apply_click(&mut browser, &LocalVfs, Hit::CurrentRow(row), Click::Single).unwrap();

        assert_eq!(browser.selected_entry().unwrap().name, "beta.txt");
        assert_eq!(browser.current_dir(), root);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn double_clicking_a_directory_enters_it() {
        let root = tree("enter");
        let mut browser = BrowserState::new(&LocalVfs, root.clone()).unwrap();
        let row = index_of(browser.current_entries(), "alpha");

        apply_click(&mut browser, &LocalVfs, Hit::CurrentRow(row), Click::Double).unwrap();

        assert_eq!(browser.current_dir(), root.join("alpha"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn double_clicking_a_file_only_selects_it() {
        let root = tree("file");
        let mut browser = BrowserState::new(&LocalVfs, root.clone()).unwrap();
        let row = index_of(browser.current_entries(), "beta.txt");

        apply_click(&mut browser, &LocalVfs, Hit::CurrentRow(row), Click::Double).unwrap();

        assert_eq!(browser.current_dir(), root);
        assert_eq!(browser.selected_entry().unwrap().name, "beta.txt");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn clicking_the_left_column_goes_up_and_selects_that_entry() {
        let root = tree("parent");
        let mut browser = BrowserState::new(&LocalVfs, root.join("sub")).unwrap();
        let row = index_of(browser.parent_entries(), "beta.txt");

        apply_click(&mut browser, &LocalVfs, Hit::ParentRow(row), Click::Single).unwrap();

        assert_eq!(browser.current_dir(), root);
        assert_eq!(browser.selected_entry().unwrap().name, "beta.txt");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_double_click_in_the_left_column_does_not_go_up_twice() {
        let root = tree("parent-double");
        let mut browser = BrowserState::new(&LocalVfs, root.join("sub")).unwrap();
        let row = index_of(browser.parent_entries(), "beta.txt");

        apply_click(&mut browser, &LocalVfs, Hit::ParentRow(row), Click::Double).unwrap();

        assert_eq!(browser.current_dir(), root.join("sub"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn clicking_blank_space_changes_nothing() {
        let root = tree("blank");
        let mut browser = BrowserState::new(&LocalVfs, root.clone()).unwrap();
        let before = browser.selected_index();

        for hit in [
            Hit::Blank(Pane::Current),
            Hit::Blank(Pane::Preview),
            Hit::Elsewhere,
        ] {
            apply_click(&mut browser, &LocalVfs, hit, Click::Double).unwrap();
        }

        assert_eq!(browser.selected_index(), before);
        assert_eq!(browser.current_dir(), root);
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn any_rect() -> impl Strategy<Value = Rect> {
        (0u16..60, 0u16..30, 0u16..300, 0u16..100).prop_map(|(x, y, w, h)| Rect::new(x, y, w, h))
    }

    proptest! {
        /// The three columns and the two bars tile part of the area without overlapping, so a
        /// pointer is never over two regions at once.
        #[test]
        fn layout_regions_stay_inside_the_area_and_never_overlap(area in any_rect()) {
            let l = BrowserLayout::split(area);
            let regions = [l.header, l.parent, l.current, l.preview, l.status];
            for region in regions.into_iter().filter(|r| !r.is_empty()) {
                prop_assert_eq!(region.intersection(area), region);
            }
            for (i, a) in regions.iter().enumerate() {
                for b in &regions[i + 1..] {
                    prop_assert!(!a.intersects(*b), "{a:?} overlaps {b:?}");
                }
            }
        }

        /// A hit never names an entry the list does not have, and always names the row that is
        /// actually drawn under the pointer.
        #[test]
        fn a_row_hit_names_the_entry_drawn_under_the_pointer(
            area in any_rect(),
            x in 0u16..400,
            y in 0u16..150,
            parent_len in 0usize..300,
            current_len in 0usize..300,
            offset in 0usize..300,
        ) {
            let l = BrowserLayout::split(area);
            let pos = at(x, y);
            let parent = Listing::unscrolled(parent_len);
            let current = Listing { len: current_len, offset };
            match hit_test(&l, pos, parent, current) {
                Hit::ParentRow(i) => {
                    prop_assert!(i < parent_len);
                    prop_assert!(list_area(l.parent).contains(pos));
                    prop_assert_eq!(i, usize::from(y - list_area(l.parent).y));
                }
                Hit::CurrentRow(i) => {
                    prop_assert!(i < current_len);
                    prop_assert!(list_area(l.current).contains(pos));
                    prop_assert_eq!(i, offset + usize::from(y - list_area(l.current).y));
                }
                Hit::Blank(_) | Hit::Elsewhere => {}
            }
        }

        /// Every entry that is on screen can be clicked, and clicking its row hits it.
        #[test]
        fn every_visible_row_is_clickable(
            area in any_rect(),
            current_len in 1usize..300,
            offset in 0usize..300,
        ) {
            let l = BrowserLayout::split(area);
            let rows = list_area(l.current);
            for row in 0..rows.height {
                let index = offset + usize::from(row);
                let hit = hit_test(
                    &l,
                    at(rows.x, rows.y + row),
                    Listing::unscrolled(0),
                    Listing { len: current_len, offset },
                );
                if index < current_len && rows.width > 0 {
                    prop_assert_eq!(hit, Hit::CurrentRow(index));
                } else {
                    prop_assert!(!matches!(hit, Hit::CurrentRow(_)));
                }
            }
        }

        /// One notch never leaves the list and always heads the way the wheel turned.
        #[test]
        fn the_wheel_stays_in_range_and_moves_the_right_way(
            len in 0usize..1000,
            selected in 0usize..1000,
        ) {
            let selected = selected.min(len.saturating_sub(1));
            let down = wheel_target(selected, len, Wheel::Down);
            let up = wheel_target(selected, len, Wheel::Up);
            prop_assert!(down >= selected && up <= selected);
            prop_assert!(len == 0 && down == 0 && up == 0 || down < len && up < len);
        }

        /// A double-click needs the previous click on the same row inside the window, and two
        /// doubles never come back to back.
        #[test]
        fn double_clicks_need_the_same_row_inside_the_window(
            clicks in proptest::collection::vec((0usize..3, 0u64..1000), 1..40),
        ) {
            let start = Instant::now();
            let mut tracker = ClickTracker::default();
            let mut elapsed = 0;
            let mut previous: Option<(usize, u64)> = None;
            let mut previous_was_double = false;
            for (row, gap_ms) in clicks {
                elapsed += gap_ms;
                let now = start + Duration::from_millis(elapsed);
                let click = tracker.register(Hit::CurrentRow(row), now);
                if click == Click::Double {
                    let (prev_row, prev_at) = previous.expect("a double needs a click before it");
                    prop_assert_eq!(prev_row, row);
                    prop_assert!(elapsed - prev_at <= 400);
                    prop_assert!(!previous_was_double);
                }
                previous_was_double = click == Click::Double;
                previous = Some((row, elapsed));
            }
        }
    }
}
