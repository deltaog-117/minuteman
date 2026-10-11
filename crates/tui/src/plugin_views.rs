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

//! The slots of the interface that plugins fill (`set_view`), and how a view is turned into what
//! the interface draws. Plugins describe text, a tone and glyphs by name; this is where those
//! meet the user's glyph set and become cells. A slot has one owner at a time (the plugin that
//! filled it last), and only its owner can empty it, so a second plugin cannot blank another's
//! segment.

use std::collections::HashMap;

use plugin_host::protocol::{Glyph, Tone, View};
use plugin_host::{SetView, Slot};

use crate::glyphs::Glyphs;

#[derive(Default)]
pub struct Slots {
    filled: HashMap<Slot, (String, View)>,
}

impl Slots {
    /// Applies what `plugin` asked for.
    pub fn apply(&mut self, plugin: &str, set: SetView) {
        match set.view {
            Some(view) => {
                self.filled.insert(set.slot, (plugin.to_string(), view));
            }
            None => {
                if self
                    .filled
                    .get(&set.slot)
                    .is_some_and(|(owner, _)| owner == plugin)
                {
                    self.filled.remove(&set.slot);
                }
            }
        }
    }

    pub fn view(&self, slot: Slot) -> Option<&View> {
        self.filled.get(&slot).map(|(_, view)| view)
    }

    /// Forgets what plugins no longer running had filled: a segment nobody can update any more
    /// would otherwise stay on screen, wrong, until the session ends.
    pub fn retain_running(&mut self, running: impl Fn(&str) -> bool) {
        self.filled.retain(|_, (owner, _)| running(owner));
    }
}

fn symbol(glyph: Glyph, g: &Glyphs) -> &'static str {
    match glyph {
        Glyph::Branch => g.branch,
        Glyph::Ahead => g.ahead,
        Glyph::Behind => g.behind,
        Glyph::Staged => g.staged,
        Glyph::Modified => g.modified,
        Glyph::Untracked => g.untracked,
        Glyph::Conflicted => g.conflicted,
    }
}

/// The first line of `view` as one block of text, in the user's glyph set, and the tone of its
/// first span; `None` when there is nothing to draw. A status segment is one block of one
/// color, so the tones of later spans do not show there.
pub fn line_text(view: &View, g: &Glyphs) -> Option<(String, Tone)> {
    let line = view.lines.first()?;
    let mut tone = None;
    let mut parts = Vec::new();
    for span in &line.spans {
        let lead = span.glyph.map_or("", |glyph| symbol(glyph, g));
        let text = match (lead.is_empty(), span.spaced) {
            (true, _) => span.text.clone(),
            (false, true) => format!("{lead} {}", span.text),
            (false, false) => format!("{lead}{}", span.text),
        };
        if !text.is_empty() {
            tone.get_or_insert(span.tone);
            parts.push(text);
        }
    }
    (!parts.is_empty()).then(|| (parts.join(" "), tone.unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::Glyphs;
    use plugin_host::protocol::Span;
    use theming::GlyphSet;

    fn set(slot: Slot, text: Option<&str>) -> SetView {
        SetView {
            slot,
            view: text.map(|t| View::line(vec![Span::text(t, Tone::Accent)])),
        }
    }

    #[test]
    fn the_last_plugin_to_fill_a_slot_owns_it_and_only_the_owner_can_empty_it() {
        let mut slots = Slots::default();
        slots.apply("a", set(Slot::StatusRight, Some("one")));
        slots.apply("b", set(Slot::StatusRight, None));
        assert!(slots.view(Slot::StatusRight).is_some(), "b does not own it");
        slots.apply("b", set(Slot::StatusRight, Some("two")));
        slots.apply("a", set(Slot::StatusRight, None));
        assert!(slots.view(Slot::StatusRight).is_some(), "a lost it to b");
        slots.apply("b", set(Slot::StatusRight, None));
        assert!(slots.view(Slot::StatusRight).is_none());
    }

    #[test]
    fn a_plugin_that_stopped_leaves_no_stale_segment() {
        let mut slots = Slots::default();
        slots.apply("git", set(Slot::StatusRight, Some("main")));
        slots.apply("other", set(Slot::StatusEntry, Some("x")));
        slots.retain_running(|name| name == "other");
        assert!(slots.view(Slot::StatusRight).is_none());
        assert!(slots.view(Slot::StatusEntry).is_some());
    }

    #[test]
    fn glyphs_come_from_the_users_set_and_vanish_where_the_set_has_none() {
        let view = View::line(vec![
            Span::with_glyph(Glyph::Branch, true, "main", Tone::Accent),
            Span::with_glyph(Glyph::Staged, false, "3", Tone::Dim),
        ]);
        let unicode = Glyphs::for_set(GlyphSet::Unicode);
        assert_eq!(
            line_text(&view, unicode),
            Some((
                format!("{} main {}3", unicode.branch, unicode.staged),
                Tone::Accent
            ))
        );
        let ascii = Glyphs::for_set(GlyphSet::Ascii);
        // The ASCII set has no branch symbol, so the label is just the name, as before.
        assert_eq!(
            line_text(&view, ascii).map(|(t, _)| t),
            Some("main +3".to_string())
        );
    }

    #[test]
    fn an_empty_view_draws_nothing() {
        let g = Glyphs::for_set(GlyphSet::Unicode);
        assert_eq!(line_text(&View::default(), g), None);
        assert_eq!(
            line_text(&View::line(vec![Span::text("", Tone::Normal)]), g),
            None
        );
    }
}
