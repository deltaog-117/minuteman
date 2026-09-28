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
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use theming::{Config, Hsv, Theme};

use crate::appearance_popup::{
    self, AppearancePopup, AppearanceView, ManageView, PICKER_EXTRA_LINES, Row as AppearanceRow,
    RowKind as AppearanceRowKind, SV_BOX_HEIGHT, SaveView, View as AppearanceViewLevel,
};
use crate::compress_popup::{CompressPopup, Row as CompressRow};
use crate::context_menu::{ContextMenu, Entry, Item, MenuCommand, Slot};
use crate::extract_popup::{ExtractPopup, Row as ExtractRow};
use crate::glyphs;
use crate::hud::{fit_width, pad_to, text_width};
use crate::inspect::InspectView;
use crate::settings_popup::{SettingsPopup, SettingsView};
use crate::style;

/// The panel's widest and narrowest width in cells; the value column takes what the labels leave.
const PANEL_MAX_WIDTH: u16 = 76;
/// Width of the label column ("Permissions" is the longest label) plus a gap.
const LABEL_COLUMN: usize = 13;
/// The appearance popup's own, wider label column: its category labels (`Border & separator ›`),
/// long element names (`Syntax: function`) and saved themes' own names would all be cut off at
/// `LABEL_COLUMN`.
const APPEARANCE_LABEL_COLUMN: usize = 22;

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

/// Label column of the compress form, wide enough for "Delete originals".
const COMPRESS_LABEL_COLUMN: usize = 18;

/// The compress form's panel.
pub fn compress_area(frame: Rect, popup: &CompressPopup) -> Rect {
    panel_area(frame, popup.rows().len())
}

/// Where the form's rows are drawn: inside the border and one cell of margin, below a blank line.
fn compress_inner(area: Rect) -> Rect {
    area.inner(Margin::new(2, 1))
}

/// What a click landed on in the compress form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressHit {
    Row(usize),
    /// Inside the panel but not on a row.
    Panel,
    Outside,
}

pub fn compress_hit(
    frame: Rect,
    popup: &CompressPopup,
    at: ratatui::layout::Position,
) -> CompressHit {
    let area = compress_area(frame, popup);
    if !area.contains(at) {
        return CompressHit::Outside;
    }
    let inner = compress_inner(area);
    let first = inner.y.saturating_add(1);
    let row = usize::from(at.y.saturating_sub(first));
    if at.y >= first && row < popup.rows().len() && at.x >= inner.x && at.x < inner.right() {
        CompressHit::Row(row)
    } else {
        CompressHit::Panel
    }
}

/// Draws the compress form: one row per choice with the focused row highlighted, the name with
/// its fixed extension and a cursor, and any error where the blank line under the rows is.
pub fn render_compress(frame: &mut Frame<'_>, popup: &CompressPopup, config: &Config) {
    let theme = &config.theme;
    let area = compress_area(frame.area(), popup);
    frame.render_widget(Clear, area);
    let block = style::themed_block(config, &popup.title(), true);
    frame.render_widget(block, area);
    let inner = compress_inner(area);

    let label_style = style::styled(
        Style::default().fg(style::color(&theme.accent_fg)),
        config.styles.title,
    );
    let value_style = Style::default().fg(style::color(&theme.file_fg));
    let dim_style = Style::default().fg(style::color(&theme.border_fg));
    let selected_style = Style::default()
        .bg(style::color(&theme.selection_bg))
        .fg(style::color(&theme.file_fg));
    let value_width = usize::from(inner.width).saturating_sub(COMPRESS_LABEL_COLUMN);

    let mut lines = vec![Line::raw("")];
    lines.extend(popup.rows().iter().enumerate().map(|(i, &row)| {
        let base = if i == popup.cursor() {
            selected_style
        } else {
            Style::default()
        };
        let inert = match row {
            CompressRow::Name => !popup.name_applies(),
            CompressRow::Level => !popup.level_applies(),
            _ => false,
        };
        let mut spans = vec![Span::styled(
            pad_to(row.label(), COMPRESS_LABEL_COLUMN),
            label_style.patch(base),
        )];
        if row == CompressRow::Name && !inert {
            // The extension is fixed by the Format row, so it is drawn after what is typed and
            // never edited; a bar after the text marks where typing goes.
            let extension = format!(".{}", popup.format().extension());
            let room = value_width.saturating_sub(text_width(&extension) + 1);
            spans.push(Span::styled(
                fit_tail(popup.name(), room),
                value_style.patch(base),
            ));
            if i == popup.cursor() {
                spans.push(Span::styled("▏", value_style.patch(base)));
            }
            spans.push(Span::styled(extension, dim_style.patch(base)));
        } else {
            let style = if inert { dim_style } else { value_style };
            spans.push(Span::styled(
                fit_width(&popup.value(row), value_width),
                style.patch(base),
            ));
        }
        Line::from(spans)
    }));
    lines.push(match popup.error() {
        Some(message) => Line::styled(
            fit_width(message, usize::from(inner.width)),
            Style::default().fg(style::color(&theme.danger_fg)),
        ),
        None => Line::raw(""),
    });
    lines.push(Line::styled(
        "↑/↓ move, ←/→ or Space change, Enter compress, Esc cancel",
        dim_style,
    ));
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Label column of the extract form, wide enough for "Existing files".
const EXTRACT_LABEL_COLUMN: usize = 18;

/// The extract form's panel.
pub fn extract_area(frame: Rect, popup: &ExtractPopup) -> Rect {
    panel_area(frame, popup.rows().len())
}

/// Where the form's rows are drawn, laid out like the compress form's.
fn extract_inner(area: Rect) -> Rect {
    area.inner(Margin::new(2, 1))
}

/// What a click landed on in the extract form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractHit {
    Row(usize),
    /// Inside the panel but not on a row.
    Panel,
    Outside,
}

pub fn extract_hit(frame: Rect, popup: &ExtractPopup, at: ratatui::layout::Position) -> ExtractHit {
    let area = extract_area(frame, popup);
    if !area.contains(at) {
        return ExtractHit::Outside;
    }
    let inner = extract_inner(area);
    let first = inner.y.saturating_add(1);
    let row = usize::from(at.y.saturating_sub(first));
    if at.y >= first && row < popup.rows().len() && at.x >= inner.x && at.x < inner.right() {
        ExtractHit::Row(row)
    } else {
        ExtractHit::Panel
    }
}

/// Draws the extract form: one row per choice with the focused row highlighted, and the folder
/// with a cursor and a note when it is empty (extract into the browsed directory).
pub fn render_extract(frame: &mut Frame<'_>, popup: &ExtractPopup, config: &Config) {
    let theme = &config.theme;
    let area = extract_area(frame.area(), popup);
    frame.render_widget(Clear, area);
    let block = style::themed_block(config, &popup.title(), true);
    frame.render_widget(block, area);
    let inner = extract_inner(area);

    let label_style = style::styled(
        Style::default().fg(style::color(&theme.accent_fg)),
        config.styles.title,
    );
    let value_style = Style::default().fg(style::color(&theme.file_fg));
    let dim_style = Style::default().fg(style::color(&theme.border_fg));
    let selected_style = Style::default()
        .bg(style::color(&theme.selection_bg))
        .fg(style::color(&theme.file_fg));
    let value_width = usize::from(inner.width).saturating_sub(EXTRACT_LABEL_COLUMN);

    let mut lines = vec![Line::raw("")];
    lines.extend(popup.rows().iter().enumerate().map(|(i, &row)| {
        let base = if i == popup.cursor() {
            selected_style
        } else {
            Style::default()
        };
        let inert = row == ExtractRow::Folder && !popup.folder_applies();
        let mut spans = vec![Span::styled(
            pad_to(row.label(), EXTRACT_LABEL_COLUMN),
            label_style.patch(base),
        )];
        if row == ExtractRow::Folder && !inert {
            // A bar after the text marks where typing goes; an empty name says what that means.
            let note = if popup.folder().is_empty() {
                "(empty: extract into this directory)"
            } else {
                "/"
            };
            let room = value_width.saturating_sub(text_width(note) + 1);
            spans.push(Span::styled(
                fit_tail(popup.folder(), room),
                value_style.patch(base),
            ));
            if i == popup.cursor() {
                spans.push(Span::styled("▏", value_style.patch(base)));
            }
            spans.push(Span::styled(note, dim_style.patch(base)));
        } else {
            let style = if inert { dim_style } else { value_style };
            spans.push(Span::styled(
                fit_width(&popup.value(row), value_width),
                style.patch(base),
            ));
        }
        Line::from(spans)
    }));
    lines.push(match popup.error() {
        Some(message) => Line::styled(
            fit_width(message, usize::from(inner.width)),
            Style::default().fg(style::color(&theme.danger_fg)),
        ),
        None => Line::raw(""),
    });
    lines.push(Line::styled(
        "↑/↓ move, ←/→ or Space change, Enter extract, Esc cancel",
        dim_style,
    ));
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The end of `text` that fits in `width` cells, with a leading `…` when the start is cut, since
/// what is being typed is at the end.
fn fit_tail(text: &str, width: usize) -> String {
    if text_width(text) <= width {
        return text.to_owned();
    }
    let mut kept: Vec<char> = Vec::new();
    let mut used = 1;
    for c in text.chars().rev() {
        let w = text_width(&c.to_string());
        if used + w > width {
            break;
        }
        used += w;
        kept.push(c);
    }
    kept.push('…');
    kept.reverse();
    kept.into_iter().collect()
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

/// A color row's live/edited value as a two-cell swatch, painted in that color — so a color row
/// reads at a glance without decoding the hex/name text next to it.
const SWATCH_WIDTH: usize = 3;

/// `H 210° S 80% V 100%`, plus the hex it resolves to — the readout line below the gradient
/// square and hue strip while a color row's picker mode is open.
fn picker_readout(hsv: Hsv) -> String {
    format!(
        "H {:.0}°  S {:.0}%  V {:.0}%   {}",
        hsv.h,
        hsv.s,
        hsv.v,
        hsv.to_hex()
    )
}

/// The fg color that reads clearly against an `(r, g, b)` background: white on a dark cell, black
/// on a light one — used for the picker's crosshair marker, which sits on a different background
/// color every time the hue or the cursor position changes.
fn readable_on(r: u8, g: u8, b: u8) -> Color {
    if Theme::is_dark(r, g, b) {
        Color::White
    } else {
        Color::Black
    }
}

/// The saturation/value gradient square for `hsv`'s hue, `width` cells wide and `SV_BOX_HEIGHT`
/// tall, each cell painted with the color it represents (a real 2D color picker, not a slider) —
/// a "+"/"◉" marks the cell nearest the current saturation/value. `hsv_from_sv_click` is this
/// function's inverse: the same `width`/`SV_BOX_HEIGHT` grid a click is hit-tested against, so
/// the two can never disagree about which cell is where.
fn sv_box_lines(hsv: Hsv, width: u16, ascii: bool) -> Vec<Line<'static>> {
    let width = width.max(1);
    let marker = if ascii { "+" } else { "◉" };
    let marker_x = ((hsv.s / 100.0 * f64::from(width)) as u16).min(width - 1);
    let marker_y =
        (((100.0 - hsv.v) / 100.0 * f64::from(SV_BOX_HEIGHT)) as u16).min(SV_BOX_HEIGHT - 1);
    (0..SV_BOX_HEIGHT)
        .map(|y| {
            let spans: Vec<Span<'static>> = (0..width)
                .map(|x| {
                    let s = (f64::from(x) + 0.5) / f64::from(width) * 100.0;
                    let v = 100.0 - (f64::from(y) + 0.5) / f64::from(SV_BOX_HEIGHT) * 100.0;
                    let (r, g, b) = Hsv { h: hsv.h, s, v }.to_rgb();
                    let bg = Style::default().bg(Color::Rgb(r, g, b));
                    if x == marker_x && y == marker_y {
                        Span::styled(marker, bg.fg(readable_on(r, g, b)))
                    } else {
                        Span::styled(" ", bg)
                    }
                })
                .collect();
            Line::from(spans)
        })
        .collect()
}

/// The one-row hue strip below the gradient square: the full 0°–360° range at full saturation and
/// value, `width` cells wide — a "^"/"▲" marks the cell nearest the current hue. Its own inverse
/// is `hue_from_strip_click`, the same grid a click is hit-tested against.
fn hue_strip_line(hsv: Hsv, width: u16, ascii: bool) -> Line<'static> {
    let width = width.max(1);
    let marker = if ascii { "^" } else { "▲" };
    let marker_x = ((hsv.h / 360.0 * f64::from(width)) as u16).min(width - 1);
    let spans: Vec<Span<'static>> = (0..width)
        .map(|x| {
            let h = (f64::from(x) + 0.5) / f64::from(width) * 360.0;
            let (r, g, b) = Hsv {
                h,
                s: 100.0,
                v: 100.0,
            }
            .to_rgb();
            let bg = Style::default().bg(Color::Rgb(r, g, b));
            if x == marker_x {
                Span::styled(marker, bg.fg(readable_on(r, g, b)))
            } else {
                Span::styled(" ", bg)
            }
        })
        .collect();
    Line::from(spans)
}

/// Draws the appearance popup: one row per setting, the cursor's row highlighted the same way a
/// selected file is. A color row carries a live swatch of its own value next to it. The row being
/// edited shows its in-progress buffer (or, in picker mode, its HSV sliders) instead of its
/// committed value, and the "Save theme" flow — an update-or-new choice, then a name — takes over
/// the hint line while it's open. `Reset to defaults` is styled like a destructive action, the
/// same as `Delete` in the right-click menu. Session-only to draw; what it's drawing may already
/// be saved to `local.toml` by the time this runs (`main` persists on every commit).
pub fn render_appearance(
    frame: &mut Frame<'_>,
    popup: &AppearancePopup,
    view: &AppearanceView<'_>,
    config: &Config,
) {
    let theme = &config.theme;
    let rows = popup.rows();
    let editing_picker = popup.editing_picker();
    let extra = if editing_picker.is_some() {
        usize::from(PICKER_EXTRA_LINES)
    } else {
        0
    };
    let area = panel_area(frame.area(), rows.len() + extra);
    frame.render_widget(Clear, area);
    let title = match popup.view() {
        AppearanceViewLevel::Root => "appearance".to_string(),
        AppearanceViewLevel::Category(cat) => {
            format!("appearance: {}", cat.label().trim_end_matches(" ›"))
        }
    };
    let block = style::themed_block(config, &title, true);
    let inner = block.inner(area).inner(Margin::new(1, 0));
    frame.render_widget(block, area);

    let value_width = usize::from(inner.width).saturating_sub(APPEARANCE_LABEL_COLUMN);
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
    let hint_style = Style::default().fg(style::color(&theme.border_fg));

    let editing_row = popup.editing_row();
    let ascii = glyphs::of(config).ascii_borders;
    let caret = if ascii { "_" } else { "▏" };
    let swatch_glyph = if ascii { "##" } else { "██" };

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
        let is_color = matches!(row.kind(), Some(AppearanceRowKind::Color));
        let being_edited = editing_row == Some(*row);

        // A saved theme's row is labelled with its own name (which `Row::label` can't return, being
        // `'static`) and marked when it is the one the live look started from.
        let (row_label, saved_marker) = match *row {
            AppearanceRow::SavedTheme(index) => (
                popup.saved_name(index).unwrap_or_default(),
                if popup.saved_name(index) == view.active_custom_theme {
                    "active"
                } else {
                    ""
                },
            ),
            other => (other.label(), ""),
        };
        let value = if being_edited {
            match editing_picker {
                Some(hsv) => picker_readout(hsv),
                None => format!("{}{caret}", popup.editing_buffer().unwrap_or_default()),
            }
        } else if matches!(row, AppearanceRow::SavedTheme(_)) {
            saved_marker.to_string()
        } else {
            view.value(*row)
        };
        // The swatch tracks whatever's live — the in-progress edit if there is one, or the
        // committed value otherwise — so nudging the picker or typing a hex repaints it
        // immediately.
        let swatch_hex = match (being_edited, editing_picker, popup.editing_buffer()) {
            (true, Some(hsv), _) => hsv.to_hex(),
            (true, None, Some(buffer)) => buffer.to_string(),
            _ => view.value(*row),
        };

        let mut spans = vec![Span::styled(
            pad_to(row_label, APPEARANCE_LABEL_COLUMN),
            label.patch(base),
        )];
        let value_area = if is_color {
            spans.push(Span::styled(
                format!("{swatch_glyph} "),
                Style::default().fg(style::color(&swatch_hex)).patch(base),
            ));
            value_width.saturating_sub(SWATCH_WIDTH)
        } else {
            value_width
        };
        spans.push(Span::styled(
            fit_width(&value, value_area),
            value_style.patch(base),
        ));
        Line::from(spans)
    }));
    lines.push(Line::raw(""));
    if let Some(hsv) = editing_picker {
        let box_width = appearance_popup::SV_BOX_WIDTH.min(inner.width);
        lines.extend(sv_box_lines(hsv, box_width, ascii));
        lines.push(Line::raw(""));
        lines.push(hue_strip_line(hsv, box_width, ascii));
        lines.push(Line::raw(""));
        lines.push(Line::styled(picker_readout(hsv), value_style));
    }
    let editing_is_color = editing_row
        .map(|row| matches!(row.kind(), Some(AppearanceRowKind::Color)))
        .unwrap_or(false);
    let managing_hint = match popup.manage_view() {
        Some(ManageView::Rename(buffer)) => Some(Line::from(vec![
            Span::styled("rename to: ", label_style),
            Span::styled(
                format!("{buffer}{caret}  enter saves, Esc cancels"),
                value_style,
            ),
        ])),
        Some(ManageView::ConfirmDelete(name)) => Some(Line::styled(
            format!("delete '{name}'?  y: delete   n/Esc: keep"),
            hint_style,
        )),
        None => None,
    };
    let in_saved_themes = matches!(
        popup.view(),
        AppearanceViewLevel::Category(appearance_popup::Category::SavedThemes)
    );
    lines.push(match popup.save_view() {
        _ if managing_hint.is_some() => managing_hint.expect("checked by the guard"),
        Some(SaveView::Choice(name)) => Line::styled(
            format!("u: update '{name}'   n: save as new   Esc: cancel"),
            hint_style,
        ),
        Some(SaveView::Name(buffer)) => Line::from(vec![
            Span::styled("name: ", label_style),
            Span::styled(
                format!("{buffer}{caret}  enter saves, Esc cancels"),
                value_style,
            ),
        ]),
        None if editing_picker.is_some() => Line::styled(
            "click/drag the square or the hue bar, arrows move, [ ] adjusts hue, tab for hex, \
             enter confirms, Esc cancels",
            hint_style,
        ),
        None if editing_row.is_some() && editing_is_color => Line::styled(
            "type to edit, tab for the color picker, enter confirms, Esc cancels",
            hint_style,
        ),
        None if editing_row.is_some() => {
            Line::styled("type to edit, enter confirms, Esc cancels", hint_style)
        }
        None if in_saved_themes && popup.rows().len() == 1 => Line::styled(
            "nothing saved yet — use \"Save theme\" on the main screen, Esc goes back",
            hint_style,
        ),
        None if in_saved_themes => Line::styled(
            "enter applies, r renames, d deletes, Esc goes back",
            hint_style,
        ),
        None => Line::styled(
            "j/k moves, enter opens/edits/cycles, a click acts, Esc back/closes — session only, \
             not saved",
            hint_style,
        ),
    });
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
        Config::from_sources(None, None, None)
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
            archive: false,
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
        // Tall enough for the whole file menu (it grew a row with "Compress…").
        let mut terminal = Terminal::new(TestBackend::new(40, 17)).unwrap();
        let menu = file_menu((39, 16));
        terminal
            .draw(|frame| render_menu(frame, &menu, &config()))
            .unwrap();
        assert!(screen_text(&terminal).contains("Inspect"));
    }

    fn compress_form() -> CompressPopup {
        CompressPopup::new(
            vec![
                std::path::PathBuf::from("/w/a"),
                std::path::PathBuf::from("/w/b"),
            ],
            std::path::PathBuf::from("/w"),
        )
        .unwrap()
    }

    #[test]
    fn the_compress_form_draws_every_row_with_the_extension_after_the_name() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let form = compress_form();
        terminal
            .draw(|frame| render_compress(frame, &form, &config()))
            .unwrap();
        let text = screen_text(&terminal);
        for label in [
            "Name",
            "Format",
            "Level",
            "One per item",
            "Delete originals",
        ] {
            assert!(text.contains(label), "{label}");
        }
        assert!(text.contains("archive▏.zip"), "{text}");
        assert!(text.contains("compress: 2 items"));
    }

    #[test]
    fn a_click_on_a_compress_row_hits_that_row_and_outside_the_panel_misses() {
        let frame = Rect::new(0, 0, 100, 30);
        let form = compress_form();
        let area = compress_area(frame, &form);
        let inner = compress_inner(area);
        for row in 0..form.rows().len() {
            let at = Position::new(inner.x + 2, inner.y + 1 + row as u16);
            assert_eq!(compress_hit(frame, &form, at), CompressHit::Row(row));
        }
        // The blank line above the first row, and the hint line, are inside but on no row.
        assert_eq!(
            compress_hit(frame, &form, Position::new(inner.x + 2, inner.y)),
            CompressHit::Panel
        );
        assert_eq!(
            compress_hit(frame, &form, Position::new(0, 0)),
            CompressHit::Outside
        );
    }

    fn extract_form() -> ExtractPopup {
        ExtractPopup::new(
            vec![std::path::PathBuf::from("/w/photos.tar.gz")],
            std::path::PathBuf::from("/w"),
        )
        .unwrap()
    }

    #[test]
    fn the_extract_form_draws_every_row_with_the_folder_and_its_cursor() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let form = extract_form();
        terminal
            .draw(|frame| render_extract(frame, &form, &config()))
            .unwrap();
        let text = screen_text(&terminal);
        for label in ["Folder", "Existing files", "Delete archives"] {
            assert!(text.contains(label), "{label}");
        }
        assert!(text.contains("photos▏/"), "{text}");
        assert!(text.contains("extract: photos.tar.gz"));
    }

    #[test]
    fn a_click_on_an_extract_row_hits_that_row_and_outside_the_panel_misses() {
        let frame = Rect::new(0, 0, 100, 30);
        let form = extract_form();
        let area = extract_area(frame, &form);
        let inner = extract_inner(area);
        for row in 0..form.rows().len() {
            let at = Position::new(inner.x + 2, inner.y + 1 + row as u16);
            assert_eq!(extract_hit(frame, &form, at), ExtractHit::Row(row));
        }
        assert_eq!(
            extract_hit(frame, &form, Position::new(inner.x + 2, inner.y)),
            ExtractHit::Panel
        );
        assert_eq!(
            extract_hit(frame, &form, Position::new(0, 0)),
            ExtractHit::Outside
        );
    }

    #[test]
    fn the_end_of_a_long_name_is_what_stays_visible() {
        assert_eq!(fit_tail("short", 10), "short");
        assert_eq!(fit_tail("abcdefghij", 5), "…ghij");
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
