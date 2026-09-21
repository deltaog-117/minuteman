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

//! Draws the preview column for a file that is not an image or a folder: wrapped text, a hex dump
//! of a binary file, or the listing of an archive, each scrolled to where `text_preview::Scroll`
//! says and with a scrollbar over the frame's right edge when it is longer than the pane.
//!
//! Everything that decides what a row says is a plain function over the data (`hex_row`,
//! `archive_row`, `scrollbar_position`) so it can be tested without a terminal; `render` only
//! wires them to ratatui. Only the rows on screen are ever formatted: a 64 KiB hex dump is 4,096
//! rows and an archive can list 5,000 entries, and neither should be rebuilt in full each frame.

use preview::Loaded;
use preview::archive::{Hidden, Listing};
use preview::hex::{self, Head, OFFSET_CELLS};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use theming::Config;

use crate::glyphs;
use crate::hud::{self, format_size};
use crate::style;
use crate::text_preview::{PreviewStatus, Scroll, TextPreview};

/// Rows in a hex view of `head` at `per_row` bytes a row: the dump, plus one for the note saying
/// the file continues when it was cut.
pub fn hex_total(head: &Head, per_row: usize) -> usize {
    hex::row_count(head.bytes.len(), per_row) + usize::from(head.is_cut())
}

/// Row `row` of the hex view: a dump row, or the note past the last one.
pub fn hex_row(head: &Head, row: usize, per_row: usize) -> String {
    if row < hex::row_count(head.bytes.len(), per_row) {
        hex::format_row(&head.bytes, row, per_row)
    } else {
        format!(
            "first {} of {} shown",
            format_size(head.bytes.len() as u64),
            format_size(head.total)
        )
    }
}

/// One row of an archive listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveRow {
    /// What the archive is and how much it holds.
    Summary(String),
    Entry {
        /// Right-aligned size, blank for a folder.
        size: String,
        name: String,
        is_dir: bool,
    },
    /// What was left out.
    Note(String),
}

/// Rows in the listing: the summary, every entry, and a note when some were left out.
pub fn archive_total(listing: &Listing) -> usize {
    1 + listing.entries.len() + usize::from(listing.hidden != Hidden::Nothing)
}

/// Row `row` of the listing. `divider` separates the summary's parts (the glyph set's own, so the
/// ASCII set stays ASCII).
pub fn archive_row(listing: &Listing, row: usize, divider: &str) -> ArchiveRow {
    let entries = listing.entries.len();
    if row == 0 {
        let count = match listing.hidden {
            Hidden::Nothing => format!("{entries} entries"),
            Hidden::Count(more) => format!("{} entries", entries + more),
            Hidden::Unknown => format!("{entries}+ entries"),
        };
        let more = if listing.hidden == Hidden::Nothing {
            ""
        } else {
            "+"
        };
        return ArchiveRow::Summary(format!(
            "{} {divider} {count} {divider} {}{more} unpacked",
            listing.kind.label(),
            format_size(listing.unpacked_bytes()),
        ));
    }
    match listing.entries.get(row - 1) {
        Some(entry) => ArchiveRow::Entry {
            size: if entry.is_dir {
                String::new()
            } else {
                format_size(entry.size)
            },
            name: if entry.is_dir && !entry.name.ends_with('/') {
                format!("{}/", entry.name)
            } else {
                entry.name.clone()
            },
            is_dir: entry.is_dir,
        },
        None => ArchiveRow::Note(match listing.hidden {
            Hidden::Count(more) => format!("and {more} more entries"),
            _ => "more entries not shown".to_string(),
        }),
    }
}

/// Where the scrollbar's thumb sits for a pane scrolled to `offset`. The bar is built for a list
/// whose selection reaches `total - 1`, but a scrolled view stops at `total - viewport`, so the
/// offset is stretched to span the same range: the thumb is at the top for offset 0 and at the
/// bottom exactly when the last row is on screen.
pub fn scrollbar_position(offset: usize, total: usize, viewport: usize) -> usize {
    let furthest = total.saturating_sub(viewport);
    if furthest == 0 {
        return 0;
    }
    (offset.min(furthest) * (total - 1) / furthest).min(total - 1)
}

pub fn render(frame: &mut Frame<'_>, area: Rect, preview: &mut TextPreview, config: &Config) {
    let block = style::themed_block(config, "preview", false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match preview.status() {
        PreviewStatus::Loading => frame.render_widget(Paragraph::new("loading preview…"), inner),
        PreviewStatus::Failed => frame.render_widget(Paragraph::new("preview failed"), inner),
        PreviewStatus::Empty => {}
        PreviewStatus::Ready => {
            let (content, rows, scroll) = preview.parts();
            match content {
                Some(Loaded::Text(text)) => {
                    draw_text(frame, area, inner, text, rows, scroll, config)
                }
                Some(Loaded::Bytes(head)) => draw_hex(frame, area, inner, head, scroll, config),
                Some(Loaded::Archive(listing)) => {
                    draw_archive(frame, area, inner, listing, scroll, config);
                }
                _ => {}
            }
        }
    }
}

fn draw_text(
    frame: &mut Frame<'_>,
    area: Rect,
    inner: Rect,
    text: &str,
    rows: &mut crate::text_preview::RowCache,
    scroll: &mut Scroll,
    config: &Config,
) {
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let paragraph = Paragraph::new(text)
        .style(Style::default().fg(style::color(&config.theme.file_fg)))
        .wrap(Wrap { trim: false });
    let total = rows.get(inner.width, || paragraph.line_count(inner.width));
    let offset = scroll.fit(total, inner.height as usize);
    // `Paragraph::scroll` takes a `u16`; a text of over 65,535 wrapped rows is scrolled as far as
    // that reaches.
    let top = u16::try_from(offset).unwrap_or(u16::MAX);
    frame.render_widget(paragraph.scroll((top, 0)), inner);
    draw_scrollbar(frame, area, total, offset, config);
}

fn draw_hex(
    frame: &mut Frame<'_>,
    area: Rect,
    inner: Rect,
    head: &Head,
    scroll: &mut Scroll,
    config: &Config,
) {
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let theme = &config.theme;
    let (text, dim) = (
        Style::default().fg(style::color(&theme.file_fg)),
        Style::default().fg(style::color(&theme.status_fg)),
    );
    let per_row = hex::bytes_per_row(inner.width as usize);
    let total = hex_total(head, per_row);
    let offset = scroll.fit(total, inner.height as usize);
    let lines: Vec<Line<'static>> = (offset..total.min(offset + inner.height as usize))
        .map(|row| {
            let dump = hex_row(head, row, per_row);
            if row < hex::row_count(head.bytes.len(), per_row) {
                // The offset column is dimmed so the bytes stand out; it is all ASCII.
                let (offset_text, rest) = dump.split_at(OFFSET_CELLS.min(dump.len()));
                Line::from(vec![
                    Span::styled(offset_text.to_string(), dim),
                    Span::styled(rest.to_string(), text),
                ])
            } else {
                Line::from(Span::styled(dump, dim))
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
    draw_scrollbar(frame, area, total, offset, config);
}

fn draw_archive(
    frame: &mut Frame<'_>,
    area: Rect,
    inner: Rect,
    listing: &Listing,
    scroll: &mut Scroll,
    config: &Config,
) {
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let theme = &config.theme;
    let file = Style::default().fg(style::color(&theme.file_fg));
    let dir = Style::default().fg(style::color(&theme.dir_fg));
    let dim = Style::default().fg(style::color(&theme.status_fg));
    let divider = glyphs::of(config).divider;
    let total = archive_total(listing);
    let offset = scroll.fit(total, inner.height as usize);
    let lines: Vec<Line<'static>> = (offset..total.min(offset + inner.height as usize))
        .map(|row| match archive_row(listing, row, divider) {
            ArchiveRow::Summary(text) | ArchiveRow::Note(text) => {
                Line::from(Span::styled(text, dim))
            }
            ArchiveRow::Entry { size, name, is_dir } => Line::from(vec![
                Span::styled(format!("{size:>6}  "), dim),
                Span::styled(name, if is_dir { dir } else { file }),
            ]),
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
    draw_scrollbar(frame, area, total, offset, config);
}

fn draw_scrollbar(frame: &mut Frame<'_>, area: Rect, total: usize, offset: usize, config: &Config) {
    let viewport = area.height.saturating_sub(2) as usize;
    hud::render_scrollbar(
        frame,
        area,
        total,
        scrollbar_position(offset, total, viewport),
        config,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use preview::archive::{Entry, Kind};
    use proptest::prelude::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Runtime::new().unwrap()
    }

    fn screen(width: u16, height: u16, preview: &mut TextPreview) -> Vec<String> {
        let config = Config::from_sources(None, None);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), preview, &config))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        buffer
            .content()
            .chunks(width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect()
    }

    fn head(len: usize, total: u64) -> Head {
        Head {
            bytes: (0..len).map(|i| (i % 251) as u8).collect(),
            total,
        }
    }

    fn listing(count: usize, hidden: Hidden) -> Listing {
        Listing {
            kind: Kind::Zip,
            entries: (0..count)
                .map(|i| Entry {
                    name: if i % 5 == 0 {
                        format!("dir{i}/")
                    } else {
                        format!("file{i:03}.txt")
                    },
                    size: 1000 * i as u64,
                    is_dir: i % 5 == 0,
                })
                .collect(),
            hidden,
        }
    }

    #[test]
    fn a_hex_row_is_a_dump_row_and_the_row_after_the_last_is_the_cut_note() {
        let cut = head(40, 5000);
        assert_eq!(hex_total(&cut, 16), 3 + 1);
        assert!(hex_row(&cut, 0, 16).starts_with("00000000  00 01 02"));
        assert!(
            hex_row(&cut, 3, 16).starts_with("first 40B of "),
            "{}",
            hex_row(&cut, 3, 16)
        );
        let whole = head(40, 40);
        assert_eq!(
            hex_total(&whole, 16),
            3,
            "no note when the whole file is shown"
        );
    }

    #[test]
    fn an_archive_listing_is_a_summary_the_entries_and_a_note_when_cut() {
        let all = listing(3, Hidden::Nothing);
        assert_eq!(archive_total(&all), 4);
        assert_eq!(
            archive_row(&all, 0, "|"),
            ArchiveRow::Summary(format!("zip | 3 entries | {} unpacked", format_size(3000)))
        );
        assert_eq!(
            archive_row(&all, 1, "|"),
            ArchiveRow::Entry {
                size: String::new(),
                name: "dir0/".into(),
                is_dir: true
            }
        );
        assert_eq!(
            archive_row(&all, 2, "|"),
            ArchiveRow::Entry {
                size: format_size(1000),
                name: "file001.txt".into(),
                is_dir: false
            }
        );

        let cut = listing(3, Hidden::Count(7));
        assert_eq!(archive_total(&cut), 5);
        assert!(
            matches!(archive_row(&cut, 0, "|"), ArchiveRow::Summary(s) if s.contains("10 entries"))
        );
        assert_eq!(
            archive_row(&cut, 4, "|"),
            ArchiveRow::Note("and 7 more entries".into())
        );
        let unknown = listing(3, Hidden::Unknown);
        assert!(
            matches!(archive_row(&unknown, 0, "|"), ArchiveRow::Summary(s) if s.contains("3+ entries"))
        );
        assert_eq!(
            archive_row(&unknown, 4, "|"),
            ArchiveRow::Note("more entries not shown".into())
        );
    }

    #[test]
    fn a_folder_entry_always_ends_in_a_slash() {
        let mut one = listing(0, Hidden::Nothing);
        one.entries.push(Entry {
            name: "src".into(),
            size: 0,
            is_dir: true,
        });
        assert_eq!(
            archive_row(&one, 1, "|"),
            ArchiveRow::Entry {
                size: String::new(),
                name: "src/".into(),
                is_dir: true
            }
        );
    }

    proptest! {
        /// The thumb is at the top for offset 0, at the bottom exactly when the last row is on
        /// screen, never beyond the bar, and never moves up as the offset grows.
        #[test]
        fn the_scrollbar_thumb_spans_the_bar_and_only_moves_down(
            total in 1usize..500,
            viewport in 1usize..80,
            a in 0usize..600,
            b in 0usize..600,
        ) {
            let position = |offset| scrollbar_position(offset, total, viewport);
            prop_assert_eq!(position(0), 0);
            prop_assert!(position(a) < total);
            let (lo, hi) = (a.min(b), a.max(b));
            prop_assert!(position(lo) <= position(hi));
            if total > viewport {
                prop_assert_eq!(position(total - viewport), total - 1);
            }
        }
    }

    #[test]
    fn text_is_wrapped_and_scrolls_a_row_at_a_time() {
        let rt = runtime();
        let text: String = (0..30).map(|i| format!("line {i}\n")).collect();
        let mut preview = TextPreview::showing(rt.handle().clone(), Loaded::Text(text));

        let top = screen(20, 8, &mut preview);
        assert!(top[1].starts_with("│line 0"), "{top:?}");
        preview.scroll_rows(5);
        let scrolled = screen(20, 8, &mut preview);
        assert!(scrolled[1].starts_with("│line 5"), "{scrolled:?}");
        preview.scroll_rows(-100);
        assert!(screen(20, 8, &mut preview)[1].starts_with("│line 0"));
    }

    #[test]
    fn scrolling_past_the_end_stops_with_the_last_line_at_the_bottom() {
        let rt = runtime();
        let text: String = (0..30).map(|i| format!("line {i}\n")).collect();
        let mut preview = TextPreview::showing(rt.handle().clone(), Loaded::Text(text));
        preview.scroll_rows(10_000);
        let end = screen(20, 8, &mut preview);
        assert!(
            end[6].starts_with("│line 29"),
            "the last line is the last row: {end:?}"
        );
        // And another notch changes nothing.
        preview.scroll_rows(3);
        assert_eq!(screen(20, 8, &mut preview), end);
    }

    #[test]
    fn a_long_line_wraps_rather_than_running_off_the_pane() {
        let rt = runtime();
        let mut preview = TextPreview::showing(
            rt.handle().clone(),
            Loaded::Text("word ".repeat(20).trim_end().to_string()),
        );
        let out = screen(22, 8, &mut preview);
        assert!(
            out[1].contains("word") && out[2].contains("word"),
            "{out:?}"
        );
    }

    #[test]
    fn a_hex_dump_shows_offsets_bytes_and_ascii_and_scrolls() {
        let rt = runtime();
        let mut preview = TextPreview::showing(rt.handle().clone(), Loaded::Bytes(head(400, 400)));
        // 60 columns: 58 inside the frame, so 8 bytes to a row (16 needs 74).
        let top = screen(60, 8, &mut preview);
        assert!(
            top[1].contains("00000000  00 01 02 03 04 05 06 07"),
            "{top:?}"
        );
        assert!(top[2].contains("00000008  08 09"), "{top:?}");
        preview.scroll_rows(3);
        let moved = screen(60, 8, &mut preview);
        assert!(moved[1].contains("00000018"), "{moved:?}");
    }

    #[test]
    fn a_wide_pane_gets_sixteen_bytes_a_row_and_a_cut_file_says_so_at_the_end() {
        let rt = runtime();
        let mut preview =
            TextPreview::showing(rt.handle().clone(), Loaded::Bytes(head(64, 9_999_999)));
        let wide = screen(90, 8, &mut preview);
        assert!(wide[2].contains("00000010"), "{wide:?}");
        preview.scroll_rows(1000);
        let end = screen(90, 8, &mut preview);
        assert!(
            end.iter().any(|row| row.contains("first 64B of ")),
            "{end:?}"
        );
    }

    #[test]
    fn an_archive_shows_a_summary_then_sizes_and_names() {
        let rt = runtime();
        let mut preview = TextPreview::showing(
            rt.handle().clone(),
            Loaded::Archive(listing(12, Hidden::Count(3))),
        );
        let out = screen(50, 8, &mut preview);
        assert!(
            out[1].contains("zip") && out[1].contains("15 entries"),
            "{out:?}"
        );
        assert!(out[2].contains("dir0/"), "{out:?}");
        assert!(
            out[3].contains("1000") || out[3].contains("1.0K") || out[3].contains("1K"),
            "{out:?}"
        );
        assert!(out[3].contains("file001.txt"), "{out:?}");
        preview.scroll_rows(1000);
        let end = screen(50, 8, &mut preview);
        assert!(
            end.iter().any(|row| row.contains("and 3 more entries")),
            "{end:?}"
        );
    }

    #[test]
    fn loading_failed_and_empty_previews_say_so_or_stay_blank() {
        let rt = runtime();
        let mut loading = TextPreview::in_status(rt.handle().clone(), PreviewStatus::Loading);
        assert!(screen(30, 5, &mut loading)[1].contains("loading preview"));
        let mut failed = TextPreview::in_status(rt.handle().clone(), PreviewStatus::Failed);
        assert!(screen(30, 5, &mut failed)[1].contains("preview failed"));
        let mut empty = TextPreview::in_status(rt.handle().clone(), PreviewStatus::Empty);
        assert!(
            screen(30, 5, &mut empty)[1]
                .trim_matches('│')
                .trim()
                .is_empty()
        );
    }

    #[test]
    fn no_size_of_pane_panics_for_any_kind_of_content() {
        let rt = runtime();
        let contents = [
            Loaded::Text("some text\nmore text that is long enough to wrap around\n".into()),
            Loaded::Text(String::new()),
            Loaded::Bytes(head(0, 0)),
            Loaded::Bytes(head(300, 100_000)),
            Loaded::Archive(listing(0, Hidden::Nothing)),
            Loaded::Archive(listing(40, Hidden::Unknown)),
        ];
        for content in contents {
            let mut preview = TextPreview::showing(rt.handle().clone(), content);
            for (width, height) in [(1, 1), (2, 2), (3, 3), (5, 4), (10, 3), (30, 2), (80, 24)] {
                for scroll in [0, 1, 7, 10_000] {
                    preview.scroll_rows(scroll);
                    screen(width, height, &mut preview);
                }
            }
        }
    }

    /// Benchmark stub for the one costly path: a frame of a 1 MiB text scrolled to the very end,
    /// where ratatui has to wrap everything above the visible rows on every draw. The budget is
    /// the 100 ms between ticks; run it with
    /// `cargo test -p tui --release -- --ignored frame_time` (it is ignored because wall-clock
    /// assertions are flaky in an ordinary test run, and meaningless unoptimised).
    #[test]
    #[ignore = "a timing check; run in release mode on purpose"]
    fn frame_time_of_a_deeply_scrolled_megabyte_of_text_is_within_the_tick() {
        let rt = runtime();
        let line = "the quick brown fox jumps over the lazy dog, again and again\n";
        let text = line.repeat((1 << 20) / line.len());
        let mut preview = TextPreview::showing(rt.handle().clone(), Loaded::Text(text));
        preview.scroll_rows(isize::MAX / 2);
        // The first frame also measures the wrapped height; later ones reuse it.
        screen(56, 30, &mut preview);
        let started = std::time::Instant::now();
        let frames = 20;
        for _ in 0..frames {
            screen(56, 30, &mut preview);
        }
        let per_frame = started.elapsed() / frames;
        eprintln!("deep-scroll frame: {per_frame:?}");
        assert!(
            per_frame < std::time::Duration::from_millis(100),
            "{per_frame:?}"
        );
    }
}
