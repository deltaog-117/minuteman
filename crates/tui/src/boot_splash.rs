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

//! A brief, dismissible splash drawn over the browser before the first real keystroke — the
//! cinematic layer's boot flourish. `main` owns the one `BootSplash` for the whole time it's up:
//! any key or mouse event dismisses it immediately (checked ahead of everything else `run`'s
//! event loop does), and it also clears itself once [`DURATION`] has passed with no input at all.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};
use theming::Config;

use crate::{gradient, style};

/// How long the splash stays up on its own before clearing itself.
pub const DURATION: Duration = Duration::from_millis(900);

/// How long the title/subtitle take to fade in from dim to full color.
const FADE: Duration = Duration::from_millis(300);

const WIDTH: u16 = 46;
const HEIGHT: u16 = 7;

pub struct BootSplash {
    started: Instant,
}

impl BootSplash {
    pub fn start(now: Instant) -> Self {
        Self { started: now }
    }

    /// Whether [`DURATION`] has passed since `start` with nothing dismissing it early.
    pub fn done(&self, now: Instant) -> bool {
        now.duration_since(self.started) >= DURATION
    }

    /// How far the fade-in has progressed: `0.0` at `start`, `1.0` from [`FADE`] onward.
    fn fade(&self, now: Instant) -> f64 {
        (now.duration_since(self.started).as_secs_f64() / FADE.as_secs_f64()).min(1.0)
    }
}

/// The splash's rectangle: a fixed size, centered, clamped to the frame so it can never panic on
/// a tiny terminal.
fn area(frame: Rect) -> Rect {
    let width = WIDTH.min(frame.width);
    let height = HEIGHT.min(frame.height);
    Rect::new(
        frame.x + frame.width.saturating_sub(width) / 2,
        frame.y + frame.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

pub fn render(frame: &mut Frame<'_>, frame_area: Rect, splash: &BootSplash, config: &Config) {
    let area = area(frame_area);
    if area.width < 4 || area.height < 4 {
        return; // nothing legible fits; skip rather than draw a broken box.
    }
    frame.render_widget(Clear, area);
    let theme = &config.theme;
    let block = style::themed_block(config, "", false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if config.gradient_borders {
        gradient::paint(
            frame.buffer_mut(),
            area,
            style::color(&theme.accent_fg),
            style::color(&theme.border_focused_fg),
        );
    }

    let dim = style::color(&theme.border_fg);
    let bright = style::color(&theme.accent_fg);
    let text_color = style::blend_rgb(dim, bright, splash.fade(Instant::now())).unwrap_or(bright);

    let centered = |text: &str, style: Style| {
        Line::styled(text.to_string(), style).alignment(ratatui::layout::Alignment::Center)
    };
    let lines = vec![
        Line::raw(""),
        centered(
            "MINUTEMAN",
            Style::default().fg(text_color).add_modifier(Modifier::BOLD),
        ),
        centered(
            "a Ranger-inspired terminal file manager",
            Style::default().fg(style::color(&theme.file_fg)),
        ),
        Line::raw(""),
        centered("press any key", Style::default().fg(dim)),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_not_done_until_duration_passes() {
        let splash = BootSplash::start(Instant::now());
        assert!(!splash.done(Instant::now()));
    }

    #[test]
    fn fade_reaches_and_holds_at_one() {
        let splash = BootSplash::start(Instant::now() - Duration::from_secs(10));
        assert_eq!(splash.fade(Instant::now()), 1.0);
    }

    #[test]
    fn area_never_exceeds_a_tiny_frame() {
        let a = area(Rect::new(0, 0, 3, 3));
        assert!(a.width <= 3 && a.height <= 3);
    }

    #[test]
    fn area_is_centered_in_a_roomy_frame() {
        let frame = Rect::new(0, 0, 120, 40);
        let a = area(frame);
        assert_eq!(a.width, WIDTH);
        assert_eq!(a.height, HEIGHT);
        assert_eq!(a.x, (120 - WIDTH) / 2);
        assert_eq!(a.y, (40 - HEIGHT) / 2);
    }
}
