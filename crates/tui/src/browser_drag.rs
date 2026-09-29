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

//! Drag and drop in the file browser: the press-move-release state machine, what a drop at a
//! given spot would do, the auto-scroll at a list's edges and the label that follows the pointer.
//!
//! Like `browser_mouse`, everything here is pure. `main` feeds it events and carries out the
//! result (a paste through `App`, text into a shell pane), and `draw` asks it the same question
//! the release will, so the highlight on screen is exactly the drop that would happen.

use std::path::{Path, PathBuf};

use ratatui::layout::{Position, Rect};
use shared::DirEntryInfo;

use crate::app::ClipboardMode;
use crate::browser_mouse::{Hit, Pane};

/// How far, in cells, the pointer must travel from where the button went down before a press
/// becomes a drag. Without it a click that wobbles by one cell would start moving files.
pub const START_DISTANCE: u16 = 2;

/// What is dragged and how it would land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Active {
    /// The entries being carried: the marked set when the pressed row was marked, otherwise just
    /// that row.
    pub sources: Vec<PathBuf>,
    pub pointer: Position,
    /// Move, or copy while `Ctrl` is held.
    pub mode: ClipboardMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Drag {
    #[default]
    Idle,
    /// The left button is down on a middle-column row and has not moved far enough to be a drag.
    Pressed {
        origin: Position,
        row: usize,
    },
    Active(Active),
}

fn mode_of(ctrl: bool) -> ClipboardMode {
    if ctrl {
        ClipboardMode::Copy
    } else {
        ClipboardMode::Move
    }
}

impl Drag {
    pub fn press(&mut self, row: usize, at: Position) {
        *self = Drag::Pressed { origin: at, row };
    }

    pub fn is_idle(&self) -> bool {
        matches!(self, Drag::Idle)
    }

    /// The drag in progress, once the pointer has moved far enough.
    pub fn active(&self) -> Option<&Active> {
        match self {
            Drag::Active(active) => Some(active),
            _ => None,
        }
    }

    /// Feeds pointer motion. A press turns into a drag once the pointer is
    /// [`START_DISTANCE`] cells from where it started; `sources_of` is asked for what the pressed
    /// row carries only then, so nothing is gathered for a plain click. A press that carries
    /// nothing goes back to idle.
    pub fn motion(
        &mut self,
        at: Position,
        ctrl: bool,
        sources_of: impl FnOnce(usize) -> Vec<PathBuf>,
    ) {
        match self {
            Drag::Idle => {}
            Drag::Pressed { origin, row } => {
                let travelled = origin.x.abs_diff(at.x) + origin.y.abs_diff(at.y);
                if travelled >= START_DISTANCE {
                    let sources = sources_of(*row);
                    *self = if sources.is_empty() {
                        Drag::Idle
                    } else {
                        Drag::Active(Active {
                            sources,
                            pointer: at,
                            mode: mode_of(ctrl),
                        })
                    };
                }
            }
            Drag::Active(active) => {
                active.pointer = at;
                active.mode = mode_of(ctrl);
            }
        }
    }

    /// Ends the gesture at the button's release. Gives back the drag when there was one; a press
    /// that never moved far enough was a click and returns `None`. `ctrl` is read here, at the
    /// drop, so `Ctrl` pressed only for the drop still copies.
    pub fn release(&mut self, at: Position, ctrl: bool) -> Option<Active> {
        match std::mem::take(self) {
            Drag::Active(mut active) => {
                active.pointer = at;
                active.mode = mode_of(ctrl);
                Some(active)
            }
            _ => None,
        }
    }

    /// Abandons the gesture; whether a drag was actually in progress.
    pub fn cancel(&mut self) -> bool {
        matches!(std::mem::take(self), Drag::Active(_))
    }
}

/// What a drop at some spot would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Move or copy `sources` into `dst`. `sources` is already reduced to what actually has to
    /// travel: an entry dropped on the folder it is in is left out.
    Folder { dst: PathBuf, sources: Vec<PathBuf> },
    /// Type the paths, quoted, into the mini-shell under the pointer.
    Shell,
    /// Nothing happens.
    Nothing,
}

/// What the browser is showing, for working out which folder a spot belongs to.
pub struct Scene<'a> {
    pub current_dir: &'a Path,
    pub current: &'a [DirEntryInfo],
    pub parent: &'a [DirEntryInfo],
}

/// What travels when `sources` are dropped into `dst`, or `None` when the drop is refused.
///
/// A drop is refused outright if any source contains `dst` (a folder into itself or its own
/// descendant), rather than quietly carrying on without that one. Sources already directly in
/// `dst` are dropped from the batch, since moving them there changes nothing and copying them
/// would only raise a conflict with themselves; if that leaves nothing, the drop is refused too.
pub fn landing(dst: &Path, sources: &[PathBuf]) -> Option<Vec<PathBuf>> {
    if sources.iter().any(|source| dst.starts_with(source)) {
        return None;
    }
    let travelling: Vec<PathBuf> = sources
        .iter()
        .filter(|source| source.parent() != Some(dst))
        .cloned()
        .collect();
    (!travelling.is_empty()).then_some(travelling)
}

fn folder_row(entries: &[DirEntryInfo], index: usize) -> Option<&Path> {
    entries
        .get(index)
        .filter(|entry| entry.is_dir)
        .map(|entry| entry.path.as_path())
}

/// What a drop of `sources` at `hit` would do. The shell box has priority over anything under
/// it. A folder row is a target, in either file column; blank space in the left column means the
/// directory above the browsed one ("up a level"); a file, blank space elsewhere and the preview
/// are nothing.
pub fn resolve(hit: Hit, over_shell: bool, scene: &Scene<'_>, sources: &[PathBuf]) -> Target {
    if over_shell {
        return Target::Shell;
    }
    let dst = match hit {
        Hit::CurrentRow(index) => folder_row(scene.current, index),
        Hit::ParentRow(index) => folder_row(scene.parent, index),
        Hit::Blank(Pane::Parent) => scene.current_dir.parent(),
        Hit::Blank(_) | Hit::Elsewhere => None,
    };
    match dst.and_then(|dst| landing(dst, sources).map(|sources| (dst, sources))) {
        Some((dst, sources)) => Target::Folder {
            dst: dst.to_path_buf(),
            sources,
        },
        None => Target::Nothing,
    }
}

/// Where to move the selection so the middle column scrolls one row while a drag hovers at its
/// top or bottom border, or `None` when the pointer is elsewhere or the list cannot scroll that
/// way. Aiming the selection at the row just outside the view is what scrolls it, since the list
/// keeps the selection on screen and nothing else moves the view. `pane` is the column's frame,
/// `list` the rows inside it.
pub fn edge_scroll(
    pointer: Position,
    pane: Rect,
    list: Rect,
    offset: usize,
    len: usize,
) -> Option<usize> {
    if list.height == 0 || pointer.x < pane.x || pointer.x >= pane.right() {
        return None;
    }
    let rows = usize::from(list.height);
    if pointer.y < list.y {
        offset.checked_sub(1)
    } else if pointer.y >= list.bottom() {
        (offset + rows < len).then_some(offset + rows)
    } else {
        None
    }
}

fn batch_label(sources: &[PathBuf]) -> String {
    match sources {
        [only] => only
            .file_name()
            .map_or_else(|| "item".into(), |n| n.to_string_lossy().into_owned()),
        many => format!("{} items", many.len()),
    }
}

/// The label that follows the pointer while dragging: what is carried, and where it would go.
pub fn ghost_text(active: &Active, target: &Target) -> String {
    let verb = match active.mode {
        ClipboardMode::Copy => "copy",
        ClipboardMode::Move => "move",
    };
    let label = batch_label(&active.sources);
    match target {
        Target::Folder { dst, .. } => {
            let name = dst
                .file_name()
                .map_or_else(|| "/".into(), |n| n.to_string_lossy().into_owned());
            format!("{verb} {label} to {name}/")
        }
        Target::Shell => format!("paste {label} into the shell"),
        Target::Nothing => format!("{verb} {label} - drop on a folder"),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn at(x: u16, y: u16) -> Position {
        Position::new(x, y)
    }

    fn p(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    fn entry(path: &str, is_dir: bool) -> DirEntryInfo {
        DirEntryInfo {
            name: path.rsplit('/').next().unwrap_or(path).into(),
            path: p(path),
            is_dir,
            size: 0,
            modified: None,
            mode: None,
        }
    }

    #[test]
    fn a_press_becomes_a_drag_only_past_the_start_distance() {
        let mut drag = Drag::Idle;
        drag.press(3, at(10, 5));
        drag.motion(at(11, 5), false, |_| panic!("gathered for a wobble"));
        assert!(drag.active().is_none());
        drag.motion(at(11, 6), false, |row| vec![p(&format!("/r/{row}"))]);
        let active = drag.active().expect("two cells away is a drag");
        assert_eq!(active.sources, [p("/r/3")]);
        assert_eq!(active.mode, ClipboardMode::Move);
    }

    #[test]
    fn a_click_that_never_moved_releases_as_nothing() {
        let mut drag = Drag::Idle;
        drag.press(0, at(4, 4));
        assert_eq!(drag.release(at(4, 4), false), None);
        assert!(drag.is_idle());
    }

    #[test]
    fn ctrl_at_the_drop_decides_copy_and_release_ends_the_gesture() {
        let mut drag = Drag::Idle;
        drag.press(0, at(4, 4));
        drag.motion(at(4, 9), false, |_| vec![p("/a")]);
        let dropped = drag.release(at(4, 12), true).expect("was dragging");
        assert_eq!(dropped.mode, ClipboardMode::Copy);
        assert_eq!(dropped.pointer, at(4, 12));
        assert!(drag.is_idle());
    }

    #[test]
    fn a_press_that_carries_nothing_goes_idle() {
        let mut drag = Drag::Idle;
        drag.press(0, at(0, 0));
        drag.motion(at(5, 5), false, |_| Vec::new());
        assert!(drag.is_idle());
    }

    #[test]
    fn cancel_reports_whether_a_drag_was_running() {
        let mut drag = Drag::Idle;
        assert!(!drag.cancel());
        drag.press(0, at(0, 0));
        assert!(!drag.cancel());
        drag.press(0, at(0, 0));
        drag.motion(at(0, 5), false, |_| vec![p("/a")]);
        assert!(drag.cancel());
        assert!(drag.is_idle());
    }

    #[test]
    fn landing_refuses_a_folder_into_itself_or_below_and_skips_same_folder_entries() {
        assert_eq!(landing(&p("/a/b"), &[p("/a/b")]), None);
        assert_eq!(landing(&p("/a/b/c"), &[p("/a/b")]), None);
        // A sibling whose name merely starts the same is not inside it.
        assert_eq!(landing(&p("/a/bc"), &[p("/a/b")]), Some(vec![p("/a/b")]));
        // Already in the folder: nothing to do, unless something else is coming too.
        assert_eq!(landing(&p("/a"), &[p("/a/x")]), None);
        assert_eq!(
            landing(&p("/a"), &[p("/a/x"), p("/z/y")]),
            Some(vec![p("/z/y")])
        );
        // One bad apple refuses the whole batch rather than being silently left behind.
        assert_eq!(landing(&p("/a/b"), &[p("/z/y"), p("/a/b")]), None);
    }

    fn scene<'a>(current: &'a [DirEntryInfo], parent: &'a [DirEntryInfo]) -> Scene<'a> {
        Scene {
            current_dir: Path::new("/r/cur"),
            current,
            parent,
        }
    }

    #[test]
    fn folder_rows_are_targets_in_both_columns_and_files_are_not() {
        let current = [entry("/r/cur/sub", true), entry("/r/cur/f.txt", false)];
        let parent = [entry("/r/other", true), entry("/r/cur", true)];
        let sources = [p("/r/cur/f.txt")];
        let s = scene(&current, &parent);

        let into = |hit| resolve(hit, false, &s, &sources);
        assert_eq!(
            into(Hit::CurrentRow(0)),
            Target::Folder {
                dst: p("/r/cur/sub"),
                sources: sources.to_vec()
            }
        );
        assert_eq!(into(Hit::CurrentRow(1)), Target::Nothing);
        assert_eq!(
            into(Hit::ParentRow(0)),
            Target::Folder {
                dst: p("/r/other"),
                sources: sources.to_vec()
            }
        );
        // The row for the folder being browsed is where the sources already are.
        assert_eq!(into(Hit::ParentRow(1)), Target::Nothing);
    }

    #[test]
    fn blank_space_in_the_left_column_means_up_a_level_and_elsewhere_means_nothing() {
        let current = [entry("/r/cur/f.txt", false)];
        let s = scene(&current, &[]);
        let sources = [p("/r/cur/f.txt")];
        assert_eq!(
            resolve(Hit::Blank(Pane::Parent), false, &s, &sources),
            Target::Folder {
                dst: p("/r"),
                sources: sources.to_vec()
            }
        );
        for hit in [
            Hit::Blank(Pane::Current),
            Hit::Blank(Pane::Preview),
            Hit::Elsewhere,
        ] {
            assert_eq!(resolve(hit, false, &s, &sources), Target::Nothing);
        }
    }

    #[test]
    fn the_shell_box_wins_over_whatever_is_under_it() {
        let current = [entry("/r/cur/sub", true)];
        let s = scene(&current, &[]);
        assert_eq!(
            resolve(Hit::CurrentRow(0), true, &s, &[p("/x")]),
            Target::Shell
        );
    }

    #[test]
    fn dropping_a_folder_on_itself_is_nothing() {
        let current = [entry("/r/cur/sub", true)];
        let s = scene(&current, &[]);
        assert_eq!(
            resolve(Hit::CurrentRow(0), false, &s, &[p("/r/cur/sub")]),
            Target::Nothing
        );
    }

    #[test]
    fn edge_scroll_only_at_the_borders_and_only_when_the_list_can_move() {
        let pane = Rect::new(20, 1, 40, 12);
        let list = Rect::new(21, 2, 38, 10);
        // Top border, scrolled down by 4: aim one row above the view.
        assert_eq!(edge_scroll(at(30, 1), pane, list, 4, 100), Some(3));
        // Already at the top: nothing further to reveal.
        assert_eq!(edge_scroll(at(30, 1), pane, list, 0, 100), None);
        // Bottom border: aim at the first row past the view.
        assert_eq!(edge_scroll(at(30, 12), pane, list, 4, 100), Some(14));
        assert_eq!(edge_scroll(at(30, 12), pane, list, 90, 100), None);
        // Inside the rows, or in another column: no scrolling.
        assert_eq!(edge_scroll(at(30, 6), pane, list, 4, 100), None);
        assert_eq!(edge_scroll(at(5, 1), pane, list, 4, 100), None);
    }

    #[test]
    fn ghost_names_the_verb_what_is_carried_and_where_it_lands() {
        let one = Active {
            sources: vec![p("/r/a.txt")],
            pointer: at(0, 0),
            mode: ClipboardMode::Move,
        };
        let many = Active {
            sources: vec![p("/r/a"), p("/r/b"), p("/r/c")],
            pointer: at(0, 0),
            mode: ClipboardMode::Copy,
        };
        let folder = Target::Folder {
            dst: p("/r/sub"),
            sources: vec![],
        };
        assert_eq!(ghost_text(&one, &folder), "move a.txt to sub/");
        assert_eq!(ghost_text(&many, &folder), "copy 3 items to sub/");
        assert_eq!(
            ghost_text(&many, &Target::Shell),
            "paste 3 items into the shell"
        );
        assert!(ghost_text(&one, &Target::Nothing).contains("drop on a folder"));
    }

    proptest! {
        /// Whatever is dropped where, a folder target is never one of the sources or inside one,
        /// and nothing already in it is asked to travel.
        #[test]
        fn a_folder_target_never_swallows_or_repeats_its_sources(
            names in proptest::collection::vec("[abc]{1,2}", 1..5),
            deep in proptest::collection::vec("[abc]{1,2}", 0..3),
            dst_depth in 0usize..3,
        ) {
            let sources: Vec<PathBuf> = names.iter().map(|n| p(&format!("/r/{n}"))).collect();
            let mut dst = p("/r");
            for part in deep.iter().take(dst_depth) {
                dst.push(part);
            }
            match landing(&dst, &sources) {
                None => {}
                Some(travelling) => {
                    prop_assert!(!travelling.is_empty());
                    for source in &travelling {
                        prop_assert!(!dst.starts_with(source), "{dst:?} is inside {source:?}");
                        prop_assert!(source.parent() != Some(dst.as_path()));
                        prop_assert!(sources.contains(source));
                    }
                    for source in &sources {
                        prop_assert!(!dst.starts_with(source));
                    }
                }
            }
        }

        /// The auto-scroll target is always a real row, and only ever one row outside the view.
        #[test]
        fn edge_scroll_stays_inside_the_list_and_moves_one_row(
            y in 0u16..30,
            offset in 0usize..200,
            len in 0usize..200,
            height in 1u16..20,
        ) {
            let pane = Rect::new(0, 1, 40, height + 2);
            let list = Rect::new(1, 2, 38, height);
            if let Some(target) = edge_scroll(at(10, y), pane, list, offset, len) {
                prop_assert!(target < len.max(offset), "{target} outside {len} rows");
                prop_assert!(
                    target + 1 == offset || target == offset + usize::from(height),
                    "{target} is not one row outside {offset}..+{height}"
                );
            }
        }
    }
}
