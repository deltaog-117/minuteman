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

//! Where everything in the wiring diagram goes. Pure: a `Topology` and a width in, boxes and
//! lines out, with no terminal, so the drawing and the mouse ask the same function and cannot
//! disagree about where a node is.
//!
//! Two layouts share one model. The wide one puts each layer in a column, left to right (the bus,
//! the disk, its partitions, what is stacked on them, the filesystem), a box for each node and
//! lines between them; the narrow one, used when the columns would not fit the window, is the same
//! tree as one indented row per node. Positions are on a canvas that starts at the top left; the
//! view shifts them to the screen and scrolls.

use std::collections::HashMap;

use devices::topology::{NodeKind, Topology};
use ratatui::layout::Rect;

/// The width of a node's box, and the room for lines between two columns.
pub const NODE_W: u16 = 20;
pub const GAP: u16 = 4;
/// A box is a frame round three lines; a folder's is a frame round one.
pub const NODE_H: u16 = 5;
pub const FOLDER_H: u16 = 3;
/// Blank rows between two boxes in a column.
const ROW_GAP: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Boxes in columns, joined by lines.
    Wide,
    /// One indented row per node.
    Narrow,
}

/// A node's box on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub node: usize,
    pub rect: Rect,
}

/// A node's row in the narrow layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub node: usize,
    /// The branch drawing in front of the name: `│  ├─ `.
    pub prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub mode: Mode,
    /// Wide only: every node's box.
    pub placed: Vec<Placed>,
    /// Narrow only: every node's row, top to bottom.
    pub rows: Vec<Row>,
    /// The size of the canvas, in cells.
    pub size: (u16, u16),
    /// The lines of the diagram, as (parent, child), for the nodes drawn under a first parent.
    pub links: Vec<(usize, usize)>,
}

impl Layout {
    /// The box of `node`, in a wide layout.
    pub fn rect_of(&self, node: usize) -> Option<Rect> {
        self.placed.iter().find(|p| p.node == node).map(|p| p.rect)
    }

    /// The row of `node`, in a narrow layout.
    pub fn row_of(&self, node: usize) -> Option<usize> {
        self.rows.iter().position(|r| r.node == node)
    }

    /// The node under a point on the canvas.
    pub fn node_at(&self, x: u16, y: u16) -> Option<usize> {
        match self.mode {
            Mode::Wide => self
                .placed
                .iter()
                .find(|p| p.rect.contains(ratatui::layout::Position::new(x, y)))
                .map(|p| p.node),
            Mode::Narrow => self.rows.get(usize::from(y)).map(|r| r.node),
        }
    }
}

/// How tall a node's box is.
pub fn height_of(kind: NodeKind) -> u16 {
    match kind {
        NodeKind::Folder | NodeKind::More => FOLDER_H,
        _ => NODE_H,
    }
}

/// The row of a box a line joins it at: the middle.
fn mid(rect: Rect) -> u16 {
    rect.y + rect.height / 2
}

/// Whether a node is part of the hardware (everything but the folders unfolded below a
/// filesystem).
fn is_hardware(kind: NodeKind) -> bool {
    !matches!(kind, NodeKind::Folder | NodeKind::More)
}

/// The nodes drawn under `at`: those whose first parent it is. The rest reach them by a note.
fn primary_children(t: &Topology, at: usize) -> Vec<usize> {
    t.nodes[at]
        .children
        .iter()
        .copied()
        .filter(|&c| t.nodes[c].parents.first() == Some(&at))
        .collect()
}

/// Lays the topology out for a window `width` cells wide: in columns if they fit, else as a tree.
pub fn layout(t: &Topology, width: u16) -> Layout {
    let wide = wide(t);
    // Folders unfolded to the right may run past the window, which scrolls sideways to follow the
    // chosen box; it is the hardware that has to fit, or the tree is clearer.
    let hardware = wide
        .placed
        .iter()
        .filter(|p| is_hardware(t.nodes[p.node].kind))
        .map(|p| p.rect.right())
        .max()
        .unwrap_or(0);
    if hardware <= width { wide } else { narrow(t) }
}

fn wide(t: &Topology) -> Layout {
    let depths = t.depths();
    let mut placed: Vec<Placed> = Vec::new();
    let mut links = Vec::new();
    let mut cursor = 0u16;
    let mut seen = vec![false; t.nodes.len()];

    fn place(
        t: &Topology,
        at: usize,
        depths: &[usize],
        cursor: &mut u16,
        placed: &mut Vec<Placed>,
        links: &mut Vec<(usize, usize)>,
        seen: &mut [bool],
    ) -> u16 {
        seen[at] = true;
        let height = height_of(t.nodes[at].kind);
        let start = *cursor;
        let kids: Vec<usize> = primary_children(t, at)
            .into_iter()
            .filter(|&k| !seen[k])
            .collect();
        let y = if kids.is_empty() {
            let y = *cursor;
            *cursor += height + ROW_GAP;
            y
        } else {
            let mids: Vec<u16> = kids
                .iter()
                .map(|&k| {
                    links.push((at, k));
                    let kid_height = height_of(t.nodes[k].kind);
                    place(t, k, depths, cursor, placed, links, seen) + kid_height / 2
                })
                .collect();
            // Centred on its children, but never above where its own family begins.
            let centre = (mids[0] + mids[mids.len() - 1]) / 2;
            let y = centre.saturating_sub(height / 2).max(start);
            // A box taller than what is under it still needs its own room.
            *cursor = (*cursor).max(y + height + ROW_GAP);
            y
        };
        let x = u16::try_from(depths[at])
            .unwrap_or(u16::MAX)
            .saturating_mul(NODE_W + GAP);
        placed.push(Placed {
            node: at,
            rect: Rect::new(x, y, NODE_W, height),
        });
        y
    }

    for root in t.roots() {
        if !seen[root] {
            place(
                t,
                root,
                &depths,
                &mut cursor,
                &mut placed,
                &mut links,
                &mut seen,
            );
            // A little more room between one root's family and the next.
            cursor += 1;
        }
    }
    let width = placed.iter().map(|p| p.rect.right()).max().unwrap_or(0);
    let height = placed.iter().map(|p| p.rect.bottom()).max().unwrap_or(0);
    // Top to bottom, so moving through the nodes follows the page.
    placed.sort_by_key(|p| (p.rect.y, p.rect.x));
    Layout {
        mode: Mode::Wide,
        placed,
        rows: Vec::new(),
        size: (width, height),
        links,
    }
}

/// The state of a walk down the tree for the narrow layout.
struct Tree<'a> {
    t: &'a Topology,
    rows: Vec<Row>,
    links: Vec<(usize, usize)>,
    seen: Vec<bool>,
    /// For each ancestor of the node being placed, whether it has later siblings, which decides
    /// if its line still runs down this far.
    trail: Vec<bool>,
}

impl Tree<'_> {
    fn walk(&mut self, at: usize, last: bool, top: bool) {
        self.seen[at] = true;
        let mut prefix = String::new();
        for &more in &self.trail {
            prefix.push_str(if more { "│  " } else { "   " });
        }
        if !top {
            prefix.push_str(if last { "└─ " } else { "├─ " });
        }
        self.rows.push(Row { node: at, prefix });
        let kids: Vec<usize> = primary_children(self.t, at)
            .into_iter()
            .filter(|&k| !self.seen[k])
            .collect();
        if !top {
            self.trail.push(!last);
        }
        for (i, &kid) in kids.iter().enumerate() {
            self.links.push((at, kid));
            self.walk(kid, i + 1 == kids.len(), false);
        }
        if !top {
            self.trail.pop();
        }
    }
}

fn narrow(t: &Topology) -> Layout {
    let mut tree = Tree {
        t,
        rows: Vec::new(),
        links: Vec::new(),
        seen: vec![false; t.nodes.len()],
        trail: Vec::new(),
    };
    for root in t.roots() {
        if !tree.seen[root] {
            tree.walk(root, true, true);
        }
    }
    let height = u16::try_from(tree.rows.len()).unwrap_or(u16::MAX);
    Layout {
        mode: Mode::Narrow,
        placed: Vec::new(),
        rows: tree.rows,
        size: (0, height),
        links: tree.links,
    }
}

const N: u8 = 1;
const E: u8 = 2;
const S: u8 = 4;
const W: u8 = 8;

/// The box-drawing character for the lines that meet in a cell.
fn glyph(bits: u8) -> char {
    match bits {
        b if b == N | S => '│',
        b if b == E | W => '─',
        b if b == N | E => '└',
        b if b == N | W => '┘',
        b if b == S | E => '┌',
        b if b == S | W => '┐',
        b if b == N | S | E => '├',
        b if b == N | S | W => '┤',
        b if b == E | W | S => '┬',
        b if b == E | W | N => '┴',
        b if b == N | E | S | W => '┼',
        N => '╵',
        S => '╷',
        E => '╶',
        W => '╴',
        _ => ' ',
    }
}

/// Marks a straight run from `(x0, y0)` to `(x1, y1)` (either the same row or the same column)
/// as lines, each end only reaching toward the other.
fn run(cells: &mut HashMap<(u16, u16), u8>, (x0, y0): (u16, u16), (x1, y1): (u16, u16)) {
    if y0 == y1 {
        let (a, b) = (x0.min(x1), x0.max(x1));
        for x in a..=b {
            let mut bits = 0;
            if x > a {
                bits |= W;
            }
            if x < b {
                bits |= E;
            }
            *cells.entry((x, y0)).or_insert(0) |= bits;
        }
    } else if x0 == x1 {
        let (a, b) = (y0.min(y1), y0.max(y1));
        for y in a..=b {
            let mut bits = 0;
            if y > a {
                bits |= N;
            }
            if y < b {
                bits |= S;
            }
            *cells.entry((x0, y)).or_insert(0) |= bits;
        }
    }
}

/// Where a parent's line runs out of its box, down a shared trunk and into each child.
fn trunk_x(parent: &Placed) -> u16 {
    parent.rect.right() + GAP / 2 - 1
}

/// The lines between the boxes of a wide layout, a character per canvas cell.
pub fn connectors(layout: &Layout) -> HashMap<(u16, u16), char> {
    let mut cells: HashMap<(u16, u16), u8> = HashMap::new();
    let mut by_parent: HashMap<usize, Vec<usize>> = HashMap::new();
    for &(parent, child) in &layout.links {
        by_parent.entry(parent).or_default().push(child);
    }
    for (parent, kids) in by_parent {
        let Some(from) = layout.placed.iter().find(|p| p.node == parent) else {
            continue;
        };
        let start = (from.rect.right(), mid(from.rect));
        let trunk = trunk_x(from);
        run(&mut cells, start, (trunk, start.1));
        // The line starts against the box's side, so it reaches into it.
        *cells.entry(start).or_insert(0) |= W;
        let mut ys = vec![start.1];
        for kid in kids {
            let Some(to) = layout.placed.iter().find(|p| p.node == kid) else {
                continue;
            };
            let y = mid(to.rect);
            ys.push(y);
            run(&mut cells, (trunk, y), (to.rect.x - 1, y));
            // And ends against the child's side.
            *cells.entry((to.rect.x - 1, y)).or_insert(0) |= E;
        }
        let (top, bottom) = (
            *ys.iter().min().unwrap_or(&start.1),
            *ys.iter().max().unwrap_or(&start.1),
        );
        run(&mut cells, (trunk, top), (trunk, bottom));
    }
    cells
        .into_iter()
        .map(|(at, bits)| (at, glyph(bits)))
        .collect()
}

/// The cells of the last stretch of the line into `child`, left to right, which activity moves
/// along.
pub fn path_into(layout: &Layout, parent: usize, child: usize) -> Vec<(u16, u16)> {
    let (Some(from), Some(to)) = (
        layout.placed.iter().find(|p| p.node == parent),
        layout.placed.iter().find(|p| p.node == child),
    ) else {
        return Vec::new();
    };
    let y = mid(to.rect);
    (trunk_x(from)..to.rect.x).map(|x| (x, y)).collect()
}

#[cfg(test)]
mod tests {
    use devices::topology::{Layer, Node, NodeKind};

    use super::*;

    /// bus -> disk -> {boot, pv -> lvm -> fs}, as on a real laptop.
    fn laptop() -> (Topology, [usize; 6]) {
        let mut t = Topology::default();
        let bus = t.add(Node::new("PCIe 02:00.0", NodeKind::Bus));
        let disk = t.add(Node::new("nvme0n1", NodeKind::Disk));
        let boot = t.add(Node::new("nvme0n1p1", NodeKind::Partition));
        let pv = t.add(Node::new("nvme0n1p2", NodeKind::Partition));
        let lvm = t.add(Node::new("dm-0", NodeKind::Layer(Layer::Lvm)));
        let fs = t.add(Node::new("dm-1", NodeKind::Layer(Layer::Crypt)));
        for (a, b) in [(bus, disk), (disk, boot), (disk, pv), (pv, lvm), (lvm, fs)] {
            t.link(a, b);
        }
        (t, [bus, disk, boot, pv, lvm, fs])
    }

    #[test]
    fn the_columns_are_the_layers_left_to_right_and_a_parent_is_centred_on_its_children() {
        let (t, [bus, disk, boot, pv, lvm, fs]) = laptop();
        let l = wide(&t);
        assert_eq!(l.mode, Mode::Wide);
        let col = |n: usize| l.rect_of(n).unwrap().x / (NODE_W + GAP);
        assert_eq!([bus, disk, boot, pv, lvm, fs].map(col), [0, 1, 2, 2, 3, 4]);
        let y = |n: usize| l.rect_of(n).unwrap().y;
        assert!(y(boot) < y(pv), "siblings in order, top to bottom");
        assert_eq!(
            y(disk),
            (y(boot) + y(pv)) / 2,
            "the disk sits between its partitions"
        );
        assert_eq!(y(bus), y(disk));
        assert_eq!([y(pv), y(lvm)], [y(lvm), y(fs)], "a chain stays level");
    }

    #[test]
    fn boxes_never_overlap_and_the_canvas_holds_them_all() {
        let (t, _) = laptop();
        let l = wide(&t);
        for (i, a) in l.placed.iter().enumerate() {
            assert!(a.rect.right() <= l.size.0 && a.rect.bottom() <= l.size.1);
            for b in &l.placed[i + 1..] {
                assert!(!a.rect.intersects(b.rect), "{:?} and {:?}", a, b);
            }
        }
        assert_eq!(l.placed.len(), t.nodes.len());
    }

    #[test]
    fn a_window_too_narrow_for_the_columns_gets_the_tree() {
        let (t, _) = laptop();
        let needed = wide(&t).size.0;
        assert_eq!(layout(&t, needed).mode, Mode::Wide);
        assert_eq!(layout(&t, needed - 1).mode, Mode::Narrow);
        assert_eq!(layout(&t, 0).mode, Mode::Narrow);
    }

    #[test]
    fn the_narrow_tree_draws_branches_the_way_a_directory_listing_does() {
        let (t, [bus, disk, boot, pv, lvm, fs]) = laptop();
        let l = narrow(&t);
        assert_eq!(l.mode, Mode::Narrow);
        let prefixes: Vec<(&usize, &str)> = l
            .rows
            .iter()
            .map(|r| (&r.node, r.prefix.as_str()))
            .collect();
        assert_eq!(
            prefixes,
            [
                (&bus, ""),
                (&disk, "└─ "),
                (&boot, "   ├─ "),
                (&pv, "   └─ "),
                (&lvm, "      └─ "),
                (&fs, "         └─ "),
            ]
        );
    }

    #[test]
    fn a_line_with_later_siblings_keeps_running_past_a_deeper_branch() {
        let mut t = Topology::default();
        let disk = t.add(Node::new("sda", NodeKind::Disk));
        let first = t.add(Node::new("sda1", NodeKind::Partition));
        let second = t.add(Node::new("sda2", NodeKind::Partition));
        let under = t.add(Node::new("dm-0", NodeKind::Layer(Layer::Crypt)));
        t.link(disk, first);
        t.link(disk, second);
        t.link(first, under);
        let rows = narrow(&t).rows;
        assert_eq!(rows[2].node, under);
        assert_eq!(
            rows[2].prefix, "│  └─ ",
            "sda2 still follows, so the trunk continues"
        );
        assert_eq!(rows[3].prefix, "└─ ");
    }

    #[test]
    fn a_node_with_two_parents_is_drawn_once_under_the_first() {
        let mut t = Topology::default();
        let a = t.add(Node::new("sda1", NodeKind::Partition));
        let b = t.add(Node::new("sdb1", NodeKind::Partition));
        let md = t.add(Node::new("md0", NodeKind::Layer(Layer::Raid)));
        t.link(a, md);
        t.link(b, md);
        for l in [wide(&t), narrow(&t)] {
            let count = match l.mode {
                Mode::Wide => l.placed.iter().filter(|p| p.node == md).count(),
                Mode::Narrow => l.rows.iter().filter(|r| r.node == md).count(),
            };
            assert_eq!(count, 1, "{:?}", l.mode);
            assert_eq!(l.links.iter().filter(|(_, c)| *c == md).count(), 1);
        }
    }

    #[test]
    fn lines_join_a_parent_to_each_child_through_a_shared_trunk() {
        let (t, [_, disk, boot, pv, ..]) = laptop();
        let l = wide(&t);
        let cells = connectors(&l);
        let (d, b, p) = (
            l.rect_of(disk).unwrap(),
            l.rect_of(boot).unwrap(),
            l.rect_of(pv).unwrap(),
        );
        let trunk = d.right() + GAP / 2 - 1;
        let at = |x: u16, y: u16| cells.get(&(x, y)).copied();
        assert_eq!(at(d.right(), mid(d)), Some('─'), "out of the disk");
        assert_eq!(at(trunk, mid(d)), Some('┤'), "where the trunk meets it");
        assert_eq!(
            at(trunk, mid(b)),
            Some('┌'),
            "the top of the trunk bends into the first child"
        );
        assert_eq!(at(trunk, mid(p)), Some('└'), "and the bottom into the last");
        assert_eq!(at(b.x - 1, mid(b)), Some('─'), "into the box");
        assert_eq!(at(trunk, mid(b) + 1), Some('│'), "down the trunk");
        assert!(cells.keys().all(|&(x, y)| x < l.size.0 && y < l.size.1));
    }

    #[test]
    fn a_single_child_is_a_straight_line_and_the_path_into_it_runs_left_to_right() {
        let (t, [_, _, _, pv, lvm, _]) = laptop();
        let l = wide(&t);
        let cells = connectors(&l);
        let (a, b) = (l.rect_of(pv).unwrap(), l.rect_of(lvm).unwrap());
        let y = mid(a);
        for x in a.right()..b.x {
            assert_eq!(cells.get(&(x, y)), Some(&'─'), "x = {x}");
        }
        let path = path_into(&l, pv, lvm);
        assert_eq!(path.first(), Some(&(a.right() + GAP / 2 - 1, y)));
        assert_eq!(path.last(), Some(&(b.x - 1, y)));
        assert!(path.windows(2).all(|w| w[1].0 == w[0].0 + 1 && w[1].1 == y));
    }

    #[test]
    fn a_click_finds_the_node_in_either_layout() {
        let (t, [bus, _, boot, ..]) = laptop();
        let w = wide(&t);
        let r = w.rect_of(boot).unwrap();
        assert_eq!(w.node_at(r.x + 2, r.y + 2), Some(boot));
        assert_eq!(w.node_at(r.x + 2, r.bottom()), None, "the gap under a box");
        assert_eq!(
            w.node_at(w.rect_of(bus).unwrap().x, w.rect_of(bus).unwrap().y),
            Some(bus)
        );
        let n = narrow(&t);
        assert_eq!(n.node_at(0, 0), Some(bus));
        assert_eq!(n.node_at(0, 99), None);
        assert_eq!(n.row_of(boot), Some(2));
    }

    #[test]
    fn several_separate_machines_of_storage_stack_down_the_page_without_crossing() {
        let mut t = Topology::default();
        for i in 0..4 {
            let d = t.add(Node::new(&format!("sd{i}"), NodeKind::Disk));
            let p = t.add(Node::new(&format!("sd{i}1"), NodeKind::Partition));
            t.link(d, p);
        }
        let l = wide(&t);
        let mut ys: Vec<u16> = l
            .placed
            .iter()
            .filter(|p| p.rect.x == 0)
            .map(|p| p.rect.y)
            .collect();
        ys.sort_unstable();
        ys.dedup();
        assert_eq!(ys.len(), 4);
        assert!(ys.windows(2).all(|w| w[1] >= w[0] + NODE_H));
    }

    /// A filesystem (a tall box) with `n` folders (short ones) under it, and a second filesystem.
    fn with_folders(n: usize) -> (Topology, usize, usize) {
        let mut t = Topology::default();
        let fs = t.add(Node::new("sda1", NodeKind::Partition));
        for i in 0..n {
            let f = t.add(Node::new(&format!("/d{i}"), NodeKind::Folder));
            t.link(fs, f);
        }
        let other = t.add(Node::new("sdb1", NodeKind::Partition));
        (t, fs, other)
    }

    #[test]
    fn folders_are_short_boxes_in_the_column_after_their_filesystem() {
        let (t, fs, _) = with_folders(3);
        let l = wide(&t);
        let fs_rect = l.rect_of(fs).unwrap();
        assert_eq!(fs_rect.height, NODE_H);
        for p in l
            .placed
            .iter()
            .filter(|p| t.nodes[p.node].kind == NodeKind::Folder)
        {
            assert_eq!(p.rect.height, FOLDER_H);
            assert_eq!(
                p.rect.x,
                fs_rect.x + NODE_W + GAP,
                "one column to the right"
            );
        }
    }

    #[test]
    fn a_tall_box_over_one_short_one_still_gets_its_own_room_and_nothing_overlaps() {
        for n in [0, 1, 2, 5] {
            let (t, ..) = with_folders(n);
            let l = wide(&t);
            for (i, a) in l.placed.iter().enumerate() {
                for b in &l.placed[i + 1..] {
                    assert!(!a.rect.intersects(b.rect), "n = {n}: {a:?} and {b:?}");
                }
            }
            assert_eq!(l.placed.len(), t.nodes.len());
        }
        let (t, fs, other) = with_folders(1);
        let l = wide(&t);
        assert!(
            l.rect_of(other).unwrap().y >= l.rect_of(fs).unwrap().bottom(),
            "the next filesystem starts below the tall box, not under its single folder"
        );
    }

    #[test]
    fn a_line_joins_boxes_of_different_heights_at_their_middles() {
        let (t, fs, _) = with_folders(1);
        let l = wide(&t);
        let folder = l
            .placed
            .iter()
            .find(|p| p.node != fs && t.nodes[p.node].kind == NodeKind::Folder)
            .unwrap();
        let cells = connectors(&l);
        let y = folder.rect.y + FOLDER_H / 2;
        assert_eq!(
            cells.get(&(folder.rect.x - 1, y)),
            Some(&'─'),
            "into the middle of the short box"
        );
        let path = path_into(&l, fs, folder.node);
        assert!(path.iter().all(|&(_, py)| py == y));
    }

    #[test]
    fn folders_running_out_past_the_window_do_not_turn_the_diagram_into_a_tree() {
        let (mut t, fs, _) = with_folders(1);
        let hardware = layout(&t, 0);
        assert_eq!(hardware.mode, Mode::Narrow, "no width at all");
        let width = wide(&t).size.0;
        // Unfold a chain of ten folders, far wider than the window.
        let mut parent = fs;
        for i in 0..10 {
            let f = t.add(Node::new(&format!("/c{i}"), NodeKind::Folder));
            t.link(parent, f);
            parent = f;
        }
        let l = layout(&t, width);
        assert_eq!(l.mode, Mode::Wide, "the hardware still fits");
        assert!(
            l.size.0 > width,
            "and the canvas is wider than the window: it scrolls sideways"
        );
        let hardware_only = Topology {
            nodes: t.nodes[..2].to_vec(),
        };
        let _ = hardware_only;
    }

    #[test]
    fn nothing_to_draw_is_an_empty_layout_in_either_mode() {
        let t = Topology::default();
        let l = layout(&t, 100);
        assert_eq!(l.size, (0, 0));
        assert!(l.placed.is_empty() && l.rows.is_empty() && l.links.is_empty());
        assert!(connectors(&l).is_empty());
        assert_eq!(narrow(&t).rows.len(), 0);
    }

    #[test]
    fn every_combination_of_lines_has_a_character() {
        for bits in 0..16u8 {
            let c = glyph(bits);
            assert_eq!(
                c == ' ',
                bits == 0 || bits.count_ones() == 0,
                "{bits:04b} -> {c:?}"
            );
        }
        assert_eq!(glyph(N | E | S | W), '┼');
    }
}
