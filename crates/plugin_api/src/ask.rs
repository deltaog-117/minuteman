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

//! A plugin asking the user something it cannot settle itself: a password or passphrase, or
//! whether to trust a key it has not seen. The plugin owns the question (it is `ssh` that asks it);
//! the host draws it over the browser, takes the answer and sends it back as the reply to the
//! `ask` request. Nothing about the answer is kept by the host.
//!
//! A plugin may be wrong or hostile and its text ends up in the user's terminal, so
//! [`AskSpec::checked`] cleans the prompt before it is drawn.

use serde::{Deserialize, Serialize};

use crate::archive::Password;
use crate::view::is_unsafe;

/// The request that asks the user something: `ask`. The host answers with an error when the user
/// declines, or when the interface cannot ask right now.
pub const METHOD_ASK: &str = "ask";

/// Most characters of a prompt; longer text is cut. A host-key question is about four lines of
/// under a hundred characters each.
pub const MAX_ASK_PROMPT_CHARS: usize = 1024;

/// What is being asked for, and so how the answer is typed and shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskKind {
    /// A line the user types and the screen does not echo.
    Secret,
    /// A yes or a no.
    Confirm,
}

/// The parameters of `ask`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskSpec {
    pub kind: AskKind,
    /// What the user is told; line breaks are kept.
    pub prompt: String,
}

impl AskSpec {
    /// This question with its text cleaned: no control characters but the line break, no
    /// direction overrides, and no more than [`MAX_ASK_PROMPT_CHARS`] characters.
    pub fn checked(self) -> Self {
        let prompt = self
            .prompt
            .replace('\t', " ")
            .chars()
            .filter(|&c| c == '\n' || !is_unsafe(c))
            .take(MAX_ASK_PROMPT_CHARS)
            .collect::<String>()
            .trim()
            .to_string();
        Self { prompt, ..self }
    }
}

/// What the host replies with when the user answered. Declining is an error reply instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AskAnswer {
    Secret { secret: Password },
    Confirm { yes: bool },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prompt_loses_what_could_steer_the_terminal_and_keeps_its_lines() {
        let spec = AskSpec {
            kind: AskKind::Confirm,
            prompt: "key is\x1b[2J\n SHA256:abc\u{202E}\tok?".to_string(),
        };
        assert_eq!(spec.checked().prompt, "key is[2J\n SHA256:abc ok?");
    }

    #[test]
    fn a_long_prompt_is_cut() {
        let spec = AskSpec {
            kind: AskKind::Secret,
            prompt: "x".repeat(MAX_ASK_PROMPT_CHARS + 50),
        };
        assert_eq!(spec.checked().prompt.chars().count(), MAX_ASK_PROMPT_CHARS);
    }

    #[test]
    fn the_wire_names_the_kind_and_each_answer_reads_back_as_itself() {
        let spec = AskSpec {
            kind: AskKind::Secret,
            prompt: "Password:".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&spec).unwrap(),
            r#"{"kind":"secret","prompt":"Password:"}"#
        );
        let yes: AskAnswer = serde_json::from_str(r#"{"yes":true}"#).unwrap();
        assert_eq!(yes, AskAnswer::Confirm { yes: true });
        let secret: AskAnswer = serde_json::from_str(r#"{"secret":"pw"}"#).unwrap();
        let mut typed = Password::new();
        "pw".chars().for_each(|c| typed.push(c));
        assert_eq!(secret, AskAnswer::Secret { secret: typed });
        assert!(serde_json::from_str::<AskAnswer>(r#"{"secret":"a\u0007"}"#).is_err());
    }
}
