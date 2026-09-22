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

//! Draws the two floating panels — the right-click menu and the Inspect panel — over the
//! browser. All geometry comes from `context_menu` (the menu) or is derived here from the frame
//! alone (the panel), so this file only paints: it holds no state and decides nothing.

use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use theming::Config;

use crate::appearance_popup::{AppearancePopup, AppearanceView, Row as AppearanceRow};
use crate::context_menu::{ContextMenu, Entry, Item, MenuCommand, Slot};
use crate::glyphs;
use crate::hud::{fit_width, pad_to, text_width};
use crate::inspect::InspectView;
use crate::settings_popup::{SettingsPopup, SettingsView};
use crate::style;

/// The panel's widest and narrowest width in cells; the value column takes what the labels leave.
const PANEL_MAX_WIDTH: u16 = 76;
/// Width of the label column ("Permissions" is the longest label) plus a gap.
const LABEL_COLUMN: usize = 13;

/// A bordered frame with no title, in the same border style as the panes.
fn frame_block(config: &Config) -> Block<'static> {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(style::color(&config.theme.border_focused_fg)));
    if glyphs::of(config).ascii_borders {
        block.border_set(glyphs::ASCII_BORDER)
    } else {
        block.border_type(style::border_type(&config.theme.border_type))
    }
}

/// Draws `menu` and its open submenu, if any, over whatever is underneath.
pub fn render_menu(frame: &mut Frame<'_>, menu: &ContextMenu, config: &Config) {
    let layout = menu.layout(frame.area());
    let marker = if glyphs::of(config).ascii_borders {
        ">"
    } else {
        "▸"
    };
    let main_rows: Vec<Row<'_>> = menu
        .entries()
        .iter()
        .map(|entry| match entry {
            Entry::Separator => Row::Separator,
            Entry::Item(item) => Row::Text {
                label: &item.label,
                submenu: false,
                enabled: item.enabled,
                danger: item.command == MenuCommand::Delete,
            },
            Entry::Submenu { label, items } => Row::Text {
                label,
                submenu: true,
                enabled: !items.is_empty(),
                danger: false,
            },
        })
        .collect();
    let lit = match menu.hover() {
        Some(Slot::Main(i)) => Some(i),
        _ => None,
    };
    // A submenu row stays lit while its submenu is open, so it reads as the parent.
    let lit = lit.or(menu.open_submenu());
    render_rows(frame, layout.main, &main_rows, lit, marker, config);

    if let (Some(rect), Some(items)) = (layout.sub, menu.sub_items()) {
        let rows: Vec<Row<'_>> = items.iter().map(item_row).collect();
        let lit = match menu.hover() {
            Some(Slot::Sub(j)) => Some(j),
            _ => None,
        };
        render_rows(frame, rect, &rows, lit, marker, config);
    }
}

fn item_row(item: &Item) -> Row<'_> {
    Row::Text {
        label: &item.label,
        submenu: false,
        enabled: item.enabled,
        danger: false,
    }
}

enum Row<'a> {
    Separator,
    Text {
        label: &'a str,
        submenu: bool,
        enabled: bool,
        danger: bool,
    },
}

fn render_rows(
    frame: &mut Frame<'_>,
    rect: Rect,
    rows: &[Row<'_>],
    lit: Option<usize>,
    marker: &str,
    config: &Config,
) {
    let theme = &config.theme;
    frame.render_widget(Clear, rect);
    let block = frame_block(config);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let width = usize::from(inner.width);
    let lines: Vec<Line<'_>> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| match row {
            Row::Separator => Line::styled(
                "─".repeat(width),
                Style::default().fg(style::color(&theme.border_fg)),
            ),
            Row::Text {
                label,
                submenu,
                enabled,
                danger,
            } => {
                let tail = if *submenu { marker } else { "" };
                // One space of padding each side; the submenu marker sits in the last cells.
                let body = pad_to(
                    &format!(" {label}"),
                    width.saturating_sub(text_width(tail) + 1),
                );
                let text = format!("{body}{tail} ");
                let mut style = Style::default().fg(if !enabled {
                    style::color(&theme.border_fg)
                } else if *danger {
                    style::color(&theme.danger_fg)
                } else {
                    style::color(&theme.file_fg)
                });
                if !enabled {
                    style = style.add_modifier(Modifier::DIM);
                }
                if lit == Some(i) {
                    style = style.bg(style::color(&theme.selection_bg));
                    if let Some(fg) = style::selection_fg(theme) {
                        style = style.fg(fg);
                    }
                }
                Line::styled(text, style)
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The Inspect panel's rectangle: centred, as wide as its values need up to a limit, and never
/// taller than the screen.
pub fn panel_area(frame: Rect, rows: usize) -> Rect {
    let width = PANEL_MAX_WIDTH.min(frame.width.saturating_sub(2));
    // Border, a blank line above the rows, a blank line and the hint below.
    let height = (rows as u16 + 5).min(frame.height);
    Rect::new(
        frame.x + frame.width.saturating_sub(width) / 2,
        frame.y + frame.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

/// Draws the Inspect panel for `view` in the middle of the screen.
pub fn render_inspect(frame: &mut Frame<'_>, view: &InspectView, config: &Config) {
    let theme = &config.theme;
    let rows = view.rows();
    let area = panel_area(frame.area(), rows.len());
    frame.render_widget(Clear, area);
    let block = style::themed_block(config, &format!("inspect: {}", view.title()), true);
    let inner = block.inner(area).inner(Margin::new(1, 0));
    frame.render_widget(block, area);

    let value_width = usize::from(inner.width).saturating_sub(LABEL_COLUMN);
    let label_style = style::styled(
        Style::default().fg(style::color(&theme.accent_fg)),
        config.styles.title,
    );
    let value_style = Style::default().fg(style::color(&theme.file_fg));
    let mut lines = vec![Line::raw("")];
    lines.extend(rows.iter().map(|(label, value)| {
        Line::from(vec![
            Span::styled(pad_to(label, LABEL_COLUMN), label_style),
            Span::styled(fit_width(value, value_width), value_style),
        ])
    }));
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Esc, Enter or a click closes",
        Style::default().fg(style::color(&theme.border_fg)),
    ));
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Draws the settings popup: one row per setting, the cursor's row highlighted the same way a
/// selected file is. Session-only — nothing here is written back to a config file.
pub fn render_settings(
    frame: &mut Frame<'_>,
    popup: &SettingsPopup,
    view: &SettingsView,
    config: &Config,
) {
    let theme = &config.theme;
    let rows = popup.rows();
    let area = panel_area(frame.area(), rows.len());
    frame.render_widget(Clear, area);
    let block = style::themed_block(config, "settings", true);
    let inner = block.inner(area).inner(Margin::new(1, 0));
    frame.render_widget(block, area);

    let value_width = usize::from(inner.width).saturating_sub(LABEL_COLUMN);
    let label_style = style::styled(
        Style::default().fg(style::color(&theme.accent_fg)),
        config.styles.title,
    );
    let value_style = Style::default().fg(style::color(&theme.file_fg));
    let selected_style = Style::default()
        .bg(style::color(&theme.selection_bg))
        .fg(style::color(&theme.file_fg));

    let mut lines = vec![Line::raw("")];
    lines.extend(rows.iter().enumerate().map(|(i, row)| {
        let base = if i == popup.cursor() {
            selected_style
        } else {
            Style::default()
        };
        Line::from(vec![
            Span::styled(pad_to(row.label(), LABEL_COLUMN), label_style.patch(base)),
            Span::styled(
                fit_width(&view.value(*row), value_width),
                value_style.patch(base),
            ),
        ])
    }));
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "j/k moves, h/l/enter changes, Esc closes — session only, not saved",
        Style::default().fg(style::color(&theme.border_fg)),
    ));
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Draws the appearance popup: one row per setting, the cursor's row highlighted the same way a
/// selected file is. The row being edited shows its in-progress buffer with a caret instead of
/// its committed value; `Reset to defaults` is styled like a destructive action, the same as
/// `Delete` in the right-click menu. Session-only — nothing here is written back to a config file.
pub fn render_appearance(
    frame: &mut Frame<'_>,
    popup: &AppearancePopup,
    view: &AppearanceView<'_>,
    config: &Config,
) {
    let theme = &config.theme;
    let rows = popup.rows();
    let area = panel_area(frame.area(), rows.len());
    frame.render_widget(Clear, area);
    let block = style::themed_block(config, "appearance", true);
    let inner = block.inner(area).inner(Margin::new(1, 0));
    frame.render_widget(block, area);

    let value_width = usize::from(inner.width).saturating_sub(LABEL_COLUMN);
    let label_style = style::styled(
        Style::default().fg(style::color(&theme.accent_fg)),
        config.styles.title,
    );
    let danger_label_style = style::styled(
        Style::default().fg(style::color(&theme.danger_fg)),
        config.styles.title,
    );
    let value_style = Style::default().fg(style::color(&theme.file_fg));
    let selected_style = Style::default()
        .bg(style::color(&theme.selection_bg))
        .fg(style::color(&theme.file_fg));

    let editing_row = popup.editing_row();
    let caret = if glyphs::of(config).ascii_borders {
        "_"
    } else {
        "▏"
    };

    let mut lines = vec![Line::raw("")];
    lines.extend(rows.iter().enumerate().map(|(i, row)| {
        let base = if i == popup.cursor() {
            selected_style
        } else {
            Style::default()
        };
        let label = if *row == AppearanceRow::Reset {
            danger_label_style
        } else {
            label_style
        };
        let value = if editing_row == Some(*row) {
            format!("{}{caret}", popup.editing_buffer().unwrap_or_default())
        } else {
            view.value(*row)
        };
        Line::from(vec![
            Span::styled(pad_to(row.label(), LABEL_COLUMN), label.patch(base)),
            Span::styled(fit_width(&value, value_width), value_style.patch(base)),
        ])
    }));
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        if editing_row.is_some() {
            "type to edit, enter confirms, Esc cancels"
        } else {
            "j/k moves, enter edits/cycles, a click acts, Esc closes — session only, not saved"
        },
        Style::default().fg(style::color(&theme.border_fg)),
    ));
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Position;

    use super::*;
    use crate::context_menu::{Context, Target, entries};

    fn config() -> Config {
        Config::from_sources(None, None)
    }

    fn screen_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn file_menu(at: (u16, u16)) -> ContextMenu {
        let context = Context {
            target: Target::Entry { is_dir: false },
            marked: false,
            mark_count: 0,
            clipboard: false,
            hidden_shown: false,
        };
        ContextMenu::new(
            Position::new(at.0, at.1),
            context.target,
            entries(&context, &["Neovim".into()]),
        )
    }

    #[test]
    fn the_menu_draws_every_label_and_its_submenu_marker() {
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        let menu = file_menu((5, 2));
        terminal
            .draw(|frame| render_menu(frame, &menu, &config()))
            .unwrap();
        let text = screen_text(&terminal);
        for label in [
            "Open",
            "Open with",
            "Cut",
            "Copy",
            "Rename",
            "Delete",
            "Inspect",
        ] {
            assert!(text.contains(label), "missing {label}:\n{text}");
        }
        assert!(text.contains('▸'), "the submenu row carries its marker");
    }

    #[test]
    fn an_open_submenu_is_drawn_beside_its_parent() {
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        let mut menu = file_menu((5, 2));
        menu.hover_at(Rect::new(0, 0, 60, 20), Position::new(7, 4));
        terminal
            .draw(|frame| render_menu(frame, &menu, &config()))
            .unwrap();
        assert!(screen_text(&terminal).contains("Neovim"));
    }

    #[test]
    fn the_hovered_row_gets_the_selection_background() {
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        let mut menu = file_menu((5, 2));
        let frame_area = Rect::new(0, 0, 60, 20);
        menu.hover_at(frame_area, Position::new(7, 3));
        let config = config();
        terminal
            .draw(|frame| render_menu(frame, &menu, &config))
            .unwrap();
        let lit = style::color(&config.theme.selection_bg);
        // Row 0 ("Open") is one cell below the top border, one in from the left border.
        assert_eq!(terminal.backend().buffer()[(7, 3)].bg, lit);
        assert_ne!(terminal.backend().buffer()[(7, 4)].bg, lit);
    }

    #[test]
    fn a_menu_drawn_at_the_screen_corner_still_fits() {
        let mut terminal = Terminal::new(TestBackend::new(40, 14)).unwrap();
        let menu = file_menu((39, 13));
        terminal
            .draw(|frame| render_menu(frame, &menu, &config()))
            .unwrap();
        assert!(screen_text(&terminal).contains("Inspect"));
    }

    #[test]
    fn the_panel_is_centred_and_never_larger_than_the_screen() {
        let area = panel_area(Rect::new(0, 0, 100, 30), 10);
        assert_eq!((area.width, area.height), (PANEL_MAX_WIDTH, 15));
        assert_eq!(area.x, (100 - PANEL_MAX_WIDTH) / 2);
        let tiny = panel_area(Rect::new(0, 0, 20, 6), 10);
        assert!(tiny.right() <= 20 && tiny.bottom() <= 6);
    }

    #[test]
    fn the_inspect_panel_draws_the_rows_for_a_real_file() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = std::env::temp_dir().join(format!("minuteman-overlay-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.txt"), b"hello").unwrap();
        let view = InspectView::open(runtime.handle(), &dir.join("note.txt")).unwrap();

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| render_inspect(frame, &view, &config()))
            .unwrap();
        let text = screen_text(&terminal);
        for expected in [
            "inspect: note.txt",
            "Permissions",
            "Modified",
            "5B (5 bytes)",
        ] {
            assert!(text.contains(expected), "missing {expected}:\n{text}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
