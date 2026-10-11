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

//! Draws a plugin's picker (the fuzzy finder's): a panel floating over the browser with the query on top, the ranked
//! results below it, and, on a screen wide enough, the selected result's preview beside them. The
//! preview itself is drawn by `main` (it owns the preview pipelines); this module only says where
//! it goes. The panel is laid out by `trash_view::layout`, which already splits a rect into a
//! header, rows, a footer and a preview, so the two read the same.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use theming::Config;

use plugin_host::protocol::PickerIndexing;

use crate::glyphs;
use crate::hud::{self, fit_width, pad_to, text_width};
use crate::picker_popup::PickerPopup;
use crate::style;
use crate::trash_view::{self, Layout, elide_left, scroll_top};

/// The panel's rect: most of the screen, centered, so what is behind it still shows at the edges.
pub fn area(screen: Rect) -> Rect {
    let width = (screen.width * 9 / 10).max(screen.width.min(40));
    let height = (screen.height * 8 / 10).max(screen.height.min(10));
    Rect {
        x: screen.x + (screen.width - width) / 2,
        y: screen.y + (screen.height - height) / 2,
        width,
        height,
    }
}

/// Where the parts of the panel go; with `preview` false the whole panel is the list, however
/// wide the screen.
pub fn layout(screen: Rect, preview: bool) -> Layout {
    trash_view::layout_with(area(screen), preview)
}

/// The header's right-hand note: how many entries match out of how many are indexed, and whether
/// the walk is still going or was cut short by a limit.
fn status(popup: &PickerPopup) -> String {
    let counts = format!("{}/{}", popup.matches(), popup.indexed());
    match (popup.indexing(), popup.settled()) {
        (PickerIndexing::Walking, _) => format!("{counts} indexing…"),
        (PickerIndexing::Done | PickerIndexing::Truncated, false) => format!("{counts} ranking…"),
        (PickerIndexing::Truncated, true) => format!("{counts} (limit reached)"),
        (PickerIndexing::Done, true) => counts,
    }
}

pub fn render(frame: &mut Frame<'_>, popup: &PickerPopup, config: &Config) {
    let places = layout(frame.area(), popup.wants_preview());
    let theme = &config.theme;
    let g = glyphs::of(config);
    style::cast_shadow(frame.buffer_mut(), places.frame, config);
    frame.render_widget(Clear, places.frame);
    if let Some(preview) = places.preview {
        frame.render_widget(Clear, preview);
    }
    frame.render_widget(
        style::themed_block(config, popup.title(), true),
        places.frame,
    );

    let dim = Style::default().fg(style::color(&theme.status_fg));
    let accent = Style::default().fg(style::color(&theme.accent_fg));
    let width = usize::from(places.header.width);

    let note = status(popup);
    let note_width = text_width(&note);
    let typed = popup.query().render(g.cursor);
    let prompt = format!("> {typed}");
    let room = width.saturating_sub(note_width + 1);
    let left = pad_to(&fit_width(&prompt, room), room);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(left, accent),
            Span::raw(" "),
            Span::styled(note, dim),
        ])),
        places.header,
    );

    let viewport = usize::from(places.rows.height);
    let len = popup.results().len();
    let top = scroll_top(popup.selected(), viewport, popup.top().get(), len);
    popup.top().set(top);
    popup.viewport().set(viewport);

    if len == 0 {
        let note = match (popup.indexing(), popup.settled()) {
            (PickerIndexing::Walking, _) => "indexing…",
            (PickerIndexing::Done | PickerIndexing::Truncated, false) => "ranking…",
            (PickerIndexing::Done | PickerIndexing::Truncated, true) => "no match",
        };
        frame.render_widget(Paragraph::new(Span::styled(note, dim)), places.rows);
    } else {
        let rows = usize::from(places.rows.width);
        let name_width = (rows * 45 / 100).max(12);
        let lines: Vec<Line<'static>> = popup
            .results()
            .enumerate()
            .skip(top)
            .take(viewport)
            .map(|(index, candidate)| {
                let selected = index == popup.selected();
                let name_color = if candidate.is_dir {
                    &theme.dir_fg
                } else {
                    &theme.file_fg
                };
                let label = if candidate.is_dir {
                    format!("{}/", candidate.name())
                } else {
                    candidate.name().to_string()
                };
                let folder_room = rows.saturating_sub(2 + name_width + 1);
                let mut spans = vec![
                    Span::styled(
                        if selected {
                            format!("{} ", g.stripe)
                        } else {
                            "  ".to_string()
                        },
                        accent,
                    ),
                    Span::styled(
                        pad_to(&fit_width(&label, name_width), name_width),
                        Style::default().fg(style::color(name_color)),
                    ),
                ];
                if folder_room > 0 {
                    spans.push(Span::styled(
                        format!(" {}", elide_left(candidate.parent_rel(), folder_room)),
                        dim,
                    ));
                }
                let line = Line::from(spans);
                if selected {
                    line.style(Style::default().bg(style::color(&theme.selection_bg)))
                } else {
                    line
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), places.rows);
    }

    frame.render_widget(
        Paragraph::new(Span::styled(
            fit_width("↑↓ move  Enter show in browser  Esc close", width),
            dim,
        )),
        places.footer,
    );
    hud::render_scrollbar(frame, places.frame, len, popup.selected(), config);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use plugin_host::protocol::{PickerSpec, PickerUpdate, TreeEntry};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Position;
    use shared::MemVfs;

    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn entry(rel: &str) -> TreeEntry {
        TreeEntry {
            path: PathBuf::from("/root").join(rel),
            rel: rel.to_string(),
            is_dir: false,
            size: None,
            modified: None,
        }
    }

    /// A picker holding what a finder over a small tree would have sent for `typed`, drawn on a
    /// `width` x `height` screen.
    fn drawn(typed: &str, width: u16, height: u16) -> String {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let spec = PickerSpec {
            picker: 1,
            title: "find".to_string(),
            preview: true,
        };
        let mut popup =
            PickerPopup::open(runtime.handle(), Arc::new(MemVfs::new()), "finder", &spec);
        for c in typed.chars() {
            popup.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        let all = [entry("src/main.rs"), entry("docs/guide.md")];
        let items: Vec<TreeEntry> = all
            .iter()
            .filter(|e| e.rel.contains(typed))
            .cloned()
            .collect();
        popup.apply(PickerUpdate {
            picker: 1,
            generation: u64::try_from(typed.chars().count()).unwrap(),
            matches: items.len(),
            indexed: 4,
            items,
            indexing: PickerIndexing::Done,
            settled: true,
        });
        let config = Config::from_sources(None, None, None);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, &popup, &config))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[Position::new(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_panel_shows_the_query_the_counts_and_each_result_with_its_folder() {
        let shown = drawn("main", 120, 40);
        assert!(shown.contains("> main"), "{shown}");
        assert!(shown.contains("1/4"), "one match of four indexed:\n{shown}");
        assert!(shown.contains("main.rs"), "{shown}");
        assert!(
            shown.contains("src"),
            "the folder is shown beside the name:\n{shown}"
        );
        assert!(!shown.contains("guide.md"), "{shown}");
    }

    #[test]
    fn the_panel_is_titled_by_the_plugin() {
        assert!(drawn("m", 120, 40).contains("find"));
    }

    #[test]
    fn a_query_with_no_match_says_so() {
        assert!(drawn("zzzz", 120, 40).contains("no match"));
    }

    #[test]
    fn it_draws_on_any_screen_without_panicking() {
        for (w, h) in [
            (0, 0),
            (1, 1),
            (20, 6),
            (60, 12),
            (99, 30),
            (100, 30),
            (300, 80),
        ] {
            drawn("m", w, h);
        }
    }

    #[test]
    fn the_panel_is_centered_and_inside_the_screen() {
        let screen = Rect::new(0, 0, 120, 40);
        let panel = area(screen);
        assert_eq!(panel.x * 2 + panel.width, screen.width);
        assert_eq!(panel.y * 2 + panel.height, screen.height);
        assert!(screen.contains(ratatui::layout::Position::new(panel.x, panel.y)));
    }

    #[test]
    fn a_tiny_screen_never_makes_a_panel_larger_than_itself() {
        for (w, h) in [(0, 0), (1, 1), (10, 3), (30, 9), (200, 5)] {
            let screen = Rect::new(0, 0, w, h);
            let panel = area(screen);
            assert!(panel.width <= screen.width && panel.height <= screen.height);
            let _ = layout(screen, true);
        }
    }

    #[test]
    fn a_picker_without_a_preview_gives_the_whole_panel_to_the_list() {
        let screen = Rect::new(0, 0, 200, 50);
        assert!(layout(screen, true).preview.is_some());
        let plain = layout(screen, false);
        assert!(plain.preview.is_none());
        assert_eq!(plain.frame, area(screen));
    }
}
