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

//! What is typed into the search, turned into something that can be checked against an entry.
//!
//! A query is words separated by whitespace, and an entry matches when it satisfies every one
//! of them. A word is one of:
//!
//! | Word | Matches |
//! |---|---|
//! | `src/main` | entries whose path holds that text (lower case ignores case, an upper case letter does not) |
//! | `*.rs`, `?.md`, `[ab]*` | entries whose name fits the glob; with a `/` in it, the whole path |
//! | `re:^src/.*\.rs$` | entries whose path a regular expression finds (case sensitive; `(?i)` turns it off) |
//! | `size:>1M`, `size:1k..2M` | files by size, with `k`, `M`, `G`, `T` as powers of 1024 |
//! | `mtime:<7d`, `mtime:>1w` | entries changed less (`<`) or more (`>`) than that long ago: `s m h d w` |
//! | `type:f`, `type:d` | files only, or folders only |
//!
//! A word that cannot be read (`size:>` while it is still being typed, `re:(`) is left out of the
//! search and handed back in `Parsed::ignored`, so the caller can say so; the rest of the query
//! keeps working.

use plugin_api::TreeEntry;
use regex::{Regex, RegexBuilder};

/// The same bound for a number of bytes or of seconds: both ends inclusive, `min <= max` always.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    min: u64,
    max: u64,
}

impl Span {
    /// `None` when `min > max`, which no value could satisfy.
    pub fn new(min: u64, max: u64) -> Option<Self> {
        (min <= max).then_some(Self { min, max })
    }

    pub fn contains(self, value: u64) -> bool {
        (self.min..=self.max).contains(&value)
    }
}

/// Which part of an entry a text word is looked for in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// The entry's own name.
    Name,
    /// The path below the folder the search began at.
    Path,
}

/// A text word, compiled once when the query is parsed so that checking an entry costs only the
/// match itself.
#[derive(Debug, Clone)]
pub struct Pattern {
    regex: Regex,
    target: Target,
}

impl Pattern {
    fn is_match(&self, entry: &TreeEntry) -> bool {
        match self.target {
            Target::Name => self.regex.is_match(entry.name()),
            Target::Path => self.regex.is_match(&entry.rel),
        }
    }
}

/// What kind of entry a `type:` word allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
}

/// One condition of a search.
#[derive(Debug, Clone)]
enum Term {
    Text(Pattern),
    /// A file whose size is in the span; a folder has no size worth comparing.
    Size(Span),
    /// An entry changed so many seconds ago, the age counted from the moment the query was typed.
    Age(Span),
    Kind(Kind),
}

/// A query that has been read: every term has to hold.
#[derive(Debug, Clone, Default)]
pub struct Search {
    terms: Vec<Term>,
}

/// A query read, with the words it could not use.
#[derive(Debug, Clone, Default)]
pub struct Parsed {
    pub search: Search,
    /// The words left out, in the order typed.
    pub ignored: Vec<String>,
}

impl Search {
    /// Reads `text`. Never fails: a word that makes no sense is reported in `Parsed::ignored`.
    pub fn parse(text: &str) -> Parsed {
        let mut parsed = Parsed::default();
        for word in text.split_whitespace() {
            match term_of(word) {
                Some(term) => parsed.search.terms.push(term),
                None => parsed.ignored.push(word.to_string()),
            }
        }
        parsed
    }

    /// Whether the query holds no condition, which every entry then satisfies.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Whether `entry` satisfies every term. `now` is the time as seconds since the epoch, which
    /// ages are counted back from; an entry that claims to be from the future is zero seconds old.
    pub fn matches(&self, entry: &TreeEntry, now: u64) -> bool {
        self.terms.iter().all(|term| match term {
            Term::Text(pattern) => pattern.is_match(entry),
            Term::Size(span) => !entry.is_dir && entry.size.is_some_and(|size| span.contains(size)),
            Term::Age(span) => entry
                .modified
                .is_some_and(|modified| span.contains(now.saturating_sub(modified))),
            Term::Kind(kind) => (*kind == Kind::Dir) == entry.is_dir,
        })
    }
}

fn term_of(word: &str) -> Option<Term> {
    if let Some(source) = word.strip_prefix("re:") {
        return regex_pattern(source).map(Term::Text);
    }
    if let Some(expression) = word.strip_prefix("size:") {
        return span_of(expression, parse_size).map(Term::Size);
    }
    if let Some(expression) = word.strip_prefix("mtime:") {
        return span_of(expression, parse_age).map(Term::Age);
    }
    if let Some(kind) = word.strip_prefix("type:") {
        return match kind {
            "f" | "file" => Some(Term::Kind(Kind::File)),
            "d" | "dir" => Some(Term::Kind(Kind::Dir)),
            _ => None,
        };
    }
    // Anything else, a colon included, is part of a path: `notes:2024` names a file as well as
    // any other text does.
    Some(Term::Text(match is_glob(word) {
        true => glob_pattern(word)?,
        false => literal_pattern(word)?,
    }))
}

fn is_glob(word: &str) -> bool {
    word.contains(['*', '?', '['])
}

/// A word with no capital letter is matched without regard to case, so `readme` finds `README.md`
/// and `README` finds only that.
fn smart_case(word: &str) -> bool {
    !word.chars().any(char::is_uppercase)
}

fn compiled(source: &str, ignore_case: bool, target: Target) -> Option<Pattern> {
    RegexBuilder::new(source)
        .case_insensitive(ignore_case)
        .build()
        .ok()
        .map(|regex| Pattern { regex, target })
}

fn regex_pattern(source: &str) -> Option<Pattern> {
    // An empty expression is a word still being typed, not a request for everything.
    (!source.is_empty())
        .then(|| compiled(source, false, Target::Path))
        .flatten()
}

fn literal_pattern(word: &str) -> Option<Pattern> {
    compiled(&regex::escape(word), smart_case(word), Target::Path)
}

fn glob_pattern(word: &str) -> Option<Pattern> {
    let target = match word.contains('/') {
        true => Target::Path,
        false => Target::Name,
    };
    compiled(&glob_to_regex(word), smart_case(word), target)
}

/// The regular expression for a glob, whole-name (or whole-path) anchored. `*` stops at a `/`,
/// `**` does not, `?` is one character that is not a `/`, and `[abc]` / `[!abc]` are classes.
fn glob_to_regex(glob: &str) -> String {
    let chars: Vec<char> = glob.chars().collect();
    let mut out = String::from("^");
    let mut at = 0;
    while at < chars.len() {
        match chars[at] {
            '*' if chars.get(at + 1) == Some(&'*') => {
                out.push_str(".*");
                at += 1;
            }
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            '[' => match class_end(&chars, at) {
                Some(end) => {
                    out.push_str(&class_of(&chars[at + 1..end]));
                    at = end;
                }
                // No closing bracket: a bracket is only a bracket.
                None => out.push_str("\\["),
            },
            other => out.push_str(&regex::escape(&other.to_string())),
        }
        at += 1;
    }
    out.push('$');
    out
}

/// Where the `]` closing the class opened at `open` is, if there is one. A `]` straight after the
/// opening (or after `!`) is a member of the class, as in the shell.
fn class_end(chars: &[char], open: usize) -> Option<usize> {
    let mut at = open + 1;
    if chars.get(at) == Some(&'!') {
        at += 1;
    }
    if chars.get(at) == Some(&']') {
        at += 1;
    }
    chars[at.min(chars.len())..]
        .iter()
        .position(|&c| c == ']')
        .map(|offset| at + offset)
}

fn class_of(inside: &[char]) -> String {
    let (negated, members) = match inside.split_first() {
        Some(('!', rest)) => (true, rest),
        _ => (false, inside),
    };
    let mut out = String::from(if negated { "[^" } else { "[" });
    for &member in members {
        match member {
            // A range's dash stays one; anything the regex syntax would read as structure is
            // escaped, which for a member is also what the shell means.
            '-' => out.push('-'),
            '\\' | '[' | ']' | '^' | '&' | '~' => {
                out.push('\\');
                out.push(member);
            }
            other => out.push(other),
        }
    }
    out.push(']');
    out
}

/// A comparison (`>5`, `<=5`, `=5`) or a range (`2..5`, `2..`, `..5`) over numbers that
/// `number` reads, as an inclusive span.
fn span_of(expression: &str, number: impl Fn(&str) -> Option<u64>) -> Option<Span> {
    if let Some((low, high)) = expression.split_once("..") {
        let low = match low.is_empty() {
            true => 0,
            false => number(low)?,
        };
        let high = match high.is_empty() {
            true => u64::MAX,
            false => number(high)?,
        };
        return Span::new(low, high);
    }
    let (operator, rest) = ["<=", ">=", "<", ">", "="]
        .iter()
        .find_map(|operator| Some((*operator, expression.strip_prefix(operator)?)))?;
    let value = number(rest)?;
    match operator {
        "<=" => Span::new(0, value),
        ">=" => Span::new(value, u64::MAX),
        "<" => Span::new(0, value.checked_sub(1)?),
        ">" => Span::new(value.checked_add(1)?, u64::MAX),
        _ => Span::new(value, value),
    }
}

/// Digits, then an optional unit: nothing or `b` for bytes, `k`, `m`, `g`, `t` for powers of 1024
/// (any case, and `kb`, `kib` as well). `None` for anything else or a number too big to hold.
pub fn parse_size(text: &str) -> Option<u64> {
    let (digits, unit) = split_number(text)?;
    let power = match unit.to_ascii_lowercase().as_str() {
        "" | "b" => 0,
        "k" | "kb" | "kib" => 1,
        "m" | "mb" | "mib" => 2,
        "g" | "gb" | "gib" => 3,
        "t" | "tb" | "tib" => 4,
        _ => return None,
    };
    digits.checked_mul(1024u64.checked_pow(power)?)
}

/// Digits and a unit, which is required: `s`, `m`, `h`, `d` or `w`, in seconds.
pub fn parse_age(text: &str) -> Option<u64> {
    let (digits, unit) = split_number(text)?;
    let seconds = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        "w" => 604_800,
        _ => return None,
    };
    digits.checked_mul(seconds)
}

/// The leading run of digits as a number, and what is left after it.
fn split_number(text: &str) -> Option<(u64, &str)> {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (digits, unit) = text.split_at(end);
    Some((digits.parse().ok()?, unit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::path::PathBuf;

    const NOW: u64 = 100_000_000;

    fn file(rel: &str, size: u64, modified: u64) -> TreeEntry {
        TreeEntry {
            path: PathBuf::from("/root").join(rel),
            rel: rel.to_string(),
            is_dir: false,
            size: Some(size),
            modified: Some(modified),
        }
    }

    fn dir(rel: &str) -> TreeEntry {
        TreeEntry {
            is_dir: true,
            size: None,
            ..file(rel, 0, NOW)
        }
    }

    fn holds(query: &str, entry: &TreeEntry) -> bool {
        let parsed = Search::parse(query);
        assert!(
            parsed.ignored.is_empty(),
            "{query:?} left out {:?}",
            parsed.ignored
        );
        parsed.search.matches(entry, NOW)
    }

    #[test]
    fn plain_text_is_found_anywhere_in_the_path_and_lower_case_ignores_case() {
        let entry = file("src/ui/README.md", 10, NOW);
        assert!(holds("src/ui", &entry));
        assert!(holds("readme", &entry));
        assert!(holds("README", &entry));
        assert!(!holds("Readme", &entry), "a capital makes the word exact");
        assert!(!holds("main", &entry));
    }

    #[test]
    fn a_glob_without_a_slash_fits_the_name_and_a_star_stops_at_a_slash() {
        let entry = file("src/ui/main.rs", 10, NOW);
        assert!(holds("*.rs", &entry));
        assert!(holds("m???.rs", &entry));
        assert!(holds("[a-m]ain.rs", &entry));
        assert!(holds("[!x]ain.rs", &entry));
        assert!(!holds("*.md", &entry));
        assert!(holds("src/*/main.rs", &entry));
        assert!(!holds("src/*.rs", &entry), "`*` does not cross a folder");
        assert!(holds("src/**.rs", &entry), "`**` does");
    }

    #[test]
    fn a_glob_must_fit_the_whole_name() {
        let entry = file("a/xmain.rs", 10, NOW);
        assert!(!holds("main.*", &entry));
        assert!(holds("*main.*", &entry));
    }

    #[test]
    fn a_regex_is_searched_for_in_the_path_and_is_case_sensitive() {
        let entry = file("src/Main.rs", 10, NOW);
        assert!(holds(r"re:^src/M.*\.rs$", &entry));
        assert!(!holds("re:main", &entry));
        assert!(holds("re:(?i)main", &entry));
    }

    #[test]
    fn sizes_compare_files_and_never_folders() {
        let entry = file("a.bin", 2048, NOW);
        assert!(holds("size:>1k", &entry));
        assert!(holds("size:>=2k", &entry));
        assert!(!holds("size:>2k", &entry));
        assert!(holds("size:<3KB", &entry));
        assert!(holds("size:1k..4k", &entry));
        assert!(holds("size:..4k", &entry));
        assert!(holds("size:2k..", &entry));
        assert!(!holds("size:..1k", &entry));
        assert!(!holds("size:>=0", &dir("d")), "a folder has no size");
    }

    #[test]
    fn age_counts_back_from_now() {
        let day = 86_400;
        let fresh = file("f", 1, NOW - 3600);
        let old = file("o", 1, NOW - 40 * day);
        assert!(holds("mtime:<1d", &fresh));
        assert!(!holds("mtime:<1d", &old));
        assert!(holds("mtime:>1w", &old));
        assert!(!holds("mtime:>1w", &fresh));
        assert!(holds("mtime:1h..2h", &fresh));
        assert!(!holds("mtime:2h..3h", &fresh));
        let future = file("t", 1, NOW + 500);
        assert!(
            holds("mtime:<1s", &future),
            "the future is zero seconds old"
        );
    }

    #[test]
    fn an_entry_without_the_data_a_filter_needs_does_not_match_it() {
        let bare = TreeEntry {
            size: None,
            modified: None,
            ..file("x", 0, 0)
        };
        assert!(!holds("size:>=0", &bare));
        assert!(!holds("mtime:>=0s", &bare));
        assert!(holds("x", &bare));
    }

    #[test]
    fn type_separates_files_from_folders() {
        assert!(holds("type:f", &file("a", 1, NOW)));
        assert!(!holds("type:f", &dir("a")));
        assert!(holds("type:dir", &dir("a")));
        assert!(!holds("type:d", &file("a", 1, NOW)));
    }

    #[test]
    fn every_word_has_to_hold() {
        let entry = file("src/main.rs", 5000, NOW - 60);
        assert!(holds("*.rs size:>1k mtime:<1h type:f src", &entry));
        assert!(!holds("*.rs size:>1M", &entry));
    }

    #[test]
    fn a_word_that_cannot_be_read_is_set_aside_and_the_rest_still_works() {
        let parsed = Search::parse("*.rs size:> re:( mtime:5 type:x re: size:1k..0 ok");
        assert_eq!(
            parsed.ignored,
            ["size:>", "re:(", "mtime:5", "type:x", "re:", "size:1k..0"]
        );
        let entry = file("ok/main.rs", 1, NOW);
        assert!(parsed.search.matches(&entry, NOW));
        assert!(!parsed.search.matches(&file("main.rs", 1, NOW), NOW));
    }

    #[test]
    fn an_empty_query_holds_nothing_back() {
        let parsed = Search::parse("   ");
        assert!(parsed.search.is_empty());
        assert!(parsed.search.matches(&file("anything", 0, 0), NOW));
    }

    #[test]
    fn an_unknown_prefix_is_text_and_a_stray_bracket_is_a_bracket() {
        assert!(holds("notes:2024", &file("notes:2024.txt", 1, NOW)));
        assert!(holds("a[b", &file("a[b", 1, NOW)));
        assert!(holds("[]a]", &file("]", 1, NOW)));
    }

    #[test]
    fn sizes_and_ages_read_their_units() {
        assert_eq!(parse_size("10"), Some(10));
        assert_eq!(parse_size("1k"), Some(1024));
        assert_eq!(parse_size("2MiB"), Some(2 * 1024 * 1024));
        assert_eq!(parse_size("1t"), Some(1 << 40));
        assert_eq!(parse_size("1x"), None);
        assert_eq!(parse_size("k"), None);
        assert_eq!(parse_size("99999999999999999999"), None);
        assert_eq!(parse_size("18446744073709551615t"), None);
        assert_eq!(parse_age("90s"), Some(90));
        assert_eq!(parse_age("2w"), Some(1_209_600));
        assert_eq!(parse_age("5"), None, "an age needs its unit");
    }

    proptest! {
        /// Whatever is typed, reading it returns, and every word is accounted for.
        #[test]
        fn parsing_never_panics_and_loses_no_word(text in "\\PC{0,60}") {
            let parsed = Search::parse(&text);
            let words = text.split_whitespace().count();
            prop_assert_eq!(parsed.search.terms.len() + parsed.ignored.len(), words);
        }

        /// A path typed back as a word finds itself, so a word is never stricter than the text.
        #[test]
        fn a_path_matches_the_word_made_of_itself(rel in "[a-z0-9_./-]{1,30}") {
            let entry = file(&rel, 1, NOW);
            let parsed = Search::parse(&rel);
            prop_assert!(parsed.ignored.is_empty());
            prop_assert!(parsed.search.matches(&entry, NOW));
        }

        /// Adding a word can only remove matches.
        #[test]
        fn another_word_never_adds_a_match(
            rel in "[a-zA-Z0-9_./-]{1,20}",
            size in 0u64..1_000_000,
            age in 0u64..10_000_000,
            first in prop::sample::select(vec!["a", "*.rs", "size:>1k", "mtime:<1d", "type:f", "re:x"]),
            second in prop::sample::select(vec!["b", "*", "size:<100k", "mtime:>1h", "type:d", "re:."]),
        ) {
            let entry = file(&rel, size, NOW.saturating_sub(age));
            let alone = Search::parse(first).search.matches(&entry, NOW);
            let both = Search::parse(&format!("{first} {second}")).search.matches(&entry, NOW);
            prop_assert!(!both || alone);
        }

        /// The two ends of a span are honoured exactly, whatever the numbers.
        #[test]
        fn a_span_contains_its_ends_and_nothing_beyond(low in 0u64..1_000_000, extra in 0u64..1_000_000) {
            let high = low + extra;
            let span = span_of(&format!("{low}..{high}"), parse_size).unwrap();
            prop_assert!(span.contains(low) && span.contains(high));
            prop_assert!(high == u64::MAX || !span.contains(high + 1));
            prop_assert!(low == 0 || !span.contains(low - 1));
        }
    }
}
