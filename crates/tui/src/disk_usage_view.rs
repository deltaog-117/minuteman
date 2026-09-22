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

//! Draws the disk usage view (`disk_usage`) over the whole screen, and answers "which row is under
//! the pointer" from the same geometry it draws with.
//!
//! What a row says and where things sit are plain functions (`layout`, `row_at`, `scroll_top`,
//! `share_percent`, `gauge_cells`) so they can be tested without a terminal; `render` only wires
//! them to ratatui. Only the rows on screen are built, since a folder can have twenty thousand.

use ratatui::Frame;
use ratatui::layout::{Margin, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use theming::Config;

use crate::disk_usage::{DiskUsageView, End, Kind, Row};
use crate::glyphs;
use crate::hud::{self, fit_width, format_size, text_width};
use crate::inspect::group_thousands;
use crate::style;

/// Cells the bar takes.
const GAUGE_CELLS: usize = 10;

/// The regions of the view within the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// The frame drawn around everything.
    pub frame: Rect,
    /// The totals line.
    pub header: Rect,
    /// Where the rows go.
    pub rows: Rect,
    /// The key hints.
    pub footer: Rect,
}

/// Where everything sits in `screen`: a frame around all of it, a totals line at the top, a line
/// of key hints at the bottom, and the rows between.
pub fn layout(screen: Rect) -> Layout {
    let inner = screen.inner(Margin::new(2, 1));
    let header = Rect {
        height: inner.height.min(1),
        ..inner
    };
    let footer = Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: inner.height.min(1),
        ..inner
    };
    let rows = Rect {
        y: inner.y + inner.height.min(1),
        height: inner.height.saturating_sub(2),
        ..inner
    };
    Layout {
        frame: screen,
        header,
        rows,
        footer,
    }
}

/// The row under `pointer`, given which row is at the top of the list and how many there are.
pub fn row_at(layout: &Layout, top: usize, len: usize, pointer: Position) -> Option<usize> {
    if !layout.rows.contains(pointer) {
        return None;
    }
    let index = top + usize::from(pointer.y - layout.rows.y);
    (index < len).then_some(index)
}

/// The first row to draw so that `selected` is on screen with as little scrolling as possible,
/// and no blank space is left under the last row when the list is longer than the screen.
pub fn scroll_top(selected: usize, viewport: usize, top: usize, len: usize) -> usize {
    if viewport == 0 || len == 0 {
        return 0;
    }
    let mut top = top.min(len.saturating_sub(viewport));
    if selected < top {
        top = selected;
    } else if selected >= top + viewport {
        top = selected + 1 - viewport;
    }
    top
}

/// `size` as a whole percentage of `total`, rounded down; zero when there is nothing to divide.
pub fn share_percent(size: u64, total: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    ((u128::from(size) * 100) / u128::from(total)).min(100) as u64
}

/// How many of `cells` a bar for `size` fills when the biggest row fills all of them. Anything
/// that takes space gets at least one cell, so a small file is still visibly there.
pub fn gauge_cells(size: u64, biggest: u64, cells: usize) -> usize {
    if size == 0 || biggest == 0 {
        return 0;
    }
    let filled = (u128::from(size) * cells as u128).div_ceil(u128::from(biggest));
    (filled as usize).clamp(1, cells)
}

/// `path` cut from the left to fit `width` cells, so the end (the part that says where you are)
/// stays.
pub fn elide_left(path: &str, width: usize) -> String {
    if text_width(path) <= width {
        return path.to_string();
    }
    let keep = width.saturating_sub(3);
    let tail: String = path
        .chars()
        .rev()
        .scan(0usize, |used, c| {
            *used += text_width(&c.to_string());
            (*used <= keep).then_some(c)
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("...{tail}")
}

/// What follows a row's name: a folder's slash, and why its size may be off.
fn name_of(row: &Row) -> String {
    let mut name = row.name.clone();
    match row.kind {
        Kind::Dir => name.push('/'),
        Kind::OtherFilesystem => name.push_str("/  (other filesystem)"),
        Kind::Link => name.push_str("  (link)"),
        Kind::File | Kind::Other | Kind::Rest => {}
    }
    if row.partial {
        name.push_str("  ! some of it could not be read");
    }
    name
}

/// The totals line: what the rows add up to, how many entries, which size, and how the scan is
/// doing.
pub fn header_text(view: &DiskUsageView, divider: &str) -> String {
    let entries: u64 = view
        .rows()
        .iter()
        .map(|row| row.items + u64::from(row.kind == Kind::Dir))
        .sum();
    let scan = match view.end() {
        None => format!("scanning {}", group_thousands(view.seen())),
        Some(End::Complete) => String::new(),
        Some(End::Truncated) => "stopped early, sizes are lower bounds".to_string(),
        Some(End::Cancelled) => "stopped".to_string(),
        Some(End::Unreadable) => "cannot read this folder".to_string(),
    };
    let mut parts = vec![
        format_size(view.total()),
        format!("{} entries", group_thousands(entries)),
        view.measure().label().to_string(),
    ];
    if !scan.is_empty() {
        parts.push(scan);
    }
    parts.join(&format!(" {divider} "))
}

pub fn render(frame: &mut Frame<'_>, view: &DiskUsageView, config: &Config) {
    let screen = frame.area();
    let places = layout(screen);
    let (theme, g) = (&config.theme, glyphs::of(config));
    frame.render_widget(Clear, screen);
    let title = format!(
        "disk usage: {}",
        elide_left(
            &view.path().display().to_string(),
            usize::from(screen.width).saturating_sub(20)
        )
    );
    frame.render_widget(style::themed_block(config, &title, true), screen);

    let dim = Style::default().fg(style::color(&theme.status_fg));
    let accent = Style::default().fg(style::color(&theme.accent_fg));
    let width = usize::from(places.header.width);
    frame.render_widget(
        Paragraph::new(Span::styled(
            fit_width(&header_text(view, g.divider), width),
            accent,
        )),
        places.header,
    );

    // Everything about which rows are shown is settled here, from the size of the space they are
    // in, and remembered so a key or a click can be mapped to a row on the next event.
    let viewport = usize::from(places.rows.height);
    let rows = view.rows();
    let top = scroll_top(view.selected(), viewport, view.top().get(), rows.len());
    view.top().set(top);
    view.viewport().set(viewport);

    let biggest = rows
        .iter()
        .map(|row| row.sizes.of(view.measure()))
        .max()
        .unwrap_or(0);
    let total = view.total();
    let lines: Vec<Line<'static>> = rows
        .iter()
        .enumerate()
        .skip(top)
        .take(viewport)
        .map(|(index, row)| {
            row_line(
                row,
                index == view.selected(),
                view,
                (biggest, total),
                width,
                config,
            )
        })
        .collect();
    if rows.is_empty() {
        let note = match view.end() {
            None => "scanning...",
            Some(End::Unreadable) => "cannot read this folder",
            _ => "empty",
        };
        frame.render_widget(Paragraph::new(Span::styled(note, dim)), places.rows);
    } else {
        frame.render_widget(Paragraph::new(lines), places.rows);
    }

    let toggle = match view.measure() {
        crate::disk_usage::Measure::OnDisk => "apparent size",
        crate::disk_usage::Measure::Apparent => "size on disk",
    };
    let hints = format!("j/k move  enter open  h up  a {toggle}  r rescan  q close");
    frame.render_widget(
        Paragraph::new(Span::styled(fit_width(&hints, width), dim)),
        places.footer,
    );

    // The bar is drawn for the rows' region rather than the whole frame, so its thumb spans just
    // the list.
    let strip = Rect {
        y: places.rows.y.saturating_sub(1),
        height: places.rows.height + 2,
        ..screen
    };
    hud::render_scrollbar(frame, strip, rows.len(), view.selected(), config);
}

/// One row: `> 1.4G  35% ▰▰▰▰▱▱▱▱▱▱  name/`, thinned out on a narrow screen (the bar goes first,
/// then the percentage).
fn row_line(
    row: &Row,
    selected: bool,
    view: &DiskUsageView,
    (biggest, total): (u64, u64),
    width: usize,
    config: &Config,
) -> Line<'static> {
    let (theme, g) = (&config.theme, glyphs::of(config));
    let size = row.sizes.of(view.measure());
    let dim = Style::default().fg(style::color(&theme.status_fg));
    let background = if selected {
        Style::default().bg(style::color(&theme.selection_bg))
    } else {
        Style::default()
    };
    let name_style = match row.kind {
        Kind::Dir => Style::default().fg(style::color(&theme.dir_fg)),
        Kind::OtherFilesystem | Kind::Rest => dim,
        Kind::File | Kind::Link | Kind::Other => Style::default().fg(style::color(&theme.file_fg)),
    };
    let mut spans = vec![Span::styled(
        if selected {
            format!("{} ", g.stripe)
        } else {
            "  ".to_string()
        },
        Style::default().fg(style::color(&theme.accent_fg)),
    )];
    let mut used = 2;
    let size_text = format!("{:>7} ", format_size(size));
    used += text_width(&size_text);
    spans.push(Span::styled(size_text, dim));
    if width >= used + 6 + 4 {
        let percent = format!("{:>3}% ", share_percent(size, total));
        used += text_width(&percent);
        spans.push(Span::styled(percent, dim));
    }
    if width >= used + GAUGE_CELLS + 2 + 8 {
        let bar = hud::gauge(gauge_cells(size, biggest, GAUGE_CELLS), GAUGE_CELLS, g);
        used += GAUGE_CELLS + 2;
        spans.push(Span::styled(
            format!("{bar}  "),
            Style::default().fg(style::color(&theme.accent_fg)),
        ));
    }
    spans.push(Span::styled(
        fit_width(&name_of(row), width.saturating_sub(used)),
        name_style,
    ));
    Line::from(spans).style(background)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk_usage::{Kind, Measure, Row, Sizes};
    use proptest::prelude::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    #[test]
    fn the_rows_sit_between_the_totals_line_and_the_hints_inside_a_frame() {
        let places = layout(Rect::new(0, 0, 80, 24));
        assert_eq!(places.frame, Rect::new(0, 0, 80, 24));
        assert_eq!(places.header, Rect::new(2, 1, 76, 1));
        assert_eq!(places.rows, Rect::new(2, 2, 76, 20));
        assert_eq!(places.footer, Rect::new(2, 22, 76, 1));
    }

    #[test]
    fn a_tiny_screen_gets_a_layout_without_overlapping_or_underflowing() {
        for (w, h) in [(0, 0), (1, 1), (2, 2), (3, 3), (5, 3), (10, 4), (80, 1)] {
            let places = layout(Rect::new(0, 0, w, h));
            assert!(places.rows.height <= h, "{w}x{h}: {places:?}");
            assert!(places.header.width <= w && places.footer.width <= w);
        }
    }

    #[test]
    fn a_click_maps_to_the_row_drawn_there_and_nothing_outside_the_list() {
        let places = layout(Rect::new(0, 0, 80, 24));
        let at = |x, y| row_at(&places, 5, 100, Position::new(x, y));
        assert_eq!(at(10, 2), Some(5));
        assert_eq!(at(10, 21), Some(24));
        assert_eq!(at(10, 22), None, "the hints line");
        assert_eq!(at(10, 1), None, "the totals line");
        assert_eq!(at(0, 5), None, "the frame");
        // Fewer rows than the screen has room for: the blank space below them is not a row.
        assert_eq!(row_at(&places, 0, 3, Position::new(10, 5)), None);
        assert_eq!(row_at(&places, 0, 3, Position::new(10, 4)), Some(2));
    }

    proptest! {
        /// After scrolling, the selected row is on screen, the view never starts past the point
        /// where the last row is the last one shown, and it does not move when it need not.
        #[test]
        fn the_selected_row_is_always_visible_and_the_view_moves_only_when_it_must(
            len in 1usize..500,
            viewport in 1usize..60,
            top in 0usize..600,
            selected in 0usize..500,
        ) {
            let selected = selected.min(len - 1);
            let now = scroll_top(selected, viewport, top, len);
            prop_assert!(now <= selected && selected < now + viewport);
            prop_assert!(now <= len.saturating_sub(viewport));
            let clamped = top.min(len.saturating_sub(viewport));
            if selected >= clamped && selected < clamped + viewport {
                prop_assert_eq!(now, clamped);
            }
        }

        /// A share is at most the whole; a bar never overflows and grows with the size.
        #[test]
        fn shares_and_bars_stay_in_range_and_grow_with_size(
            a in 0u64..u64::MAX / 2,
            b in 0u64..u64::MAX / 2,
            biggest in 1u64..u64::MAX,
            cells in 1usize..30,
        ) {
            let (lo, hi) = (a.min(b), a.max(b));
            prop_assert!(share_percent(hi, hi.max(1)) <= 100);
            prop_assert!(share_percent(lo, hi.max(1)) <= share_percent(hi, hi.max(1)));
            let (bar_lo, bar_hi) = (gauge_cells(lo.min(biggest), biggest, cells), gauge_cells(hi.min(biggest), biggest, cells));
            prop_assert!(bar_lo <= bar_hi && bar_hi <= cells);
            prop_assert_eq!(gauge_cells(biggest, biggest, cells), cells);
            if lo > 0 {
                prop_assert!(bar_lo >= 1, "something that takes space must show");
            }
        }

        #[test]
        fn eliding_a_path_never_exceeds_the_width_and_keeps_its_end(path in "(/[a-z]{1,8}){1,12}", width in 4usize..60) {
            let cut = elide_left(&path, width);
            prop_assert!(text_width(&cut) <= width.max(text_width(&path).min(width)));
            if text_width(&path) > width {
                prop_assert!(cut.starts_with("..."));
                prop_assert!(path.ends_with(&cut[3..]));
            } else {
                prop_assert_eq!(cut, path);
            }
        }
    }

    #[test]
    fn zero_totals_and_sizes_do_not_divide_by_zero() {
        assert_eq!(share_percent(5, 0), 0);
        assert_eq!(share_percent(0, 100), 0);
        assert_eq!(share_percent(100, 100), 100);
        assert_eq!(gauge_cells(0, 100, 10), 0);
        assert_eq!(gauge_cells(5, 0, 10), 0);
        assert_eq!(gauge_cells(1, 1_000_000, 10), 1);
        assert_eq!(gauge_cells(u64::MAX, u64::MAX, 10), 10);
    }

    // ---- drawing ----------------------------------------------------------------------------

    fn config() -> Config {
        Config::from_sources(None, None, None)
    }

    fn fill(rt: &tokio::runtime::Runtime, root: &std::path::Path) -> DiskUsageView {
        let mut view = DiskUsageView::open(rt.handle().clone(), root.to_path_buf());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while view.end().is_none() {
            view.poll();
            assert!(
                std::time::Instant::now() < deadline,
                "the scan never finished"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        view
    }

    fn scratch(label: &str) -> PathBuf {
        // Tests run in parallel, so each call gets a directory of its own.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "minuteman-usage-view-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn screen(width: u16, height: u16, view: &DiskUsageView, config: &Config) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, view, config)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect()
    }

    fn populated() -> (tokio::runtime::Runtime, PathBuf, DiskUsageView) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("populated");
        std::fs::create_dir_all(root.join("logs")).unwrap();
        std::fs::write(root.join("logs/big.log"), vec![b'x'; 300_000]).unwrap();
        std::fs::write(root.join("notes.txt"), vec![b'x'; 1_200]).unwrap();
        std::fs::write(root.join("empty"), b"").unwrap();
        let view = fill(&rt, &root);
        (rt, root, view)
    }

    #[test]
    fn the_screen_shows_the_path_totals_sizes_shares_bars_and_names_biggest_first() {
        let (_rt, root, view) = populated();
        let out = screen(100, 14, &view, &config());
        let all = out.join("\n");
        assert!(out[0].contains("disk usage:"), "{out:?}");
        assert!(
            out[1].contains("entries") && out[1].contains("on disk"),
            "{out:?}"
        );
        let logs = out
            .iter()
            .position(|l| l.contains("logs/"))
            .expect("the folder row");
        let notes = out
            .iter()
            .position(|l| l.contains("notes.txt"))
            .expect("the file row");
        assert!(logs < notes, "biggest first: {out:?}");
        assert!(
            out[logs].contains('%') && out[logs].contains('▰'),
            "{}",
            out[logs]
        );
        assert!(
            out[logs].contains("▌"),
            "the selected row has the stripe: {}",
            out[logs]
        );
        assert!(all.contains("enter open"), "{all}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_hint_names_the_other_measure_and_the_header_names_the_current_one() {
        let (rt, root, mut view) = populated();
        let text = |view: &DiskUsageView| screen(110, 12, view, &config()).join("\n");
        assert!(text(&view).contains("a apparent size") && text(&view).contains("on disk"));
        view.toggle_measure();
        assert!(text(&view).contains("a size on disk") && text(&view).contains("apparent size"));
        drop(rt);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_ascii_glyph_set_draws_the_whole_view_in_ascii() {
        let (_rt, root, view) = populated();
        let mut ascii = config();
        ascii.ui.glyphs = theming::GlyphSet::Ascii;
        let all = screen(100, 14, &view, &ascii).join("\n");
        assert!(all.is_ascii(), "non-ASCII in {all:?}");
        assert!(
            all.contains('#') || all.contains('-'),
            "the bar is drawn: {all}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_row_that_is_a_link_or_cut_says_so() {
        let mk = |kind, partial| Row {
            name: "thing".into(),
            path: PathBuf::from("/x/thing"),
            kind,
            sizes: Sizes::default(),
            items: 1,
            partial,
        };
        assert_eq!(name_of(&mk(Kind::Dir, false)), "thing/");
        assert_eq!(name_of(&mk(Kind::Link, false)), "thing  (link)");
        assert!(name_of(&mk(Kind::OtherFilesystem, false)).ends_with("(other filesystem)"));
        assert!(name_of(&mk(Kind::Dir, true)).contains("could not be read"));
        assert_eq!(name_of(&mk(Kind::File, false)), "thing");
    }

    #[test]
    fn scrolling_a_long_list_keeps_the_selected_row_on_screen() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("long");
        for i in 0..60 {
            std::fs::write(root.join(format!("file{i:02}")), vec![b'x'; 100 + i]).unwrap();
        }
        let mut view = fill(&rt, &root);
        screen(80, 12, &view, &config());
        view.move_to(45);
        let out = screen(80, 12, &view, &config());
        let name = view.selected_row().unwrap().name.clone();
        assert!(
            out.iter().any(|l| l.contains(&name)),
            "{name} is not on screen: {out:?}"
        );
        assert!(view.top().get() + view.viewport().get() > 45);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_empty_or_unreadable_folder_says_so_and_no_size_of_screen_panics() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("empty");
        let empty = fill(&rt, &root);
        assert!(
            screen(60, 8, &empty, &config())
                .join("\n")
                .contains("empty")
        );
        let gone = fill(&rt, &root.join("missing"));
        assert!(
            screen(60, 8, &gone, &config())
                .join("\n")
                .contains("cannot read")
        );

        let (_rt2, populated_root, view) = populated();
        for (w, h) in [
            (1, 1),
            (3, 2),
            (5, 5),
            (12, 6),
            (20, 4),
            (30, 30),
            (60, 3),
            (140, 40),
        ] {
            screen(w, h, &view, &config());
            screen(w, h, &empty, &config());
        }
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&populated_root).unwrap();
    }

    #[test]
    fn a_scan_that_is_still_running_shows_its_progress() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("running");
        let view = DiskUsageView::open(rt.handle().clone(), root.clone());
        // Drawn before any poll: the scan has not reported, so it is still "scanning".
        let out = screen(80, 8, &view, &config()).join("\n");
        assert!(out.contains("scanning"), "{out}");
        assert_eq!(view.measure(), Measure::OnDisk);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
