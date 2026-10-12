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

//! A whole-body view that a plugin draws (`plugin_api::Canvas`): what the interface keeps of it
//! between frames, and how a key or the mouse reaches the plugin. It is pure state plus the
//! messages to send; `main` owns the terminal, `canvas_view` paints, and the plugin decides what
//! the picture means.
//!
//! The pane tells the plugin when the view opens and closes and how many cells it has, once per
//! change. It forwards only the keys the current canvas claims (the plugin cannot be asked
//! "do you want this key?" without waiting a frame for the answer) and every click and wheel turn
//! inside the body, in cells of the canvas. Until the first canvas arrives, and when the plugin has
//! stopped, it says so in place of the picture rather than showing nothing.

use std::path::Path;
use std::time::Instant;

use crossterm::event::{KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use plugin_api::{Button, Canvas, CanvasInput, Mods, SetCanvas, Surface};
use plugin_host::protocol::key_name;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Clear, Paragraph};
use theming::Config;

use crate::app::App;
use crate::canvas_view;
use crate::style;

/// How long a flowing mark stays on one cell of its path.
const FLOW_STEP_MS: u128 = 150;

/// What the view is called in its frame and in the messages about it.
fn label(surface: Surface) -> &'static str {
    match surface {
        Surface::Atlas => "atlas",
        Surface::Wiring => "wiring",
    }
}

/// A view a plugin draws, while it shows.
#[derive(Debug)]
pub struct CanvasPane {
    surface: Surface,
    canvas: Option<Canvas>,
    /// The size the plugin was last told, so it hears of a change once.
    told: (u16, u16),
    /// The plugin's process ended while the view showed, so no canvas will follow.
    stopped: bool,
    since: Instant,
}

impl CanvasPane {
    /// Opens `surface` over `body`, telling the plugin; `None`, with the reason on the status
    /// line, when no plugin can be reached.
    pub fn open(
        app: &mut App,
        surface: Surface,
        body: Rect,
        here: &Path,
        now: Instant,
    ) -> Option<Self> {
        let told = (body.width, body.height);
        let input = CanvasInput::Open {
            width: told.0,
            height: told.1,
            here: here.to_path_buf(),
        };
        if !app.dispatch_canvas(surface, input) {
            app.status = Some(crate::bundled::missing(surface.provides()));
            return None;
        }
        Some(Self {
            surface,
            canvas: None,
            told,
            stopped: false,
            since: now,
        })
    }

    /// The plugin's answer; one for another view is not this pane's. Returns the hint the new
    /// canvas wants on the status bar when it differs from the last one's, so a hint is said when
    /// it changes and not every time the picture does.
    pub fn accept(&mut self, set: SetCanvas) -> Option<String> {
        if set.surface != self.surface {
            return None;
        }
        let before = self.canvas.as_ref().and_then(|canvas| canvas.hint.clone());
        self.canvas = set.canvas;
        self.stopped = false;
        let after = self.canvas.as_ref().and_then(|canvas| canvas.hint.clone());
        (after != before).then_some(after).flatten()
    }

    /// Once per turn: lay out again if the body changed size, and notice a plugin that has ended.
    pub fn follow(&mut self, app: &mut App, body: Rect) {
        let size = (body.width, body.height);
        if size != self.told {
            self.told = size;
            let _ = app.dispatch_canvas(
                self.surface,
                CanvasInput::Resize {
                    width: size.0,
                    height: size.1,
                },
            );
        }
        if !self.stopped && !app.canvas_running(self.surface) {
            self.stopped = true;
        }
    }

    /// Tells the plugin the view is gone, so it stops looking at the machine.
    pub fn close(self, app: &mut App) {
        let _ = app.dispatch_canvas(self.surface, CanvasInput::Close);
    }

    /// Whether the canvas claims `key` right now.
    pub fn claims(&self, key: &KeyEvent) -> bool {
        let name = key_name(key.code);
        self.canvas
            .as_ref()
            .is_some_and(|canvas| canvas.claims.contains(&name))
    }

    /// Sends a key the canvas claims; `false` (nothing sent) for any other.
    pub fn key(&self, app: &mut App, key: &KeyEvent) -> bool {
        if !self.claims(key) {
            return false;
        }
        let input = CanvasInput::Key {
            key: key_name(key.code),
            mods: mods(key.modifiers),
        };
        let _ = app.dispatch_canvas(self.surface, input);
        true
    }

    /// Sends a click or a wheel turn at a cell of `body`; `false` (nothing sent) for a pointer
    /// outside it or an event that is neither.
    pub fn pointer(&self, app: &mut App, body: Rect, mouse: &MouseEvent) -> bool {
        if !body.contains(ratatui::layout::Position::new(mouse.column, mouse.row)) {
            return false;
        }
        let (x, y) = (mouse.column - body.x, mouse.row - body.y);
        let input = match mouse.kind {
            MouseEventKind::Down(button) => CanvasInput::Click {
                x,
                y,
                button: match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Middle => Button::Middle,
                    MouseButton::Right => Button::Right,
                },
                double: false,
                mods: mods(mouse.modifiers),
            },
            MouseEventKind::ScrollUp => CanvasInput::Wheel { x, y, down: false },
            MouseEventKind::ScrollDown => CanvasInput::Wheel { x, y, down: true },
            _ => return false,
        };
        let _ = app.dispatch_canvas(self.surface, input);
        true
    }

    /// Draws the view over `body`: the canvas, or why there is none yet.
    pub fn draw(&self, frame: &mut Frame<'_>, body: Rect, config: &Config, now: Instant) {
        frame.render_widget(Clear, body);
        match &self.canvas {
            Some(canvas) => {
                let phase =
                    u64::try_from(now.duration_since(self.since).as_millis() / FLOW_STEP_MS)
                        .unwrap_or(0);
                canvas_view::paint(frame.buffer_mut(), body, canvas, config, phase);
            }
            None => {
                let note = if self.stopped {
                    crate::bundled::missing(self.surface.provides())
                } else {
                    "loading…".to_string()
                };
                draw_note(frame, body, config, self.surface, &note);
            }
        }
    }
}

/// Draws the view of `surface` as its frame holding only `note`, for the moment there is no
/// canvas: while it loads, and when no plugin can draw it.
pub fn draw_note(frame: &mut Frame<'_>, body: Rect, config: &Config, surface: Surface, note: &str) {
    frame.render_widget(Clear, body);
    frame.render_widget(style::themed_block(config, label(surface), true), body);
    let inner = Rect::new(
        body.x.saturating_add(2),
        body.y.saturating_add(1),
        body.width.saturating_sub(4),
        body.height.saturating_sub(2),
    );
    frame.render_widget(
        Paragraph::new(note.to_string())
            .style(Style::default().fg(style::color(&config.theme.status_fg)))
            .wrap(ratatui::widgets::Wrap { trim: true }),
        inner,
    );
}

fn mods(modifiers: KeyModifiers) -> Mods {
    Mods {
        ctrl: modifiers.contains(KeyModifiers::CONTROL),
        alt: modifiers.contains(KeyModifiers::ALT),
        shift: modifiers.contains(KeyModifiers::SHIFT),
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;
    use plugin_api::{Cells, Item, Paint};

    use super::*;

    fn pane(claims: &[&str]) -> CanvasPane {
        CanvasPane {
            surface: Surface::Atlas,
            canvas: Some(Canvas {
                width: 4,
                height: 2,
                items: vec![Item::Block {
                    at: Cells {
                        x: 0,
                        y: 0,
                        w: 4,
                        h: 2,
                    },
                    paint: Paint::default(),
                    level: None,
                }],
                claims: claims.iter().map(|c| (*c).to_string()).collect(),
                hint: Some("j/k move".into()),
            }),
            told: (4, 2),
            stopped: false,
            since: Instant::now(),
        }
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn only_the_keys_the_canvas_claims_are_the_plugins() {
        let pane = pane(&["j", "enter", "space"]);
        assert!(pane.claims(&press(KeyCode::Char('j'))));
        assert!(pane.claims(&press(KeyCode::Enter)));
        assert!(pane.claims(&press(KeyCode::Char(' '))));
        assert!(!pane.claims(&press(KeyCode::Char('q'))));
        assert!(!pane.claims(&press(KeyCode::Esc)));
    }

    #[test]
    fn before_the_first_canvas_no_key_is_claimed() {
        let mut pane = pane(&["j"]);
        pane.canvas = None;
        assert!(!pane.claims(&press(KeyCode::Char('j'))));
    }

    #[test]
    fn an_answer_for_another_view_is_ignored_and_one_for_this_replaces_the_canvas() {
        let mut pane = pane(&["j"]);
        pane.accept(SetCanvas {
            surface: Surface::Wiring,
            canvas: None,
        });
        assert!(pane.canvas.is_some());
        pane.accept(SetCanvas {
            surface: Surface::Atlas,
            canvas: None,
        });
        assert!(pane.canvas.is_none());
    }

    #[test]
    fn a_hint_is_said_when_it_changes_and_not_when_it_stays() {
        let mut pane = pane(&[]);
        let same = pane.canvas.clone();
        assert_eq!(
            pane.accept(SetCanvas {
                surface: Surface::Atlas,
                canvas: same
            }),
            None
        );
        let mut other = pane.canvas.clone();
        if let Some(canvas) = other.as_mut() {
            canvas.hint = Some("enter zoom".into());
        }
        assert_eq!(
            pane.accept(SetCanvas {
                surface: Surface::Atlas,
                canvas: other
            }),
            Some("enter zoom".into())
        );
    }

    #[test]
    fn modifiers_are_passed_on_as_they_were_held() {
        assert_eq!(
            mods(KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            Mods {
                ctrl: true,
                alt: false,
                shift: true
            }
        );
    }
}
