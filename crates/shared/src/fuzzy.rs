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

//! The fuzzy matcher behind the finder and the search: how well a typed run of letters fits a name.
//! It lives here, and not with the browser, so the plugin that ranks the finder's results and the
//! browser's own search score names the same way without one depending on the other.

/// What to look for: a non-empty run of letters that must appear, in order, in an entry's name.
/// Non-empty by construction, because an empty query matches everything and would "find" the
/// first entry of the directory it started in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// As typed, for the small bonus a matching capital earns.
    typed: Vec<char>,
    lower: Vec<char>,
}

// Scoring. A name's score is the best way of laying the typed letters over it; higher is better.
// Only the differences matter, and the weights are tuned so that: a run of letters side by side
// beats scattered ones, a letter at the start of a name or of a word beats one in the middle, and
// any name holding the whole query as one piece beats every name that does not.
const MATCH: i32 = 16;
const ADJACENT: i32 = 12;
const WORD_START: i32 = 10;
const NAME_START: i32 = 8;
const SAME_CASE: i32 = 2;
const GAP: i32 = 1;
const LEADING_GAP: i32 = 1;
const LEADING_GAP_CAP: i32 = 6;
const WHOLE_PIECE: i32 = 1_000;
/// Stands for "cannot be matched this way"; far enough below any score to survive additions.
const NONE: i32 = i32::MIN / 4;

fn fold(c: char) -> char {
    // Nearly every name is ASCII, and the general path builds an iterator per character.
    if c.is_ascii() {
        c.to_ascii_lowercase()
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

fn is_separator(c: char) -> bool {
    matches!(c, '.' | '_' | '-' | ' ' | '/' | '(' | '[')
}

impl Query {
    /// `None` for an empty `text`.
    pub fn new(text: &str) -> Option<Self> {
        let typed: Vec<char> = text.chars().collect();
        if typed.is_empty() {
            return None;
        }
        let lower = typed.iter().copied().map(fold).collect();
        Some(Self { typed, lower })
    }

    /// Whether this query is `shorter` with more letters after it (or the same letters), ignoring
    /// case. Everything that matches this query then also matches `shorter`, because a name that
    /// holds `abc` in order holds `ab` in order. That lets a ranking narrow as letters are typed
    /// instead of starting over.
    pub fn extends(&self, shorter: &Query) -> bool {
        self.lower.starts_with(&shorter.lower)
    }

    pub fn matches(&self, name: &str) -> bool {
        self.score(name).is_some()
    }

    /// How well `name` fits, or `None` if it does not hold the letters in order. Capitals in the
    /// query count only as a small bonus, so `README` still finds `readme`.
    pub fn score(&self, name: &str) -> Option<i32> {
        // Most names in a big folder fail here, so it must not allocate.
        if !self.is_subsequence_of(name.chars().map(fold)) {
            return None;
        }
        let chars: Vec<char> = name.chars().collect();
        let lower: Vec<char> = chars.iter().copied().map(fold).collect();

        let m = self.lower.len();
        let n = lower.len();
        // `previous[j]`: the best score with the letters so far laid out and the last of them on
        // `name[j]`. Each row is filled left to right with a running best of the gap-discounted
        // scores before it, so a name costs `m * n`, not `m * n * n`.
        let mut previous = vec![NONE; n];
        for (i, &want) in self.lower.iter().enumerate() {
            let mut current = vec![NONE; n];
            let mut gapped = NONE;
            for j in 0..n {
                // `gapped` = best of `previous[k] - (j - 1 - k) * GAP` for `k <= j - 2`.
                if j >= 2 {
                    gapped = (gapped - GAP).max(previous[j - 2] - GAP);
                } else {
                    gapped = NONE;
                }
                if lower[j] != want {
                    continue;
                }
                let before = if i == 0 {
                    -(j as i32).min(LEADING_GAP_CAP) * LEADING_GAP
                } else {
                    let adjacent = if j >= 1 && previous[j - 1] > NONE {
                        previous[j - 1] + ADJACENT
                    } else {
                        NONE
                    };
                    adjacent.max(gapped)
                };
                if before <= NONE {
                    continue;
                }
                let mut here = MATCH + before;
                if j == 0 {
                    here += NAME_START;
                }
                if j == 0
                    || is_separator(chars[j - 1])
                    || (chars[j - 1].is_lowercase() && chars[j].is_uppercase())
                {
                    here += WORD_START;
                }
                if chars[j] == self.typed[i] {
                    here += SAME_CASE;
                }
                current[j] = here;
            }
            previous = current;
        }

        let best = previous.into_iter().max().filter(|&score| score > NONE)?;
        let whole = lower
            .windows(m)
            .any(|window| window == self.lower.as_slice());
        // Among equal fits the shorter name is the closer one.
        Some(best + if whole { WHOLE_PIECE } else { 0 } - n as i32)
    }

    fn is_subsequence_of(&self, folded: impl Iterator<Item = char>) -> bool {
        let mut wanted = self.lower.iter();
        let mut next = wanted.next();
        for c in folded {
            if Some(&c) == next {
                next = wanted.next();
            }
        }
        next.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_query_is_not_a_query() {
        assert_eq!(Query::new(""), None);
        assert!(Query::new("a").is_some());
    }

    #[test]
    fn matching_ignores_case() {
        let query = Query::new("AeReNd").unwrap();
        assert!(query.matches("aerend"));
        assert!(query.matches("My-AEREND-notes"));
        assert!(!query.matches("aeren"));
    }

    #[test]
    fn letters_must_appear_in_order_but_need_not_touch() {
        let query = Query::new("aernd").unwrap();
        assert!(query.matches("aerend"));
        assert!(query.matches("a-e-r-n-d.txt"));
        assert!(!query.matches("dnreaa"), "right letters, wrong order");
        assert!(!query.matches("aern"), "a letter is missing");
        assert!(
            !Query::new("aa").unwrap().matches("a"),
            "a letter is used once"
        );
    }

    fn score(query: &str, name: &str) -> i32 {
        Query::new(query)
            .unwrap()
            .score(name)
            .expect("should match")
    }

    #[test]
    fn a_tighter_fit_scores_higher() {
        assert!(
            score("aerend", "aerend") > score("aerend", "aerend.txt"),
            "shorter wins"
        );
        assert!(
            score("aer", "aerend") > score("aer", "a-e-r"),
            "side by side beats scattered"
        );
        assert!(score("rend", "aerend") > score("rend", "r_e_n_d"));
        assert!(
            score("ab", "ab_cd") > score("ab", "xab_cd"),
            "the start of a name beats the middle"
        );
    }

    #[test]
    fn a_word_start_beats_the_middle_of_a_word() {
        assert!(
            score("nt", "my-notes") > score("nt", "mynotes"),
            "after a separator"
        );
        assert!(
            score("mn", "myNotes") > score("mn", "mynotes"),
            "a capital after lowercase"
        );
    }

    #[test]
    fn a_whole_piece_beats_any_scattered_match() {
        // The scattered name has every bonus going for it; the whole-piece one still wins.
        assert!(score("test", "xx_test_xx") > score("test", "t-e-s-t"));
        assert!(score("note", "my-notebook") > score("note", "n_o_t_e"));
    }

    #[test]
    fn a_matching_capital_is_a_small_bonus_not_a_requirement() {
        assert!(score("README", "README") > score("README", "readme"));
        assert!(Query::new("README").unwrap().matches("readme"));
        assert!(Query::new("readme").unwrap().matches("README"));
    }

    #[test]
    fn multibyte_names_score_without_panicking() {
        assert!(Query::new("日語").unwrap().matches("日本語.txt"));
        assert!(Query::new("é").unwrap().matches("café"));
        assert!(!Query::new("z").unwrap().matches("日本語"));
    }
}
