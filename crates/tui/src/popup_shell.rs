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

//! The crossterm/ratatui side of `shell_overlay::PopupShell`: sizing the popup, translating key
//! events into the raw bytes a real terminal would send, and rendering the `vt100` screen buffer
//! as a bordered widget layered over the main UI. `shell_overlay` stays free of any UI-toolkit
//! dependency (beyond `vt100`, which is the screen-buffer data model both sides need) — it only
//! knows pty mechanics.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use shell_overlay::PopupShell;
use theming::Config;

/// The popup's on-screen rectangle: 80% of the frame's width and 70% of its height, clamped so
/// it never exceeds the frame itself (a tiny terminal just gets a full-bleed popup). `offset`
/// shifts it from its default centered position (as accumulated by move mode — see
/// `main.rs::run`); the result is clamped back into the frame so the popup can never be nudged
/// fully or partially off-screen.
pub(crate) fn popup_area(frame_area: Rect, offset: (i32, i32)) -> Rect {
    let width = (frame_area.width.saturating_mul(4) / 5)
        .max(20)
        .min(frame_area.width);
    let height = (frame_area.height.saturating_mul(7) / 10)
        .max(6)
        .min(frame_area.height);
    let centered_x = (frame_area.width.saturating_sub(width)) / 2;
    let centered_y = (frame_area.height.saturating_sub(height)) / 2;
    let max_x = frame_area.width.saturating_sub(width) as i32;
    let max_y = frame_area.height.saturating_sub(height) as i32;
    let x = (centered_x as i32 + offset.0).clamp(0, max_x) as u16;
    let y = (centered_y as i32 + offset.1).clamp(0, max_y) as u16;
    Rect::new(x, y, width, height)
}

/// The pty size (rows, cols) for a popup whose bordered outer rect is `area` — one cell of
/// border on every side, so the pty only ever sees the space actually available for output.
pub(crate) fn pty_size(area: Rect) -> (u16, u16) {
    (
        area.height.saturating_sub(2).max(1),
        area.width.saturating_sub(2).max(1),
    )
}

/// Encodes a key event into the raw bytes a real terminal emits for it (the same escape
/// sequences `xterm`-family terminals use for arrows, function keys, etc.), so it can be
/// forwarded straight to the popup's pty. Returns `None` for events with no terminal
/// representation, such as a bare modifier-key press.
pub(crate) fn encode_key(key: KeyEvent) -> Option<Vec<u8>> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    let mut bytes = match key.code {
        KeyCode::Char(' ') if ctrl => vec![0u8],
        KeyCode::Char(c) if ctrl && c.is_ascii_alphabetic() => {
            vec![c.to_ascii_uppercase() as u8 & 0x1f]
        }
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::F(n) => function_key_sequence(n)?,
        _ => return None,
    };

    if alt {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

/// The classic xterm function-key sequences. F13+ have no widely-agreed-on sequence, so they're
/// left unsupported rather than guessed at.
fn function_key_sequence(n: u8) -> Option<Vec<u8>> {
    Some(match n {
        1 => b"\x1bOP".to_vec(),
        2 => b"\x1bOQ".to_vec(),
        3 => b"\x1bOR".to_vec(),
        4 => b"\x1bOS".to_vec(),
        5 => b"\x1b[15~".to_vec(),
        6 => b"\x1b[17~".to_vec(),
        7 => b"\x1b[18~".to_vec(),
        8 => b"\x1b[19~".to_vec(),
        9 => b"\x1b[20~".to_vec(),
        10 => b"\x1b[21~".to_vec(),
        11 => b"\x1b[23~".to_vec(),
        12 => b"\x1b[24~".to_vec(),
        _ => return None,
    })
}

/// Renders the popup's current screen buffer as a bordered widget over `area`, and positions
/// the real terminal cursor over the child's cursor cell (when it isn't hidden).
pub(crate) fn render(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    popup: &PopupShell,
    config: &Config,
) {
    frame.render_widget(Clear, area);
    let block = crate::themed_block(config, "shell");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let default_fg = crate::color_from_name(&config.theme.file_fg);
    let (lines, cursor, hide_cursor) = popup.with_screen(|screen| {
        let (rows, cols) = screen.size();
        let lines: Vec<Line<'static>> = (0..rows)
            .map(|row| cell_row_to_line(screen, row, cols, default_fg))
            .collect();
        (lines, screen.cursor_position(), screen.hide_cursor())
    });

    frame.render_widget(Paragraph::new(lines), inner);

    let (cursor_row, cursor_col) = cursor;
    if !hide_cursor && cursor_row < inner.height && cursor_col < inner.width {
        frame.set_cursor_position((inner.x + cursor_col, inner.y + cursor_row));
    }
}

fn cell_row_to_line(
    screen: &vt100::Screen,
    row: u16,
    cols: u16,
    default_fg: Color,
) -> Line<'static> {
    let spans: Vec<Span<'static>> = (0..cols)
        .map(|col| {
            let Some(cell) = screen.cell(row, col) else {
                return Span::raw(" ");
            };
            let text = if cell.has_contents() {
                cell.contents().to_string()
            } else {
                " ".to_string()
            };
            let mut style = Style::default()
                .fg(vt100_color(cell.fgcolor(), default_fg))
                .bg(vt100_color(cell.bgcolor(), Color::Reset));
            if cell.bold() {
                style = style.add_modifier(Modifier::BOLD);
            }
            if cell.italic() {
                style = style.add_modifier(Modifier::ITALIC);
            }
            if cell.underline() {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            if cell.inverse() {
                style = style.add_modifier(Modifier::REVERSED);
            }
            Span::styled(text, style)
        })
        .collect();
    Line::from(spans)
}

fn vt100_color(color: vt100::Color, default: Color) -> Color {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn plain_char_forwards_its_utf8_bytes() {
        assert_eq!(
            encode_key(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(b"a".to_vec())
        );
    }

    #[test]
    fn ctrl_letter_maps_to_its_control_code() {
        // Ctrl-C, the most common one to get wrong.
        assert_eq!(
            encode_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(vec![0x03])
        );
    }

    #[test]
    fn arrow_keys_use_the_standard_xterm_sequences() {
        assert_eq!(
            encode_key(key(KeyCode::Up, KeyModifiers::NONE)),
            Some(b"\x1b[A".to_vec())
        );
    }

    #[test]
    fn alt_prefixes_esc_to_the_underlying_sequence() {
        assert_eq!(
            encode_key(key(KeyCode::Char('x'), KeyModifiers::ALT)),
            Some(b"\x1bx".to_vec())
        );
    }

    #[test]
    fn unrepresentable_keys_return_none() {
        assert_eq!(
            encode_key(KeyEvent {
                code: KeyCode::CapsLock,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: crossterm::event::KeyEventState::NONE,
            }),
            None
        );
    }

    #[test]
    fn popup_area_never_exceeds_the_frame_and_stays_centered() {
        let frame = Rect::new(0, 0, 100, 40);
        let popup = popup_area(frame, (0, 0));
        assert!(popup.width <= frame.width && popup.height <= frame.height);
        assert_eq!(popup.x, (frame.width - popup.width) / 2);
        assert_eq!(popup.y, (frame.height - popup.height) / 2);
    }

    #[test]
    fn popup_area_clamps_to_a_tiny_frame() {
        let frame = Rect::new(0, 0, 10, 4);
        let popup = popup_area(frame, (0, 0));
        assert_eq!(popup.width, frame.width);
        assert_eq!(popup.height, frame.height);
    }

    #[test]
    fn popup_area_offset_shifts_the_popup_from_center() {
        let frame = Rect::new(0, 0, 100, 40);
        let centered = popup_area(frame, (0, 0));
        let moved = popup_area(frame, (5, -3));
        assert_eq!(moved.x, centered.x + 5);
        assert_eq!(moved.y, centered.y - 3);
    }

    #[test]
    fn popup_area_offset_clamps_within_the_frame() {
        let frame = Rect::new(0, 0, 100, 40);
        let clamped_high = popup_area(frame, (10_000, 10_000));
        let max_x = frame.width - clamped_high.width;
        let max_y = frame.height - clamped_high.height;
        assert_eq!(clamped_high.x, max_x);
        assert_eq!(clamped_high.y, max_y);

        let clamped_low = popup_area(frame, (-10_000, -10_000));
        assert_eq!(clamped_low.x, 0);
        assert_eq!(clamped_low.y, 0);
    }

    #[test]
    fn pty_size_accounts_for_the_border() {
        assert_eq!(pty_size(Rect::new(0, 0, 82, 26)), (24, 80));
    }

    #[test]
    fn pty_size_never_goes_below_one_cell() {
        assert_eq!(pty_size(Rect::new(0, 0, 1, 1)), (1, 1));
    }
}
