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

//! What a plugin sends to put something on screen without drawing it: `set_view` fills a named
//! slot of the interface with a small document of styled lines. The plugin says what the text is
//! and how it should feel (a tone, a glyph by name); the interface decides the colors, the
//! symbols the user's glyph set can draw, the width it has and what to drop when it has too
//! little. A plugin that wants a slot empty again sends `set_view` with no view.
//!
//! A plugin is a separate program that may be wrong or hostile, and its text ends up in the user's
//! terminal, so nothing in a view is trusted until [`SetView::checked`] has cleaned it: control
//! characters (an escape sequence in a name would otherwise reach the terminal) and
//! direction-changing characters are removed, and size limits are applied. The host calls it on
//! every `set_view` it reads.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The notification that fills or empties a slot: `set_view`.
pub const METHOD_SET_VIEW: &str = "set_view";

/// Most spans a view may hold, in all its lines.
pub const MAX_SPANS: usize = 64;

/// Most characters a view may hold, in all its spans.
pub const MAX_CHARS: usize = 512;

/// A place in the interface a plugin can fill. New slots are added over time (never removed or
/// renamed), and a plugin names only the ones it knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    /// The status bar's left side, after the selected entry's details: one word about it.
    StatusEntry,
    /// The status bar's right side, before the position: one line about the folder.
    StatusRight,
    /// The header's right edge, after the pills the interface itself draws: one short line, drawn
    /// as a pill in the one tone of its first span.
    HeaderPill,
}

impl Slot {
    /// Most lines the slot has room for.
    pub fn line_limit(self) -> usize {
        match self {
            Self::StatusEntry | Self::StatusRight | Self::HeaderPill => 1,
        }
    }
}

/// How a span should look, in the interface's own palette.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    #[default]
    Normal,
    Accent,
    Danger,
    Dim,
}

/// A symbol named rather than spelled, because which symbols the terminal can draw is the user's
/// choice (`[ui] glyphs`) and the interface draws nothing at all where the set has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Glyph {
    Branch,
    Ahead,
    Behind,
    Staged,
    Modified,
    Untracked,
    Conflicted,
}

/// A run of text in one tone, optionally led by a glyph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub text: String,
    /// Drawn in front of `text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph: Option<Glyph>,
    /// Leave a space between the glyph and `text`, when the glyph is drawn: for a glyph that
    /// labels the text (a branch symbol) rather than prefixes a count (`+3`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub spaced: bool,
    #[serde(default)]
    pub tone: Tone,
}

fn is_false(value: &bool) -> bool {
    !value
}

impl Span {
    pub fn text(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            glyph: None,
            spaced: false,
            tone,
        }
    }

    /// A span led by `glyph`, with a space between them when `spaced`.
    pub fn with_glyph(glyph: Glyph, spaced: bool, text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            glyph: Some(glyph),
            spaced,
            tone,
        }
    }
}

/// Spans drawn in order, one space apart.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub spans: Vec<Span>,
}

/// What fills a slot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub lines: Vec<Line>,
}

impl View {
    /// A view of the one line made of `spans`.
    pub fn line(spans: Vec<Span>) -> Self {
        Self {
            lines: vec![Line { spans }],
        }
    }
}

/// The parameters of `set_view`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetView {
    pub slot: Slot,
    /// What now fills the slot; `null` or absent empties it.
    #[serde(default)]
    pub view: Option<View>,
}

/// Why a view was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewError {
    TooManyLines { slot: Slot, limit: usize },
    TooManySpans,
    TooMuchText,
}

impl fmt::Display for ViewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyLines { slot, limit } => {
                write!(f, "slot {slot:?} holds at most {limit} line(s)")
            }
            Self::TooManySpans => write!(f, "a view holds at most {MAX_SPANS} spans"),
            Self::TooMuchText => write!(f, "a view holds at most {MAX_CHARS} characters"),
        }
    }
}

impl std::error::Error for ViewError {}

/// Whether `c` can change what the terminal does or the order text reads in, rather than be text.
fn is_unsafe(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        )
}

impl SetView {
    /// This request with its text cleaned, or why it cannot be shown. Cleaning goes first, so the
    /// limits count what would be drawn.
    ///
    /// # Errors
    ///
    /// A view with more lines than the slot has room for, or more spans or characters than
    /// [`MAX_SPANS`] and [`MAX_CHARS`].
    pub fn checked(mut self) -> Result<Self, ViewError> {
        let Some(view) = self.view.as_mut() else {
            return Ok(self);
        };
        let limit = self.slot.line_limit();
        if view.lines.len() > limit {
            return Err(ViewError::TooManyLines {
                slot: self.slot,
                limit,
            });
        }
        let mut spans = 0;
        let mut chars = 0;
        for span in view.lines.iter_mut().flat_map(|line| line.spans.iter_mut()) {
            span.text.retain(|c| !is_unsafe(c));
            spans += 1;
            chars += span.text.chars().count();
        }
        if spans > MAX_SPANS {
            return Err(ViewError::TooManySpans);
        }
        if chars > MAX_CHARS {
            return Err(ViewError::TooMuchText);
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn one_line(text: &str) -> SetView {
        SetView {
            slot: Slot::StatusRight,
            view: Some(View::line(vec![Span::text(text, Tone::Accent)])),
        }
    }

    fn texts(set: &SetView) -> Vec<&str> {
        set.view
            .iter()
            .flat_map(|view| &view.lines)
            .flat_map(|line| &line.spans)
            .map(|span| span.text.as_str())
            .collect()
    }

    #[test]
    fn the_wire_shape_is_the_documented_one() {
        let set = SetView {
            slot: Slot::StatusRight,
            view: Some(View::line(vec![
                Span::with_glyph(Glyph::Branch, true, "main", Tone::Accent),
                Span::text("ok", Tone::Normal),
            ])),
        };
        let json = serde_json::to_string(&set).expect("plain data serializes");
        assert_eq!(
            json,
            r#"{"slot":"status_right","view":{"lines":[{"spans":[{"text":"main","glyph":"branch","spaced":true,"tone":"accent"},{"text":"ok","tone":"normal"}]}]}}"#
        );
    }

    #[test]
    fn a_missing_view_empties_the_slot_and_defaults_fill_the_rest() {
        let empty: SetView =
            serde_json::from_str(r#"{"slot":"status_entry"}"#).expect("a view is optional");
        assert_eq!(empty.view, None);
        let bare: SetView = serde_json::from_str(
            r#"{"slot":"status_entry","view":{"lines":[{"spans":[{"text":"x"}]}]}}"#,
        )
        .expect("tone, glyph and spacing are optional");
        let span = &bare.view.expect("sent").lines[0].spans[0];
        assert_eq!(
            (span.glyph, span.spaced, span.tone),
            (None, false, Tone::Normal)
        );
    }

    #[test]
    fn the_header_pill_slot_has_its_documented_name_and_one_line() {
        let set: SetView = serde_json::from_str(
            r#"{"slot":"header_pill","view":{"lines":[{"spans":[{"text":"up 5m"}]}]}}"#,
        )
        .expect("a known slot");
        assert_eq!(set.slot, Slot::HeaderPill);
        assert_eq!(Slot::HeaderPill.line_limit(), 1);
    }

    #[test]
    fn an_unknown_slot_is_a_parse_error_not_a_guess() {
        assert!(serde_json::from_str::<SetView>(r#"{"slot":"nowhere"}"#).is_err());
    }

    #[test]
    fn escape_sequences_and_direction_marks_are_removed() {
        let set = one_line("a\u{1b}]0;title\u{7}b\u{202e}c\u{2066}d\n").checked();
        assert_eq!(texts(&set.expect("cleaned")), ["a]0;titlebcd"]);
    }

    #[test]
    fn a_status_slot_refuses_a_second_line() {
        let mut set = one_line("x");
        set.view.as_mut().expect("set").lines.push(Line::default());
        assert_eq!(
            set.checked(),
            Err(ViewError::TooManyLines {
                slot: Slot::StatusRight,
                limit: 1
            })
        );
    }

    #[test]
    fn the_limits_count_what_would_be_drawn() {
        let long = "x".repeat(MAX_CHARS);
        assert!(one_line(&long).checked().is_ok());
        assert_eq!(
            one_line(&format!("{long}x")).checked(),
            Err(ViewError::TooMuchText)
        );
        // Control characters are removed before counting, so they cannot push a view over.
        assert!(one_line(&format!("{long}\u{1}\u{2}")).checked().is_ok());

        let spans = (0..=MAX_SPANS)
            .map(|_| Span::text("x", Tone::Normal))
            .collect();
        let many = SetView {
            slot: Slot::StatusRight,
            view: Some(View::line(spans)),
        };
        assert_eq!(many.checked(), Err(ViewError::TooManySpans));
    }

    proptest! {
        #[test]
        fn checked_text_never_holds_an_unsafe_character(text in any::<String>()) {
            if let Ok(set) = one_line(&text).checked() {
                prop_assert!(texts(&set).iter().all(|t| !t.chars().any(is_unsafe)));
            }
        }

        #[test]
        fn checking_twice_changes_nothing(text in any::<String>()) {
            if let Ok(once) = one_line(&text).checked() {
                prop_assert_eq!(once.clone().checked(), Ok(once));
            }
        }

        #[test]
        fn a_checked_view_stays_within_the_limits(texts_in in prop::collection::vec(any::<String>(), 0..100)) {
            let set = SetView {
                slot: Slot::StatusRight,
                view: Some(View::line(
                    texts_in.iter().map(|t| Span::text(t.clone(), Tone::Dim)).collect(),
                )),
            };
            if let Ok(set) = set.checked() {
                let all = texts(&set);
                prop_assert!(all.len() <= MAX_SPANS);
                prop_assert!(all.iter().map(|t| t.chars().count()).sum::<usize>() <= MAX_CHARS);
            }
        }

        #[test]
        fn a_view_survives_the_wire(text in "[^\\p{Cc}]{0,40}", spaced in any::<bool>()) {
            let set = SetView {
                slot: Slot::StatusEntry,
                view: Some(View::line(vec![Span::with_glyph(Glyph::Staged, spaced, text, Tone::Danger)])),
            };
            let json = serde_json::to_string(&set).expect("plain data serializes");
            prop_assert_eq!(serde_json::from_str::<SetView>(&json).expect("round trip"), set);
        }
    }
}
