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
//! `focus_at`, `leaf_ids`) is generic over the leaf payload so it can be unit-tested without
//! spawning real shells; `ShellPanes` specializes it to `PopupShell` and owns everything that
//! actually touches a pty (spawning, resizing, rendering, reaping exited shells).

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

/// Leaf ids in left-to-right, top-to-bottom tree order — the order `focus_next`/`focus_prev`
/// cycle through.
fn leaf_ids<T>(tree: &Tree<T>, out: &mut Vec<usize>) {
    match &tree.node {
        Node::Leaf(_) => out.push(tree.id),
        Node::Split { first, second, .. } => {
            leaf_ids(first, out);
            leaf_ids(second, out);
        }
    }
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

    pub fn focus_next(&mut self) {
        self.cycle_focus(1);
    }

    fn cycle_focus(&mut self, delta: i32) {
        let mut ids = Vec::new();
        leaf_ids(&self.root, &mut ids);
        let Some(pos) = ids.iter().position(|&id| id == self.focused) else {
            return;
        };
        let len = ids.len() as i32;
        let next = (pos as i32 + delta).rem_euclid(len) as usize;
        self.focused = ids[next];
    }

    pub fn focused_id(&self) -> usize {
        self.focused
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
