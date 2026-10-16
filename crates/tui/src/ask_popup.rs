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

//! The popup a plugin's question is shown in: a password or passphrase typed without echo, or a
//! yes or no. Pure state, like `history_popup`: `overlay_view` draws it and `main` feeds it keys
//! and sends the outcome back to the plugin that asked.
//!
//! A question about trust (`Confirm`) is only a yes on an explicit `y`: `Enter` does nothing, so a
//! key pressed to dismiss something else cannot accept a host key.

use crossterm::event::KeyCode;
use plugin_api::Password;
use plugin_host::protocol::{AskAnswer, AskKind};

/// What a keystroke asks the caller to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Key {
    Stay,
    /// The user answered.
    Answer(AskAnswer),
    /// The user backed out.
    Decline,
}

pub struct AskPopup {
    plugin: String,
    kind: AskKind,
    prompt: String,
    typed: Password,
}

impl AskPopup {
    pub fn new(plugin: &str, kind: AskKind, prompt: &str) -> Self {
        Self {
            plugin: plugin.to_owned(),
            kind,
            prompt: prompt.to_owned(),
            typed: Password::new(),
        }
    }

    /// The plugin that asks, named by the host so a plugin cannot pass itself off as another.
    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    pub fn kind(&self) -> AskKind {
        self.kind
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// How many characters have been typed, for the dots drawn in their place.
    pub fn typed_len(&self) -> usize {
        self.typed.char_count()
    }

    /// A secret takes text, `Backspace`, `Enter` to send and `Esc` to back out. A question takes
    /// `y` or `n` and `Esc`.
    pub fn key(&mut self, code: KeyCode) -> Key {
        match (self.kind, code) {
            (_, KeyCode::Esc) => Key::Decline,
            (AskKind::Secret, KeyCode::Enter) => Key::Answer(AskAnswer::Secret {
                secret: self.typed.clone(),
            }),
            (AskKind::Secret, KeyCode::Backspace) => {
                self.typed.pop();
                Key::Stay
            }
            (AskKind::Secret, KeyCode::Char(c)) => {
                self.typed.push(c);
                Key::Stay
            }
            (AskKind::Confirm, KeyCode::Char('y' | 'Y')) => {
                Key::Answer(AskAnswer::Confirm { yes: true })
            }
            (AskKind::Confirm, KeyCode::Char('n' | 'N')) => {
                Key::Answer(AskAnswer::Confirm { yes: false })
            }
            _ => Key::Stay,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret_of(text: &str) -> AskAnswer {
        let mut secret = Password::new();
        text.chars().for_each(|c| secret.push(c));
        AskAnswer::Secret { secret }
    }

    #[test]
    fn a_secret_is_typed_corrected_and_sent_with_enter() {
        let mut popup = AskPopup::new("ssh", AskKind::Secret, "Password:");
        for c in "hunx".chars() {
            assert_eq!(popup.key(KeyCode::Char(c)), Key::Stay);
        }
        popup.key(KeyCode::Backspace);
        popup.key(KeyCode::Char('t'));
        assert_eq!(popup.typed_len(), 4);
        assert_eq!(popup.key(KeyCode::Enter), Key::Answer(secret_of("hunt")));
    }

    #[test]
    fn a_secret_can_be_backed_out_of_with_escape() {
        let mut popup = AskPopup::new("ssh", AskKind::Secret, "Password:");
        popup.key(KeyCode::Char('a'));
        assert_eq!(popup.key(KeyCode::Esc), Key::Decline);
    }

    #[test]
    fn a_question_is_a_yes_only_on_an_explicit_y() {
        let mut popup = AskPopup::new("ssh", AskKind::Confirm, "Trust this key?");
        assert_eq!(popup.key(KeyCode::Enter), Key::Stay);
        assert_eq!(popup.key(KeyCode::Char(' ')), Key::Stay);
        assert_eq!(
            popup.key(KeyCode::Char('n')),
            Key::Answer(AskAnswer::Confirm { yes: false })
        );
        assert_eq!(
            popup.key(KeyCode::Char('Y')),
            Key::Answer(AskAnswer::Confirm { yes: true })
        );
        assert_eq!(popup.key(KeyCode::Esc), Key::Decline);
    }

    #[test]
    fn typing_into_a_question_does_not_collect_a_secret() {
        let mut popup = AskPopup::new("ssh", AskKind::Confirm, "Trust this key?");
        popup.key(KeyCode::Char('x'));
        assert_eq!(popup.typed_len(), 0);
    }
}
