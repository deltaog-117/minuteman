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

//! What an answer from git looks like in the two status-bar slots. Pure: the same repository and
//! cursor always make the same views, so the session can tell by comparing whether anything needs
//! sending.

use std::path::Path;

use plugin_api::{Glyph, Line, MAX_BADGES, SetView, Slot, Span, Tone, View};

use crate::status::{FileState, Head, Repo};

/// The right slot: `<branch> ↑2 ↓1 +3 ~2 ?1`, drawn in the user's glyph set. A count of zero is
/// left out, so a clean branch is just its name.
pub fn repository(repo: &Repo) -> SetView {
    let head = match &repo.head {
        Head::Branch(name) => name.clone(),
        Head::Detached(id) => format!("@{id}"),
    };
    let mut spans = vec![Span::with_glyph(Glyph::Branch, true, head, Tone::Accent)];
    if let Some((ahead, behind)) = repo.ahead_behind {
        for (glyph, count) in [(Glyph::Ahead, ahead), (Glyph::Behind, behind)] {
            if count > 0 {
                spans.push(Span::with_glyph(
                    glyph,
                    false,
                    count.to_string(),
                    Tone::Accent,
                ));
            }
        }
    }
    let counts = repo.counts;
    for (glyph, count) in [
        (Glyph::Staged, counts.staged),
        (Glyph::Modified, counts.modified),
        (Glyph::Untracked, counts.untracked),
        (Glyph::Conflicted, counts.conflicted),
    ] {
        if count > 0 {
            spans.push(Span::with_glyph(
                glyph,
                false,
                count.to_string(),
                Tone::Accent,
            ));
        }
    }
    SetView {
        slot: Slot::StatusRight,
        view: Some(View::line(spans)),
    }
}

/// The left slot: the word for the state of `cursor` in `repo`, empty for a clean entry, one
/// outside the repository, or no cursor at all. A conflict is drawn as a danger.
pub fn entry(repo: &Repo, cursor: Option<&Path>) -> SetView {
    let view = cursor.and_then(|path| repo.state_of(path)).map(|state| {
        let tone = match state {
            FileState::Conflicted => Tone::Danger,
            _ => Tone::Accent,
        };
        View::line(vec![Span::text(state.label(), tone)])
    });
    SetView {
        slot: Slot::StatusEntry,
        view,
    }
}

/// The badge slot: one mark beside each changed entry of `dir`, the glyph for its state. A change
/// both staged and made again since shows the modified mark, since the work tree still differs.
/// Entries past what the slot holds are left unmarked, nearest the top of the name order first.
pub fn badges(repo: &Repo, dir: &Path) -> SetView {
    let lines: Vec<Line> = repo
        .states_in(dir)
        .into_iter()
        .take(MAX_BADGES)
        .map(|(name, state)| {
            let (glyph, tone) = match state {
                FileState::Conflicted => (Glyph::Conflicted, Tone::Danger),
                FileState::Untracked => (Glyph::Untracked, Tone::Dim),
                FileState::Staged => (Glyph::Staged, Tone::Accent),
                FileState::Modified | FileState::StagedModified => (Glyph::Modified, Tone::Accent),
            };
            Line::badge(name, Span::with_glyph(glyph, false, "", tone))
        })
        .collect();
    SetView {
        slot: Slot::EntryBadge,
        view: (!lines.is_empty()).then_some(View { lines }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::{Counts, Parsed};
    use std::path::PathBuf;

    fn repo(head: Head, ahead_behind: Option<(u32, u32)>, counts: Counts) -> Repo {
        Repo::new(
            PathBuf::from("/r"),
            Parsed {
                head,
                ahead_behind,
                counts,
                entries: vec![
                    (PathBuf::from("a"), FileState::Modified),
                    (PathBuf::from("c"), FileState::Conflicted),
                ],
            },
        )
    }

    #[test]
    fn a_badge_is_keyed_by_name_and_only_for_entries_directly_in_the_folder() {
        let r = repo(Head::Branch("main".into()), None, Counts::default());
        let set = badges(&r, Path::new("/r"));
        let lines = &set.view.as_ref().expect("two changes").lines;
        let found: Vec<_> = lines
            .iter()
            .map(|line| (line.key.as_deref(), line.spans[0].glyph, line.spans[0].tone))
            .collect();
        assert_eq!(
            found,
            [
                (Some("a"), Some(Glyph::Modified), Tone::Accent),
                (Some("c"), Some(Glyph::Conflicted), Tone::Danger),
            ]
        );
        assert_eq!(badges(&r, Path::new("/r/elsewhere")).view, None);
        assert_eq!(set.slot, Slot::EntryBadge);
        assert!(
            set.clone().checked().is_ok(),
            "the host accepts what it sends"
        );
    }

    fn glyphs_and_text(set: &SetView) -> Vec<(Option<Glyph>, String)> {
        set.view.as_ref().map_or_else(Vec::new, |view| {
            view.lines[0]
                .spans
                .iter()
                .map(|span| (span.glyph, span.text.clone()))
                .collect()
        })
    }

    #[test]
    fn a_clean_branch_is_just_its_name() {
        let set = repository(&repo(Head::Branch("main".into()), None, Counts::default()));
        assert_eq!(set.slot, Slot::StatusRight);
        assert_eq!(
            glyphs_and_text(&set),
            [(Some(Glyph::Branch), "main".to_string())]
        );
    }

    #[test]
    fn distance_and_counts_follow_in_the_old_order_and_zeros_are_left_out() {
        let counts = Counts {
            staged: 3,
            modified: 2,
            untracked: 1,
            conflicted: 0,
        };
        let set = repository(&repo(Head::Branch("dev".into()), Some((2, 0)), counts));
        assert_eq!(
            glyphs_and_text(&set),
            [
                (Some(Glyph::Branch), "dev".to_string()),
                (Some(Glyph::Ahead), "2".to_string()),
                (Some(Glyph::Staged), "3".to_string()),
                (Some(Glyph::Modified), "2".to_string()),
                (Some(Glyph::Untracked), "1".to_string()),
            ]
        );
    }

    #[test]
    fn a_detached_head_shows_the_commit_after_an_at_sign() {
        let set = repository(&repo(
            Head::Detached("abc1234".into()),
            None,
            Counts::default(),
        ));
        assert_eq!(glyphs_and_text(&set)[0].1, "@abc1234");
    }

    #[test]
    fn the_entry_slot_names_the_state_and_marks_a_conflict_as_danger() {
        let r = repo(Head::Branch("m".into()), None, Counts::default());
        let tone_of = |path: &str| {
            entry(&r, Some(Path::new(path))).view.map(|view| {
                (
                    view.lines[0].spans[0].text.clone(),
                    view.lines[0].spans[0].tone,
                )
            })
        };
        assert_eq!(
            tone_of("/r/a"),
            Some(("modified".to_string(), Tone::Accent))
        );
        assert_eq!(
            tone_of("/r/c"),
            Some(("conflict".to_string(), Tone::Danger))
        );
        assert_eq!(tone_of("/r/clean"), None);
        assert_eq!(entry(&r, None).view, None);
    }
}
