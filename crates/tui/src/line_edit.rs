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

//! The text of a one-line prompt with a cursor in it: what `rename`, `create`, `/` search, the `:`
//! command bar and "Open with → Other…" edit. It understands the usual line-editing keys
//! (arrows, Home/End, Delete, and the Ctrl-A/E/B/F/D/W/U/K set from shells) so a typo in the
//! middle of a name no longer means deleting everything after it. Pure text and a position, so it
//! is tested without a terminal.

use std::fmt;
use std::ops::Deref;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LineEdit {
    text: String,
    /// A byte offset into `text`, always on a character boundary.
    cursor: usize,
}

impl LineEdit {
    /// A line holding `text`, with the cursor after it — where typing continues.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self { text, cursor }
    }

    #[cfg(test)]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The text with `glyph` drawn where the cursor is.
    pub fn render(&self, glyph: &str) -> String {
        let (before, after) = self.text.split_at(self.cursor);
        format!("{before}{glyph}{after}")
    }

    /// Applies one key press and returns whether the text changed (so a search can re-run only
    /// when it must). Keys it does not know change nothing.
    pub fn edit(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Left if ctrl || alt => self.word_left(),
            KeyCode::Right if ctrl || alt => self.word_right(),
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.len(),
            KeyCode::Backspace if alt => return self.delete_word_back(),
            KeyCode::Backspace => return self.backspace(),
            KeyCode::Delete => return self.delete(),
            // Ctrl alone is a shortcut; with Alt as well it is how some layouts type AltGr
            // characters, which must still arrive as text.
            KeyCode::Char(c) if ctrl && !alt => return self.control(c),
            KeyCode::Char('b') if alt && !ctrl => self.word_left(),
            KeyCode::Char('f') if alt && !ctrl => self.word_right(),
            KeyCode::Char(c) => {
                self.insert(c);
                return true;
            }
            _ => {}
        }
        false
    }

    fn control(&mut self, c: char) -> bool {
        match c.to_ascii_lowercase() {
            'a' => self.cursor = 0,
            'e' => self.cursor = self.text.len(),
            'b' => self.left(),
            'f' => self.right(),
            'd' => return self.delete(),
            'w' => return self.delete_word_back(),
            'u' => return self.delete_range(0, self.cursor),
            'k' => return self.delete_range(self.cursor, self.text.len()),
            _ => {}
        }
        false
    }

    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    fn backspace(&mut self) -> bool {
        match self.prev_boundary(self.cursor) {
            Some(start) => self.delete_range(start, self.cursor),
            None => false,
        }
    }

    fn delete(&mut self) -> bool {
        match self.next_boundary(self.cursor) {
            Some(end) => self.delete_range(self.cursor, end),
            None => false,
        }
    }

    fn delete_word_back(&mut self) -> bool {
        let start = self.word_start(self.cursor);
        self.delete_range(start, self.cursor)
    }

    fn delete_range(&mut self, start: usize, end: usize) -> bool {
        if start >= end {
            return false;
        }
        self.text.replace_range(start..end, "");
        self.cursor = start;
        true
    }

    fn left(&mut self) {
        if let Some(start) = self.prev_boundary(self.cursor) {
            self.cursor = start;
        }
    }

    fn right(&mut self) {
        if let Some(end) = self.next_boundary(self.cursor) {
            self.cursor = end;
        }
    }

    fn word_left(&mut self) {
        self.cursor = self.word_start(self.cursor);
    }

    fn word_right(&mut self) {
        let mut at = self.cursor;
        // Over the spaces, then over the word, as a shell's Alt-f does.
        while let Some(next) = self
            .next_boundary(at)
            .filter(|_| self.char_at(at) == Some(' '))
        {
            at = next;
        }
        while let Some(next) = self
            .next_boundary(at)
            .filter(|_| self.char_at(at) != Some(' '))
        {
            at = next;
        }
        self.cursor = at;
    }

    /// Where the word before `from` begins: back over spaces, then back over the word.
    fn word_start(&self, from: usize) -> usize {
        let mut at = from;
        while let Some(prev) = self.prev_boundary(at) {
            if self.char_at(prev) == Some(' ') {
                at = prev;
            } else {
                break;
            }
        }
        while let Some(prev) = self.prev_boundary(at) {
            if self.char_at(prev) != Some(' ') {
                at = prev;
            } else {
                break;
            }
        }
        at
    }

    fn char_at(&self, at: usize) -> Option<char> {
        self.text[at..].chars().next()
    }

    fn prev_boundary(&self, from: usize) -> Option<usize> {
        self.text[..from].char_indices().next_back().map(|(i, _)| i)
    }

    fn next_boundary(&self, from: usize) -> Option<usize> {
        self.text[from..]
            .chars()
            .next()
            .map(|c| from + c.len_utf8())
    }
}

/// The text alone, so existing reads of a prompt's buffer (`is_empty`, `trim`, `&*buffer`) keep
/// working.
impl Deref for LineEdit {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for LineEdit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn alt(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
    }

    fn typed(text: &str) -> LineEdit {
        let mut line = LineEdit::default();
        for c in text.chars() {
            line.edit(key(KeyCode::Char(c)));
        }
        line
    }

    #[test]
    fn typing_appends_and_the_cursor_follows() {
        let line = typed("abc");
        assert_eq!(line.as_str(), "abc");
        assert_eq!(line.render("|"), "abc|");
    }

    #[test]
    fn typing_in_the_middle_inserts_there() {
        let mut line = typed("helo");
        line.edit(key(KeyCode::Left));
        line.edit(key(KeyCode::Char('l')));
        assert_eq!(line.as_str(), "hello");
        assert_eq!(line.render("|"), "hell|o");
    }

    #[test]
    fn backspace_and_delete_remove_either_side_of_the_cursor() {
        let mut line = typed("abcd");
        line.edit(key(KeyCode::Left));
        line.edit(key(KeyCode::Left));
        assert!(line.edit(key(KeyCode::Backspace)));
        assert_eq!(line.as_str(), "acd");
        assert!(line.edit(key(KeyCode::Delete)));
        assert_eq!(line.as_str(), "ad");
        assert_eq!(line.render("|"), "a|d");
    }

    #[test]
    fn the_ends_are_safe_to_press_against() {
        let mut line = typed("a");
        assert!(!line.edit(key(KeyCode::Delete)), "nothing after the cursor");
        line.edit(key(KeyCode::Home));
        assert!(!line.edit(key(KeyCode::Backspace)), "nothing before it");
        line.edit(key(KeyCode::Left));
        assert_eq!(line.render("|"), "|a");
        line.edit(key(KeyCode::End));
        line.edit(key(KeyCode::Right));
        assert_eq!(line.render("|"), "a|");
        let mut empty = LineEdit::default();
        for code in [KeyCode::Left, KeyCode::Right, KeyCode::Home, KeyCode::End] {
            assert!(!empty.edit(key(code)));
        }
        assert!(!empty.edit(key(KeyCode::Backspace)) && !empty.edit(key(KeyCode::Delete)));
    }

    #[test]
    fn home_and_end_jump_to_the_edges() {
        let mut line = typed("hello");
        line.edit(key(KeyCode::Home));
        assert_eq!(line.render("|"), "|hello");
        line.edit(key(KeyCode::End));
        assert_eq!(line.render("|"), "hello|");
        line.edit(ctrl('a'));
        assert_eq!(line.render("|"), "|hello");
        line.edit(ctrl('e'));
        assert_eq!(line.render("|"), "hello|");
    }

    #[test]
    fn control_keys_edit_like_a_shell() {
        let mut line = typed("one two three");
        assert!(line.edit(ctrl('w')));
        assert_eq!(line.as_str(), "one two ");
        assert!(line.edit(ctrl('w')));
        assert_eq!(line.as_str(), "one ");

        let mut line = typed("one two");
        line.edit(ctrl('a'));
        line.edit(ctrl('f'));
        line.edit(ctrl('f'));
        line.edit(ctrl('b'));
        assert_eq!(line.render("|"), "o|ne two");
        assert!(line.edit(ctrl('k')));
        assert_eq!(line.as_str(), "o");

        let mut line = typed("one two");
        line.edit(ctrl('b'));
        line.edit(ctrl('b'));
        assert!(line.edit(ctrl('u')));
        assert_eq!(line.as_str(), "wo");
        assert!(line.edit(ctrl('d')));
        assert_eq!(line.as_str(), "o");
    }

    #[test]
    fn words_are_jumped_and_deleted_by_alt_and_ctrl_arrows() {
        let mut line = typed("alpha beta gamma");
        line.edit(alt('b'));
        assert_eq!(line.render("|"), "alpha beta |gamma");
        line.edit(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(line.render("|"), "alpha |beta gamma");
        line.edit(alt('f'));
        assert_eq!(line.render("|"), "alpha beta| gamma");
        line.edit(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(line.render("|"), "alpha beta gamma|");
        assert!(line.edit(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT)));
        assert_eq!(line.as_str(), "alpha beta ");
    }

    #[test]
    fn multibyte_characters_move_and_delete_whole() {
        let mut line = typed("aéb");
        line.edit(key(KeyCode::Left));
        assert!(line.edit(key(KeyCode::Backspace)));
        assert_eq!(line.as_str(), "ab");
        let mut line = typed("日本語");
        line.edit(key(KeyCode::Home));
        line.edit(key(KeyCode::Right));
        assert_eq!(line.render("|"), "日|本語");
        assert!(line.edit(key(KeyCode::Delete)));
        assert_eq!(line.as_str(), "日語");
    }

    #[test]
    fn unknown_control_keys_and_other_codes_change_nothing() {
        let mut line = typed("abc");
        assert!(!line.edit(ctrl('c')));
        assert!(!line.edit(ctrl('z')));
        assert!(!line.edit(key(KeyCode::F(5))));
        assert!(!line.edit(key(KeyCode::Tab)));
        assert_eq!(line.render("|"), "abc|");
    }

    #[test]
    fn a_ctrl_alt_character_is_still_text() {
        // How some layouts type AltGr characters.
        let mut line = LineEdit::default();
        let altgr = KeyEvent::new(
            KeyCode::Char('@'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert!(line.edit(altgr));
        assert_eq!(line.as_str(), "@");
    }

    #[test]
    fn a_line_reads_as_its_text() {
        let line = LineEdit::new("name.txt");
        assert_eq!(&*line, "name.txt");
        assert!(!line.is_empty());
        assert_eq!(line.trim(), "name.txt");
        assert_eq!(format!("{line}"), "name.txt");
        assert_eq!(
            line.render("|"),
            "name.txt|",
            "new starts with the cursor at the end"
        );
    }

    proptest::proptest! {
        /// Whatever keys are pressed, the cursor stays on a character boundary and inside the
        /// text, so nothing panics and rendering always works.
        #[test]
        fn any_key_sequence_keeps_the_cursor_valid(
            keys in proptest::collection::vec(0usize..16, 0..80),
            seed in "[a-zé日 ]{0,12}",
        ) {
            let mut line = LineEdit::new(seed);
            for k in keys {
                let event = match k {
                    0 => key(KeyCode::Left),
                    1 => key(KeyCode::Right),
                    2 => key(KeyCode::Home),
                    3 => key(KeyCode::End),
                    4 => key(KeyCode::Backspace),
                    5 => key(KeyCode::Delete),
                    6 => ctrl('w'),
                    7 => ctrl('u'),
                    8 => ctrl('k'),
                    9 => alt('b'),
                    10 => alt('f'),
                    11 => key(KeyCode::Char('é')),
                    12 => key(KeyCode::Char(' ')),
                    13 => key(KeyCode::Char('日')),
                    14 => KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL),
                    _ => KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL),
                };
                line.edit(event);
                proptest::prop_assert!(line.cursor <= line.text.len());
                proptest::prop_assert!(line.text.is_char_boundary(line.cursor));
            }
            let rendered = line.render("|");
            proptest::prop_assert_eq!(rendered.replacen('|', "", 1), line.as_str());
        }
    }
}
