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

//! A binary tree of shell panes tiling the frame, tmux-style — replaces the old single floating
//! `PopupShell` with `ShellPanes`, which can hold several. A pane's on-screen rect is always
//! derived from the tree (each `Split` node carries a direction and a ratio, recursively
//! partitioning its parent's rect between two children), so panes never overlap and there is
//! nothing to drag into place; the only mouse interactions are dragging a divider to change its
//! `ratio` and clicking a pane to focus it. Reordering panes in the tree is deliberately not
//! supported (see `ROADMAP.md`).
//!
//! The tree shape/geometry logic (`Node`/`Tree`, `split_rect`, `Divider`, `close_id`, `split_id`,
//! `focus_at`, `leaf_ids`, `leaf_rects`, `neighbor`) is generic over the leaf payload so it can be unit-tested without
//! spawning real shells; `ShellPanes` specializes it to `PopupShell` and owns everything that
//! actually touches a pty (spawning, resizing, rendering, reaping exited shells).
//!
//! `resize_focused` adds a keyboard-driven equivalent of dragging a divider: it walks up from the
//! focused leaf to the nearest ancestor `Split` whose axis matches the requested `NudgeDir`, then
//! nudges that split's ratio in the direction that grows or shrinks the focused pane — i3's
//! resize-mode semantics, without adding a way to reorder panes in the tree (still out of scope,
//! per the note above). It reports whether it actually found a divider to adjust, so `tui::main`
//! can fall back to resizing the box itself (see `shell_area`'s `size_adjust`) when there isn't
//! one — a lone pane, or a tree only ever split along the other axis.
//!
//! `toggle_focused_orientation` flips a split's direction in place (side-by-side <-> stacked)
//! without touching which panes are on which side or their ratio — still not pane reordering,
//! just how the same two panes are arranged.

use std::io;
use std::path::Path;

use ratatui::layout::Rect;
use shell_overlay::{ExitOutcome, PopupShell};
use theming::Config;

use crate::popup_shell::pty_size;

/// Which way a `Split` divides its rect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    /// Panes side by side, divided by a vertical line.
    Horizontal,
    /// Panes stacked, divided by a horizontal line.
    Vertical,
}

enum Node<T> {
    Leaf(T),
    Split {
        direction: SplitDirection,
        ratio: f32,
        first: Box<Tree<T>>,
        second: Box<Tree<T>>,
    },
}

struct Tree<T> {
    id: usize,
    node: Node<T>,
}

type ShellTree = Tree<PopupShell>;

/// Splits `area` according to `direction`/`ratio`, clamping so neither side ever goes below one
/// cell — a terminal too small to actually fit two panes just gets a degenerate but panic-free
/// split rather than an arithmetic overflow.
fn split_rect(area: Rect, direction: SplitDirection, ratio: f32) -> (Rect, Rect) {
    match direction {
        SplitDirection::Horizontal => {
            let max_first = area.width.saturating_sub(1).max(1);
            let first_width = (((area.width as f32) * ratio).round() as u16).clamp(1, max_first);
            let second_width = area.width.saturating_sub(first_width);
            (
                Rect::new(area.x, area.y, first_width, area.height),
                Rect::new(area.x + first_width, area.y, second_width, area.height),
            )
        }
        SplitDirection::Vertical => {
            let max_first = area.height.saturating_sub(1).max(1);
            let first_height = (((area.height as f32) * ratio).round() as u16).clamp(1, max_first);
            let second_height = area.height.saturating_sub(first_height);
            (
                Rect::new(area.x, area.y, area.width, first_height),
                Rect::new(area.x, area.y + first_height, area.width, second_height),
            )
        }
    }
}

/// A `Split` node's shared border between its two children, as a mouse can hit-test and drag it.
#[derive(Debug, Clone, Copy)]
pub struct Divider {
    id: usize,
    direction: SplitDirection,
    area: Rect,
    ratio: f32,
}

impl Divider {
    /// The single screen column (`Horizontal`) or row (`Vertical`) the divider is drawn on —
    /// derived from the same `split_rect` math that lays out the panes themselves, so the hit
    /// zone can never drift from what's actually rendered.
    fn line(&self) -> u16 {
        let (first, _) = split_rect(self.area, self.direction, self.ratio);
        match self.direction {
            SplitDirection::Horizontal => first.x + first.width,
            SplitDirection::Vertical => first.y + first.height,
        }
    }

    /// Whether `(col, row)` lands on this divider.
    pub fn hit(&self, col: u16, row: u16) -> bool {
        match self.direction {
            SplitDirection::Horizontal => {
                col == self.line() && row >= self.area.y && row < self.area.y + self.area.height
            }
            SplitDirection::Vertical => {
                row == self.line() && col >= self.area.x && col < self.area.x + self.area.width
            }
        }
    }

    /// The ratio that would put the divider at `(col, row)`, clamped to keep at least 5% of the
    /// split on either side.
    pub fn ratio_at(&self, col: u16, row: u16) -> f32 {
        let (pos, start, len) = match self.direction {
            SplitDirection::Horizontal => (col, self.area.x, self.area.width),
            SplitDirection::Vertical => (row, self.area.y, self.area.height),
        };
        if len == 0 {
            return self.ratio;
        }
        (pos.saturating_sub(start) as f32 / len as f32).clamp(0.05, 0.95)
    }

    pub fn id(&self) -> usize {
        self.id
    }
}

/// Collects one `Divider` per `Split` node in the tree, computed against `area`.
fn dividers<T>(tree: &Tree<T>, area: Rect, out: &mut Vec<Divider>) {
    if let Node::Split {
        direction,
        ratio,
        first,
        second,
    } = &tree.node
    {
        out.push(Divider {
            id: tree.id,
            direction: *direction,
            area,
            ratio: *ratio,
        });
        let (first_area, second_area) = split_rect(area, *direction, *ratio);
        dividers(first, first_area, out);
        dividers(second, second_area, out);
    }
}

/// Leaf ids in left-to-right, top-to-bottom tree order.
fn leaf_ids<T>(tree: &Tree<T>, out: &mut Vec<usize>) {
    match &tree.node {
        Node::Leaf(_) => out.push(tree.id),
        Node::Split { first, second, .. } => {
            leaf_ids(first, out);
            leaf_ids(second, out);
        }
    }
}

/// Every leaf's id and on-screen rect (computed against `area`), in tree order.
fn leaf_rects<T>(tree: &Tree<T>, area: Rect, out: &mut Vec<(usize, Rect)>) {
    match &tree.node {
        Node::Leaf(_) => out.push((tree.id, area)),
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (first_area, second_area) = split_rect(area, *direction, *ratio);
            leaf_rects(first, first_area, out);
            leaf_rects(second, second_area, out);
        }
    }
}

/// The pane visually adjacent to `from` in direction `dir`: of the rects that lie entirely past
/// `from`'s edge on that side *and* share part of its perpendicular span, the nearest one — ties
/// (several panes stacked along the far side) go to whichever starts closest to `from`'s own
/// start, so `hjkl` feels like it stays on the row/column you're in. `None` when nothing is
/// there, so the caller can leave focus alone at an edge instead of wrapping around.
fn neighbor(rects: &[(usize, Rect)], from: usize, dir: NudgeDir) -> Option<usize> {
    let (_, f) = *rects.iter().find(|(id, _)| *id == from)?;
    let (fx, fy, fw, fh) = (f.x as i32, f.y as i32, f.width as i32, f.height as i32);
    let overlaps = |a: i32, alen: i32, b: i32, blen: i32| a < b + blen && b < a + alen;

    rects
        .iter()
        .filter(|(id, _)| *id != from)
        .filter_map(|&(id, r)| {
            let (x, y, w, h) = (r.x as i32, r.y as i32, r.width as i32, r.height as i32);
            let (gap, offset) = match dir {
                NudgeDir::Left if x + w <= fx && overlaps(fy, fh, y, h) => (fx - (x + w), y - fy),
                NudgeDir::Right if x >= fx + fw && overlaps(fy, fh, y, h) => {
                    (x - (fx + fw), y - fy)
                }
                NudgeDir::Up if y + h <= fy && overlaps(fx, fw, x, w) => (fy - (y + h), x - fx),
                NudgeDir::Down if y >= fy + fh && overlaps(fx, fw, x, w) => (y - (fy + fh), x - fx),
                _ => return None,
            };
            Some((gap, offset.abs(), id))
        })
        .min()
        .map(|(_, _, id)| id)
}

/// The id of the leaf whose rect (computed against `area`) contains `(col, row)`, if any.
fn focus_at<T>(tree: &Tree<T>, area: Rect, col: u16, row: u16) -> Option<usize> {
    match &tree.node {
        Node::Leaf(_) => {
            let inside = col >= area.x
                && col < area.x + area.width
                && row >= area.y
                && row < area.y + area.height;
            inside.then_some(tree.id)
        }
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (first_area, second_area) = split_rect(area, *direction, *ratio);
            focus_at(first, first_area, col, row)
                .or_else(|| focus_at(second, second_area, col, row))
        }
    }
}

/// Finds a leaf by id and returns a mutable reference to its payload.
fn find_leaf_mut<T>(tree: &mut Tree<T>, id: usize) -> Option<&mut T> {
    match &mut tree.node {
        Node::Leaf(payload) => (tree.id == id).then_some(payload),
        Node::Split { first, second, .. } => {
            find_leaf_mut(first, id).or_else(|| find_leaf_mut(second, id))
        }
    }
}

fn set_ratio_in<T>(tree: &mut Tree<T>, id: usize, ratio: f32) -> bool {
    if tree.id == id {
        if let Node::Split { ratio: r, .. } = &mut tree.node {
            *r = ratio;
            return true;
        }
        return false;
    }
    match &mut tree.node {
        Node::Leaf(_) => false,
        Node::Split { first, second, .. } => {
            set_ratio_in(first, id, ratio) || set_ratio_in(second, id, ratio)
        }
    }
}

/// The id of the `Split` whose *immediate* child (either side) is `target` — not any ancestor
/// further up, unlike `nearest_ancestor_split`, since there's no axis to match here.
fn parent_split_id<T>(tree: &Tree<T>, target: usize) -> Option<usize> {
    match &tree.node {
        Node::Leaf(_) => None,
        Node::Split { first, second, .. } => {
            if first.id == target || second.id == target {
                Some(tree.id)
            } else {
                parent_split_id(first, target).or_else(|| parent_split_id(second, target))
            }
        }
    }
}

/// Flips the `Split` node `id`'s direction (`Horizontal` <-> `Vertical`) in place — the two
/// children and their ratio are untouched, so this only ever changes *how* they're arranged
/// (side by side vs. stacked), never *which* panes they are or their relative share. Mirrors
/// `set_ratio_in`'s search-and-mutate shape.
fn flip_direction_in<T>(tree: &mut Tree<T>, id: usize) -> bool {
    if tree.id == id {
        if let Node::Split { direction, .. } = &mut tree.node {
            *direction = match *direction {
                SplitDirection::Horizontal => SplitDirection::Vertical,
                SplitDirection::Vertical => SplitDirection::Horizontal,
            };
            return true;
        }
        return false;
    }
    match &mut tree.node {
        Node::Leaf(_) => false,
        Node::Split { first, second, .. } => {
            flip_direction_in(first, id) || flip_direction_in(second, id)
        }
    }
}

fn ratio_of<T>(tree: &Tree<T>, id: usize) -> Option<f32> {
    match &tree.node {
        Node::Leaf(_) => None,
        Node::Split {
            ratio,
            first,
            second,
            ..
        } => {
            if tree.id == id {
                Some(*ratio)
            } else {
                ratio_of(first, id).or_else(|| ratio_of(second, id))
            }
        }
    }
}

/// A cardinal keyboard direction for a resize/move chord — see `ShellPanes::resize_focused`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NudgeDir {
    Left,
    Down,
    Up,
    Right,
}

/// Outcome of walking up from a leaf looking for the nearest ancestor `Split` along a given
/// axis — see `nearest_ancestor_split`.
enum AncestorSearch {
    /// The target leaf isn't in this subtree at all.
    NotFound,
    /// The target leaf was found, but nothing between it and here matches the requested axis —
    /// still bubbling up looking for one.
    Located,
    /// The nearest matching-axis ancestor: its split id, and whether the target sits under its
    /// `first` child (as opposed to `second`).
    Found(usize, bool),
}

/// Walks up from `target` to the nearest ancestor `Split` whose direction is `axis`, tracking
/// which side of it `target` is on. Mirrors `dividers`' recursion shape but needs the extra
/// "still looking" state (`Located`) since a matching split isn't necessarily the target's
/// immediate parent — it's the first one found while unwinding.
fn nearest_ancestor_split<T>(tree: &Tree<T>, target: usize, axis: SplitDirection) -> AncestorSearch {
    match &tree.node {
        Node::Leaf(_) => {
            if tree.id == target {
                AncestorSearch::Located
            } else {
                AncestorSearch::NotFound
            }
        }
        Node::Split {
            direction,
            first,
            second,
            ..
        } => {
            let side = match nearest_ancestor_split(first, target, axis) {
                found @ AncestorSearch::Found(..) => return found,
                AncestorSearch::Located => true,
                AncestorSearch::NotFound => match nearest_ancestor_split(second, target, axis) {
                    found @ AncestorSearch::Found(..) => return found,
                    AncestorSearch::Located => false,
                    AncestorSearch::NotFound => return AncestorSearch::NotFound,
                },
            };
            if *direction == axis {
                AncestorSearch::Found(tree.id, side)
            } else {
                AncestorSearch::Located
            }
        }
    }
}

/// The ratio step a single resize-chord keypress moves — 5%, the same minimum slice
/// `Divider::ratio_at` already clamps a mouse drag to, so a few presses can still reach either
/// edge.
const RESIZE_STEP: f32 = 0.05;

enum CloseResult<T> {
    /// `target` isn't in this subtree — hands the (unmodified) subtree back so the caller can
    /// keep searching its sibling.
    NotFound(Tree<T>),
    /// `target` was this subtree's only leaf — nothing is left of it, just the payload that was
    /// removed (for the caller to tear down).
    RemovedToNone(T),
    /// `target` was found and removed; its sibling is promoted in its place.
    RemovedTo(Tree<T>, T),
}

/// Removes the leaf `target` from `tree`, promoting its sibling into the space its parent
/// `Split` used to occupy. Pure tree surgery — the caller is responsible for anything that needs
/// to happen to the removed payload (e.g. killing the shell it wraps).
fn close_id<T>(tree: Tree<T>, target: usize) -> CloseResult<T> {
    if tree.id == target {
        return match tree.node {
            Node::Leaf(payload) => CloseResult::RemovedToNone(payload),
            other => CloseResult::NotFound(Tree {
                id: tree.id,
                node: other,
            }),
        };
    }
    match tree.node {
        Node::Leaf(_) => CloseResult::NotFound(tree),
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => match close_id(*first, target) {
            CloseResult::RemovedToNone(payload) => CloseResult::RemovedTo(*second, payload),
            CloseResult::RemovedTo(new_first, payload) => CloseResult::RemovedTo(
                Tree {
                    id: tree.id,
                    node: Node::Split {
                        direction,
                        ratio,
                        first: Box::new(new_first),
                        second,
                    },
                },
                payload,
            ),
            CloseResult::NotFound(first) => match close_id(*second, target) {
                CloseResult::RemovedToNone(payload) => CloseResult::RemovedTo(first, payload),
                CloseResult::RemovedTo(new_second, payload) => CloseResult::RemovedTo(
                    Tree {
                        id: tree.id,
                        node: Node::Split {
                            direction,
                            ratio,
                            first: Box::new(first),
                            second: Box::new(new_second),
                        },
                    },
                    payload,
                ),
                CloseResult::NotFound(second) => CloseResult::NotFound(Tree {
                    id: tree.id,
                    node: Node::Split {
                        direction,
                        ratio,
                        first: Box::new(first),
                        second: Box::new(second),
                    },
                }),
            },
        },
    }
}

enum SplitOutcome<T> {
    /// `target` isn't in this subtree — hands back the (unmodified) subtree *and* `new_payload`,
    /// since it was never consumed, so the caller can retry it against the sibling.
    NotFound(Tree<T>, T),
    Split(Tree<T>),
}

/// Replaces the leaf `target` in `tree` with a `Split` holding the old leaf and a new one
/// wrapping `new_payload`. Pure tree surgery — the caller has already constructed `new_payload`
/// (spawning a shell can fail, so that happens before this, which cannot).
fn split_id<T>(
    tree: Tree<T>,
    target: usize,
    direction: SplitDirection,
    new_leaf_id: usize,
    split_node_id: usize,
    new_payload: T,
) -> SplitOutcome<T> {
    if tree.id == target {
        return match tree.node {
            Node::Leaf(payload) => SplitOutcome::Split(Tree {
                id: split_node_id,
                node: Node::Split {
                    direction,
                    ratio: 0.5,
                    first: Box::new(Tree {
                        id: tree.id,
                        node: Node::Leaf(payload),
                    }),
                    second: Box::new(Tree {
                        id: new_leaf_id,
                        node: Node::Leaf(new_payload),
                    }),
                },
            }),
            other => SplitOutcome::NotFound(
                Tree {
                    id: tree.id,
                    node: other,
                },
                new_payload,
            ),
        };
    }
    match tree.node {
        Node::Leaf(_) => SplitOutcome::NotFound(tree, new_payload),
        Node::Split {
            direction: d,
            ratio,
            first,
            second,
        } => match split_id(
            *first,
            target,
            direction,
            new_leaf_id,
            split_node_id,
            new_payload,
        ) {
            SplitOutcome::Split(new_first) => SplitOutcome::Split(Tree {
                id: tree.id,
                node: Node::Split {
                    direction: d,
                    ratio,
                    first: Box::new(new_first),
                    second,
                },
            }),
            SplitOutcome::NotFound(first, new_payload) => {
                match split_id(
                    *second,
                    target,
                    direction,
                    new_leaf_id,
                    split_node_id,
                    new_payload,
                ) {
                    SplitOutcome::Split(new_second) => SplitOutcome::Split(Tree {
                        id: tree.id,
                        node: Node::Split {
                            direction: d,
                            ratio,
                            first: Box::new(first),
                            second: Box::new(new_second),
                        },
                    }),
                    SplitOutcome::NotFound(second, new_payload) => SplitOutcome::NotFound(
                        Tree {
                            id: tree.id,
                            node: Node::Split {
                                direction: d,
                                ratio,
                                first: Box::new(first),
                                second: Box::new(second),
                            },
                        },
                        new_payload,
                    ),
                }
            }
        },
    }
}

fn resize_tree(tree: &ShellTree, area: Rect) -> anyhow::Result<()> {
    match &tree.node {
        Node::Leaf(shell) => {
            let (rows, cols) = pty_size(area);
            shell.resize(rows, cols)
        }
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (first_area, second_area) = split_rect(area, *direction, *ratio);
            resize_tree(first, first_area)?;
            resize_tree(second, second_area)
        }
    }
}

fn render_tree(
    tree: &ShellTree,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    config: &Config,
    focused: usize,
) {
    match &tree.node {
        Node::Leaf(shell) => {
            crate::popup_shell::render(frame, area, shell, config, tree.id == focused)
        }
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (first_area, second_area) = split_rect(area, *direction, *ratio);
            render_tree(first, frame, first_area, config, focused);
            render_tree(second, frame, second_area, config, focused);
        }
    }
}

fn poll_exits_tree(tree: &mut ShellTree, out: &mut Vec<(usize, ExitOutcome)>) -> io::Result<()> {
    match &mut tree.node {
        Node::Leaf(shell) => {
            if let Some(outcome) = shell.try_wait()? {
                out.push((tree.id, outcome));
            }
            Ok(())
        }
        Node::Split { first, second, .. } => {
            poll_exits_tree(first, out)?;
            poll_exits_tree(second, out)
        }
    }
}

/// Owns the whole tree of shell panes tiling (part of) the frame. `main.rs` holds this behind an
/// `Option` — `None` means no shell panes are open at all.
pub struct ShellPanes {
    root: ShellTree,
    next_id: usize,
    focused: usize,
}

impl ShellPanes {
    /// Spawns the first shell as the tree's sole pane, sized to `area`.
    pub fn open(cwd: &Path, area: Rect) -> anyhow::Result<Self> {
        let (rows, cols) = pty_size(area);
        let shell = PopupShell::spawn(cwd, rows, cols)?;
        Ok(Self {
            root: Tree {
                id: 0,
                node: Node::Leaf(shell),
            },
            next_id: 1,
            focused: 0,
        })
    }

    /// Recomputes every pane's on-screen rect from `area` and resizes its pty to match. Call
    /// whenever the frame is resized or the tree's shape changes.
    pub fn resize(&self, area: Rect) -> anyhow::Result<()> {
        resize_tree(&self.root, area)
    }

    pub fn render(&self, frame: &mut ratatui::Frame<'_>, area: Rect, config: &Config) {
        render_tree(&self.root, frame, area, config, self.focused);
    }

    /// One `Divider` per split in the tree, for hit-testing a mouse-down against.
    pub fn dividers(&self, area: Rect) -> Vec<Divider> {
        let mut out = Vec::new();
        dividers(&self.root, area, &mut out);
        out
    }

    /// Applies a divider drag: `id` must be one from a `dividers` call against the same `area`.
    pub fn set_ratio(&mut self, id: usize, ratio: f32, area: Rect) -> anyhow::Result<()> {
        if set_ratio_in(&mut self.root, id, ratio) {
            self.resize(area)?;
        }
        Ok(())
    }

    /// Flips the orientation (side-by-side <-> stacked) of the split the focused pane is
    /// immediately part of, keeping the same two panes and their ratio — only how they're
    /// arranged changes. Returns whether there was a split to flip at all; `false` for a lone
    /// pane with no parent split.
    pub fn toggle_focused_orientation(&mut self, area: Rect) -> anyhow::Result<bool> {
        if let Some(split_id) = parent_split_id(&self.root, self.focused)
            && flip_direction_in(&mut self.root, split_id)
        {
            self.resize(area)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// The keyboard equivalent of dragging a divider: nudges the nearest ancestor split along
    /// `dir`'s axis by `RESIZE_STEP`, growing the focused pane for `Right`/`Down` and shrinking
    /// it for `Left`/`Up` — regardless of which side of that split the focused pane is actually
    /// on, so the key's direction always matches what visibly happens to the pane you're looking
    /// at. Returns whether a divider was actually found and adjusted; `false` (e.g. `Left`/`Right`
    /// on a lone pane, or one only ever split vertically) tells the caller there was nothing to
    /// resize along that axis, so it can fall back to resizing the box itself instead.
    pub fn resize_focused(&mut self, dir: NudgeDir, area: Rect) -> anyhow::Result<bool> {
        let (axis, grow) = match dir {
            NudgeDir::Left => (SplitDirection::Horizontal, false),
            NudgeDir::Right => (SplitDirection::Horizontal, true),
            NudgeDir::Up => (SplitDirection::Vertical, false),
            NudgeDir::Down => (SplitDirection::Vertical, true),
        };
        if let AncestorSearch::Found(split_id, is_first) =
            nearest_ancestor_split(&self.root, self.focused, axis)
        {
            // The focused pane's share is `ratio` when it's `first`, `1.0 - ratio` otherwise —
            // growing it means moving `ratio` in the matching direction for each case.
            let sign = if is_first == grow { 1.0 } else { -1.0 };
            if let Some(current) = ratio_of(&self.root, split_id) {
                let new_ratio = (current + sign * RESIZE_STEP).clamp(0.05, 0.95);
                self.set_ratio(split_id, new_ratio, area)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Focuses whichever pane's rect (against `area`) contains `(col, row)`. Returns whether a
    /// pane was actually hit.
    pub fn focus_at(&mut self, area: Rect, col: u16, row: u16) -> bool {
        match focus_at(&self.root, area, col, row) {
            Some(id) => {
                self.focused = id;
                true
            }
            None => false,
        }
    }

    /// Moves focus to the pane adjacent to the focused one in direction `dir` (against `area`).
    /// Returns whether there was one — `false` at the box's edge, where focus stays put.
    pub fn focus_direction(&mut self, dir: NudgeDir, area: Rect) -> bool {
        let mut rects = Vec::new();
        leaf_rects(&self.root, area, &mut rects);
        match neighbor(&rects, self.focused, dir) {
            Some(id) => {
                self.focused = id;
                true
            }
            None => false,
        }
    }

    pub fn focused_id(&self) -> usize {
        self.focused
    }

    /// The focused pane's on-screen rect against `area`, so a caller can pick a split direction
    /// from its shape. Falls back to `area` itself, which is only reachable if `focused` were
    /// stale — something every mutation already rules out.
    pub fn focused_rect(&self, area: Rect) -> Rect {
        let mut rects = Vec::new();
        leaf_rects(&self.root, area, &mut rects);
        rects
            .into_iter()
            .find(|(id, _)| *id == self.focused)
            .map_or(area, |(_, rect)| rect)
    }

    /// The currently-focused pane's shell — always present: every mutation that could remove or
    /// replace the focused leaf also reassigns `focused` to one that still exists.
    pub fn focused_shell(&mut self) -> &mut PopupShell {
        find_leaf_mut(&mut self.root, self.focused)
            .expect("`focused` always names a leaf that still exists in the tree")
    }

    /// Non-blocking check for every pane's shell having exited, e.g. via its own `exit` command.
    pub fn poll_exits(&mut self) -> io::Result<Vec<(usize, ExitOutcome)>> {
        let mut out = Vec::new();
        poll_exits_tree(&mut self.root, &mut out)?;
        Ok(out)
    }

    /// Splits the focused pane, spawning a new shell in `cwd` for the new side; the new pane
    /// becomes focused. `area` immediately resizes every pane's pty to its real, post-split rect.
    pub fn split(
        mut self,
        direction: SplitDirection,
        cwd: &Path,
        area: Rect,
    ) -> anyhow::Result<Self> {
        let new_leaf_id = self.next_id;
        let split_node_id = self.next_id + 1;
        self.next_id += 2;
        let new_shell = PopupShell::spawn(cwd, 1, 1)?;

        match split_id(
            self.root,
            self.focused,
            direction,
            new_leaf_id,
            split_node_id,
            new_shell,
        ) {
            SplitOutcome::Split(new_root) => {
                self.root = new_root;
                self.focused = new_leaf_id;
            }
            SplitOutcome::NotFound(root, mut unused_shell) => {
                // `focused` always names an existing leaf, so this never actually triggers — if
                // it somehow did, close the shell just spawned above rather than leaking it.
                self.root = root;
                let _ = unused_shell.close();
            }
        }
        self.resize(area)?;
        Ok(self)
    }

    /// Closes the pane `id`, promoting its sibling into the space it used to occupy. Returns
    /// `None` if `id` was the tree's last remaining pane (the whole tree is now empty).
    pub fn close(mut self, id: usize, area: Rect) -> anyhow::Result<Option<Self>> {
        match close_id(self.root, id) {
            CloseResult::RemovedToNone(mut shell) => {
                let _ = shell.close();
                Ok(None)
            }
            CloseResult::RemovedTo(new_root, mut shell) => {
                let _ = shell.close();
                self.root = new_root;
                if self.focused == id {
                    let mut ids = Vec::new();
                    leaf_ids(&self.root, &mut ids);
                    self.focused = *ids
                        .first()
                        .expect("a promoted subtree always keeps at least one leaf");
                }
                self.resize(area)?;
                Ok(Some(self))
            }
            CloseResult::NotFound(root) => {
                self.root = root;
                Ok(Some(self))
            }
        }
    }

    /// Closes every pane, killing and reaping each shell — dropping a `ShellPanes` would leave
    /// the child processes running, since `PopupShell` has no `Drop` of its own. Iterates over a
    /// snapshot of the ids so it terminates even if a close were ever to report `NotFound`.
    pub fn close_all(mut self, area: Rect) -> anyhow::Result<()> {
        let mut ids = Vec::new();
        leaf_ids(&self.root, &mut ids);
        for id in ids {
            match self.close(id, area)? {
                Some(rest) => self = rest,
                None => return Ok(()),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: usize, payload: &str) -> Tree<String> {
        Tree {
            id,
            node: Node::Leaf(payload.to_string()),
        }
    }

    fn split(
        id: usize,
        direction: SplitDirection,
        ratio: f32,
        first: Tree<String>,
        second: Tree<String>,
    ) -> Tree<String> {
        Tree {
            id,
            node: Node::Split {
                direction,
                ratio,
                first: Box::new(first),
                second: Box::new(second),
            },
        }
    }

    /// A leaf's rendered rect, computed the same way `render_tree`/`resize_tree` do — for
    /// asserting on layout shape (e.g. after `toggle_focused_orientation`) without needing a
    /// real `Divider`.
    fn rect_of<T>(tree: &Tree<T>, area: Rect, target: usize) -> Option<Rect> {
        match &tree.node {
            Node::Leaf(_) => (tree.id == target).then_some(area),
            Node::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (first_area, second_area) = split_rect(area, *direction, *ratio);
                rect_of(first, first_area, target).or_else(|| rect_of(second, second_area, target))
            }
        }
    }

    fn shell_rect_of(panes: &ShellPanes, area: Rect, id: usize) -> Rect {
        rect_of(&panes.root, area, id).expect("id must name a leaf in the tree")
    }

    #[test]
    fn split_rect_divides_horizontally_by_ratio() {
        let (first, second) = split_rect(Rect::new(0, 0, 100, 40), SplitDirection::Horizontal, 0.5);
        assert_eq!(first, Rect::new(0, 0, 50, 40));
        assert_eq!(second, Rect::new(50, 0, 50, 40));
    }

    #[test]
    fn split_rect_divides_vertically_by_ratio() {
        let (first, second) = split_rect(Rect::new(0, 0, 100, 40), SplitDirection::Vertical, 0.25);
        assert_eq!(first, Rect::new(0, 0, 100, 10));
        assert_eq!(second, Rect::new(0, 10, 100, 30));
    }

    #[test]
    fn split_rect_never_panics_on_a_tiny_area() {
        let (first, second) = split_rect(Rect::new(0, 0, 1, 1), SplitDirection::Horizontal, 0.5);
        assert_eq!(first.width + second.width, 1);
        // A zero-sized area is already degenerate; the property under test is that this doesn't
        // panic via integer underflow, not that the halves sum back to exactly zero.
        let _ = split_rect(Rect::new(0, 0, 0, 0), SplitDirection::Vertical, 0.5);
    }

    #[test]
    fn divider_hit_and_ratio_at_match_the_rendered_line() {
        let area = Rect::new(0, 0, 100, 40);
        let tree = split(
            0,
            SplitDirection::Horizontal,
            0.5,
            leaf(1, "a"),
            leaf(2, "b"),
        );
        let mut found = Vec::new();
        dividers(&tree, area, &mut found);
        assert_eq!(found.len(), 1);
        let divider = found[0];
        assert_eq!(divider.id(), 0);
        assert!(divider.hit(50, 20));
        assert!(!divider.hit(49, 20));
        assert!(!divider.hit(50, 40)); // outside the split's row range
        assert!((divider.ratio_at(25, 20) - 0.25).abs() < 0.01);
    }

    #[test]
    fn leaf_ids_are_left_to_right_top_to_bottom() {
        let tree = split(
            0,
            SplitDirection::Horizontal,
            0.5,
            leaf(1, "a"),
            split(2, SplitDirection::Vertical, 0.5, leaf(3, "b"), leaf(4, "c")),
        );
        let mut ids = Vec::new();
        leaf_ids(&tree, &mut ids);
        assert_eq!(ids, vec![1, 3, 4]);
    }

    /// Three panes: `1` on the left half, `2` top-right, `3` bottom-right.
    fn left_and_two_stacked_right() -> Tree<String> {
        split(
            0,
            SplitDirection::Horizontal,
            0.5,
            leaf(1, "a"),
            split(
                10,
                SplitDirection::Vertical,
                0.5,
                leaf(2, "b"),
                leaf(3, "c"),
            ),
        )
    }

    fn neighbor_of(tree: &Tree<String>, from: usize, dir: NudgeDir) -> Option<usize> {
        let mut rects = Vec::new();
        leaf_rects(tree, Rect::new(0, 0, 100, 40), &mut rects);
        neighbor(&rects, from, dir)
    }

    #[test]
    fn neighbor_moves_between_adjacent_panes() {
        let tree = left_and_two_stacked_right();
        assert_eq!(neighbor_of(&tree, 1, NudgeDir::Right), Some(2));
        assert_eq!(neighbor_of(&tree, 2, NudgeDir::Down), Some(3));
        assert_eq!(neighbor_of(&tree, 3, NudgeDir::Up), Some(2));
        assert_eq!(neighbor_of(&tree, 3, NudgeDir::Left), Some(1));
    }

    #[test]
    fn neighbor_is_none_at_the_edge_instead_of_wrapping() {
        let tree = left_and_two_stacked_right();
        assert_eq!(neighbor_of(&tree, 1, NudgeDir::Left), None);
        assert_eq!(neighbor_of(&tree, 1, NudgeDir::Up), None);
        assert_eq!(neighbor_of(&tree, 2, NudgeDir::Up), None);
        assert_eq!(neighbor_of(&tree, 3, NudgeDir::Down), None);
        assert_eq!(neighbor_of(&tree, 3, NudgeDir::Right), None);
    }

    #[test]
    fn neighbor_ignores_panes_that_do_not_share_the_axis() {
        // `2` is top-right and `3` bottom-right: neither is beside the other horizontally.
        let tree = left_and_two_stacked_right();
        assert_eq!(neighbor_of(&tree, 2, NudgeDir::Left), Some(1));
        assert_eq!(neighbor_of(&tree, 2, NudgeDir::Right), None);
    }

    #[test]
    fn neighbor_prefers_the_pane_level_with_the_focused_one() {
        // Left pane `1` is tall; on its right sit `2` (top) and `3` (bottom). Coming from `3`,
        // going left lands on `1`; from `1`, going right prefers the one starting nearest its
        // own top edge, which is `2`.
        let tree = left_and_two_stacked_right();
        assert_eq!(neighbor_of(&tree, 1, NudgeDir::Right), Some(2));
    }

    #[test]
    fn neighbor_of_an_unknown_pane_is_none() {
        let tree = left_and_two_stacked_right();
        assert_eq!(neighbor_of(&tree, 99, NudgeDir::Left), None);
    }

    #[test]
    fn focus_at_routes_a_click_to_the_leaf_under_it() {
        let area = Rect::new(0, 0, 100, 40);
        let tree = split(
            0,
            SplitDirection::Horizontal,
            0.5,
            leaf(1, "a"),
            leaf(2, "b"),
        );
        assert_eq!(focus_at(&tree, area, 10, 10), Some(1));
        assert_eq!(focus_at(&tree, area, 90, 10), Some(2));
    }

    #[test]
    fn close_id_promotes_the_sibling_and_returns_the_removed_payload() {
        let tree = split(
            0,
            SplitDirection::Horizontal,
            0.5,
            leaf(1, "a"),
            leaf(2, "b"),
        );
        match close_id(tree, 1) {
            CloseResult::RemovedTo(promoted, payload) => {
                assert_eq!(payload, "a");
                assert_eq!(promoted.id, 2);
            }
            _ => panic!("expected RemovedTo"),
        }
    }

    #[test]
    fn close_id_on_the_only_leaf_empties_the_tree() {
        let tree = leaf(0, "only");
        match close_id(tree, 0) {
            CloseResult::RemovedToNone(payload) => assert_eq!(payload, "only"),
            _ => panic!("expected RemovedToNone"),
        }
    }

    #[test]
    fn split_id_wraps_the_target_leaf_in_a_new_split() {
        let tree = leaf(0, "original");
        match split_id(tree, 0, SplitDirection::Horizontal, 1, 2, "new".to_string()) {
            SplitOutcome::Split(new_root) => {
                assert_eq!(new_root.id, 2);
                let Node::Split { first, second, .. } = new_root.node else {
                    panic!("expected a Split node");
                };
                assert_eq!(first.id, 0);
                assert_eq!(second.id, 1);
            }
            SplitOutcome::NotFound(..) => panic!("expected Split"),
        }
    }

    #[test]
    fn split_id_hands_the_unused_payload_back_when_the_target_is_missing() {
        let tree = leaf(0, "original");
        match split_id(
            tree,
            99,
            SplitDirection::Horizontal,
            1,
            2,
            "new".to_string(),
        ) {
            SplitOutcome::NotFound(tree, payload) => {
                assert_eq!(tree.id, 0);
                assert_eq!(payload, "new");
            }
            SplitOutcome::Split(_) => panic!("expected NotFound"),
        }
    }

    #[test]
    fn split_id_finds_the_target_inside_the_second_child() {
        // Regression test: the target leaf sits in `second`, so `new_payload` must survive the
        // `first` recursion (which won't consume it) and still be available for `second`'s.
        let tree = split(
            0,
            SplitDirection::Horizontal,
            0.5,
            leaf(1, "a"),
            leaf(2, "b"),
        );
        match split_id(tree, 2, SplitDirection::Vertical, 3, 4, "new".to_string()) {
            SplitOutcome::Split(new_root) => {
                let Node::Split { first, second, .. } = new_root.node else {
                    panic!("expected a Split node");
                };
                assert_eq!(first.id, 1);
                let Node::Split {
                    first: inner_first,
                    second: inner_second,
                    ..
                } = second.node
                else {
                    panic!("expected the second child to itself be a Split");
                };
                assert_eq!(inner_first.id, 2);
                assert_eq!(inner_second.id, 3);
            }
            SplitOutcome::NotFound(..) => panic!("expected Split"),
        }
    }

    #[test]
    fn close_all_tears_down_a_split_tree_without_error() {
        let area = Rect::new(0, 0, 80, 24);
        let panes = ShellPanes::open(&std::env::temp_dir(), area).unwrap();
        let panes = panes
            .split(SplitDirection::Horizontal, &std::env::temp_dir(), area)
            .unwrap();
        let panes = panes
            .split(SplitDirection::Vertical, &std::env::temp_dir(), area)
            .unwrap();
        panes.close_all(area).unwrap();
    }

    #[test]
    fn focused_rect_of_a_lone_pane_is_the_whole_area() {
        let area = Rect::new(2, 3, 80, 24);
        let panes = ShellPanes::open(&std::env::temp_dir(), area).unwrap();
        assert_eq!(panes.focused_rect(area), area);

        let id = panes.focused_id();
        assert!(panes.close(id, area).unwrap().is_none());
    }

    #[test]
    fn open_creates_a_single_focused_pane() {
        let area = Rect::new(0, 0, 80, 24);
        let panes = ShellPanes::open(&std::env::temp_dir(), area).unwrap();
        let mut ids = Vec::new();
        leaf_ids(&panes.root, &mut ids);
        assert_eq!(ids.len(), 1);
        assert_eq!(panes.focused_id(), ids[0]);

        let id = panes.focused_id();
        assert!(panes.close(id, area).unwrap().is_none());
    }

    #[test]
    fn nearest_ancestor_split_finds_the_immediate_parent() {
        let tree = split(
            0,
            SplitDirection::Horizontal,
            0.5,
            leaf(1, "a"),
            leaf(2, "b"),
        );
        match nearest_ancestor_split(&tree, 1, SplitDirection::Horizontal) {
            AncestorSearch::Found(id, is_first) => {
                assert_eq!(id, 0);
                assert!(is_first);
            }
            _ => panic!("expected Found"),
        }
        match nearest_ancestor_split(&tree, 2, SplitDirection::Horizontal) {
            AncestorSearch::Found(id, is_first) => {
                assert_eq!(id, 0);
                assert!(!is_first);
            }
            _ => panic!("expected Found"),
        }
    }

    #[test]
    fn nearest_ancestor_split_skips_a_non_matching_axis_to_find_the_grandparent() {
        // `1` sits directly under a Vertical split, but the nearest Horizontal one is the root.
        let tree = split(
            0,
            SplitDirection::Horizontal,
            0.5,
            split(2, SplitDirection::Vertical, 0.5, leaf(1, "a"), leaf(3, "b")),
            leaf(4, "c"),
        );
        match nearest_ancestor_split(&tree, 1, SplitDirection::Horizontal) {
            AncestorSearch::Found(id, is_first) => {
                assert_eq!(id, 0);
                assert!(is_first);
            }
            _ => panic!("expected Found"),
        }
        match nearest_ancestor_split(&tree, 1, SplitDirection::Vertical) {
            AncestorSearch::Found(id, is_first) => {
                assert_eq!(id, 2);
                assert!(is_first);
            }
            _ => panic!("expected Found"),
        }
    }

    #[test]
    fn nearest_ancestor_split_returns_located_when_no_axis_matches() {
        // The tree only ever splits horizontally, so a vertical ancestor doesn't exist — the
        // leaf itself is still found, just with nothing matching to report.
        let tree = split(
            0,
            SplitDirection::Horizontal,
            0.5,
            leaf(1, "a"),
            leaf(2, "b"),
        );
        assert!(matches!(
            nearest_ancestor_split(&tree, 1, SplitDirection::Vertical),
            AncestorSearch::Located
        ));
    }

    #[test]
    fn nearest_ancestor_split_returns_not_found_for_an_unknown_leaf() {
        let tree = leaf(0, "only");
        assert!(matches!(
            nearest_ancestor_split(&tree, 99, SplitDirection::Horizontal),
            AncestorSearch::NotFound
        ));
    }

    #[test]
    fn resize_focused_grows_the_focused_pane_regardless_of_which_side_it_is_on() {
        let area = Rect::new(0, 0, 100, 40);
        // The new (right-hand, `second`) pane is focused after a split.
        let mut panes = ShellPanes::open(&std::env::temp_dir(), area)
            .unwrap()
            .split(SplitDirection::Horizontal, &std::env::temp_dir(), area)
            .unwrap();
        let before = ratio_of(&panes.root, 2).unwrap();
        assert!(panes.resize_focused(NudgeDir::Right, area).unwrap());
        let after = ratio_of(&panes.root, 2).unwrap();
        // Growing the focused (second) pane shrinks `ratio`, which is `first`'s share.
        assert!(after < before);

        assert!(panes.resize_focused(NudgeDir::Left, area).unwrap());
        let restored = ratio_of(&panes.root, 2).unwrap();
        assert!((restored - before).abs() < 0.001);
    }

    #[test]
    fn resize_focused_on_a_lone_pane_reports_no_divider_to_adjust() {
        let area = Rect::new(0, 0, 100, 40);
        let mut panes = ShellPanes::open(&std::env::temp_dir(), area).unwrap();
        // No ancestor split exists at all — must not panic, and the caller needs to know nothing
        // happened so it can fall back to resizing the box itself.
        assert!(!panes.resize_focused(NudgeDir::Right, area).unwrap());
        assert!(!panes.resize_focused(NudgeDir::Down, area).unwrap());
    }

    #[test]
    fn parent_split_id_finds_the_immediate_parent_only() {
        let tree = split(
            0,
            SplitDirection::Horizontal,
            0.5,
            leaf(1, "a"),
            split(2, SplitDirection::Vertical, 0.5, leaf(3, "b"), leaf(4, "c")),
        );
        assert_eq!(parent_split_id(&tree, 1), Some(0));
        assert_eq!(parent_split_id(&tree, 3), Some(2));
        assert_eq!(parent_split_id(&tree, 4), Some(2));
        // `0` is the root split itself, not a leaf — it has no parent.
        assert_eq!(parent_split_id(&tree, 0), None);
        assert_eq!(parent_split_id(&tree, 99), None);
    }

    #[test]
    fn flip_direction_in_toggles_only_the_named_split() {
        let mut tree = split(
            0,
            SplitDirection::Horizontal,
            0.5,
            leaf(1, "a"),
            split(2, SplitDirection::Vertical, 0.5, leaf(3, "b"), leaf(4, "c")),
        );
        assert!(flip_direction_in(&mut tree, 2));

        let Node::Split {
            direction: root_direction,
            second,
            ..
        } = &tree.node
        else {
            panic!("expected the root to still be a Split");
        };
        // The root split (id 0) was never named — it must be untouched.
        assert_eq!(*root_direction, SplitDirection::Horizontal);

        let Node::Split {
            direction: inner_direction,
            ..
        } = &second.node
        else {
            panic!("expected the inner split (id 2) to still be a Split");
        };
        // Only the named split (id 2) flips: it started `Vertical`, so it's now `Horizontal`.
        assert_eq!(*inner_direction, SplitDirection::Horizontal);
    }

    #[test]
    fn toggle_focused_orientation_flips_the_parent_split_and_keeps_the_same_panes() {
        let area = Rect::new(0, 0, 100, 40);
        // Focused (`second`) pane is side by side with the first after a horizontal split.
        let mut panes = ShellPanes::open(&std::env::temp_dir(), area)
            .unwrap()
            .split(SplitDirection::Horizontal, &std::env::temp_dir(), area)
            .unwrap();
        let focused_before = panes.focused_id();
        let mut ids_before = Vec::new();
        leaf_ids(&panes.root, &mut ids_before);

        assert!(panes.toggle_focused_orientation(area).unwrap());

        let mut ids_after = Vec::new();
        leaf_ids(&panes.root, &mut ids_after);
        assert_eq!(ids_before, ids_after, "toggling must not reorder or replace panes");
        assert_eq!(panes.focused_id(), focused_before, "focus must not change");

        // The two panes now stack vertically instead of sitting side by side: their rects should
        // share the same x/width and differ in y, the opposite of the pre-toggle layout.
        let a = shell_rect_of(&panes, area, ids_after[0]);
        let b = shell_rect_of(&panes, area, ids_after[1]);
        assert_eq!(a.x, b.x);
        assert_eq!(a.width, b.width);
        assert_ne!(a.y, b.y);

        // Toggling again restores the original side-by-side layout.
        assert!(panes.toggle_focused_orientation(area).unwrap());
        let a = shell_rect_of(&panes, area, ids_after[0]);
        let b = shell_rect_of(&panes, area, ids_after[1]);
        assert_eq!(a.y, b.y);
        assert_eq!(a.height, b.height);
        assert_ne!(a.x, b.x);
    }

    #[test]
    fn toggle_focused_orientation_on_a_lone_pane_is_a_no_op() {
        let area = Rect::new(0, 0, 100, 40);
        let mut panes = ShellPanes::open(&std::env::temp_dir(), area).unwrap();
        assert!(!panes.toggle_focused_orientation(area).unwrap());
    }

    #[test]
    fn split_then_close_the_new_pane_restores_the_original_focus() {
        let area = Rect::new(0, 0, 80, 24);
        let panes = ShellPanes::open(&std::env::temp_dir(), area).unwrap();
        let original_id = panes.focused_id();

        let panes = panes
            .split(SplitDirection::Horizontal, &std::env::temp_dir(), area)
            .unwrap();
        let new_id = panes.focused_id();
        assert_ne!(new_id, original_id);
        let mut ids = Vec::new();
        leaf_ids(&panes.root, &mut ids);
        assert_eq!(ids.len(), 2);

        let panes = panes.close(new_id, area).unwrap().unwrap();
        assert_eq!(panes.focused_id(), original_id);

        assert!(panes.close(original_id, area).unwrap().is_none());
    }
}
