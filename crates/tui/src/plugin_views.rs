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

use plugin_host::protocol::{Glyph, Line, Tone, View};
use plugin_host::{SetView, Slot};

use crate::glyphs::Glyphs;

#[derive(Default)]
pub struct Slots {
    filled: HashMap<Slot, (String, View)>,
    /// Entry name to its line in the `EntryBadge` view, rebuilt whenever that slot changes: every
    /// visible row asks for its badge on every frame, so the lookup must not scan the view.
    badge_rows: HashMap<String, usize>,
}

impl Slots {
    /// Applies what `plugin` asked for.
    pub fn apply(&mut self, plugin: &str, set: SetView) {
        let slot = set.slot;
        match set.view {
            Some(view) => {
                self.filled.insert(slot, (plugin.to_string(), view));
            }
            None => {
                if self
                    .filled
                    .get(&slot)
                    .is_some_and(|(owner, _)| owner == plugin)
                {
                    self.filled.remove(&slot);
                }
            }
        }
        if slot == Slot::EntryBadge {
            self.reindex_badges();
        }
    }

    /// The badge for the entry called `name` in the current folder, if the plugin that fills the
    /// `EntryBadge` slot gave it one. A name given twice has the later line.
    pub fn badge(&self, name: &str) -> Option<&Line> {
        let row = *self.badge_rows.get(name)?;
        self.filled.get(&Slot::EntryBadge)?.1.lines.get(row)
    }

    pub fn has_badges(&self) -> bool {
        !self.badge_rows.is_empty()
    }

    fn reindex_badges(&mut self) {
        self.badge_rows.clear();
        if let Some((_, view)) = self.filled.get(&Slot::EntryBadge) {
            for (row, line) in view.lines.iter().enumerate() {
                if let Some(key) = &line.key {
                    self.badge_rows.insert(key.clone(), row);
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
        self.reindex_badges();
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
    spans_text(view.lines.first()?, g)
}

/// `line` as one block of text, in the user's glyph set, and the tone of its first span that
/// draws anything.
pub fn spans_text(line: &Line, g: &Glyphs) -> Option<(String, Tone)> {
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

    fn badge_set(plugin_lines: &[(&str, &str)]) -> SetView {
        SetView {
            slot: Slot::EntryBadge,
            view: Some(View {
                lines: plugin_lines
                    .iter()
                    .map(|(key, text)| Line::badge(*key, Span::text(*text, Tone::Accent)))
                    .collect(),
            }),
        }
    }

    #[test]
    fn a_badge_is_found_by_entry_name_and_follows_the_slot() {
        let mut slots = Slots::default();
        assert!(slots.badge("a.txt").is_none());
        slots.apply(
            "git",
            badge_set(&[("a.txt", "M"), ("b.txt", "?"), ("a.txt", "S")]),
        );
        let text = |slots: &Slots, name| {
            slots
                .badge(name)
                .and_then(|line| line.spans.first())
                .map(|span| span.text.clone())
        };
        assert_eq!(
            text(&slots, "a.txt").as_deref(),
            Some("S"),
            "the later line wins"
        );
        assert_eq!(text(&slots, "b.txt").as_deref(), Some("?"));
        assert_eq!(text(&slots, "c.txt"), None);
        slots.apply(
            "other",
            SetView {
                slot: Slot::EntryBadge,
                view: None,
            },
        );
        assert!(
            slots.badge("b.txt").is_some(),
            "only the owner can empty it"
        );
        slots.apply("git", badge_set(&[("c.txt", "M")]));
        assert!(
            slots.badge("b.txt").is_none(),
            "a new view replaces the old one"
        );
        slots.apply(
            "git",
            SetView {
                slot: Slot::EntryBadge,
                view: None,
            },
        );
        assert!(slots.badge("c.txt").is_none());
    }

    #[test]
    fn a_plugin_that_stopped_leaves_no_stale_badge() {
        let mut slots = Slots::default();
        slots.apply("git", badge_set(&[("a.txt", "M")]));
        slots.retain_running(|_| false);
        assert!(slots.badge("a.txt").is_none());
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
