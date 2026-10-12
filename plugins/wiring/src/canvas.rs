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

//! The wiring diagram as a canvas: the picture the host draws, built from the state and the size
//! of the view. Everything is said in meaning (a tone, a mark, a path that may flow) and never in
//! color or symbol, so the picture follows the user's theme and glyph set; the one cost is that
//! nothing here can use a non-ASCII symbol, so the icons and the fold signs are plain ASCII.
//!
//! When the window is wide enough, a box for every layer of the storage in columns with lines
//! between them, the lines carrying a moving mark where data is flowing; when it is not, the same
//! thing as an indented tree, one row a node. The node holding the folder being browsed has the
//! bright frame ("you are here"), the chosen node is filled, and while files are carried each
//! filesystem says whether they fit, as on the deck.
//!
//! The diagram can be larger than the view (unfolded folders run on past the window), so the
//! picture is the part of it in view, moved to the top left. A box cut by the window's edge is
//! left out whole, since the host drops a frame that is not entirely inside the canvas, and the
//! scroll keeps the chosen box whole. The host cleans text and refuses a canvas over its limits,
//! so the builder also keeps within them: past the item or character budget, boxes are left out.

use std::path::Path;

use plugin_api::{
    Canvas, Cells, Item, MAX_CANVAS_CHARS, MAX_CANVAS_ITEMS, MAX_CANVAS_TEXT_CHARS, Mark, Paint,
    Tone,
};
use shared::cargo_fit::{self, Verdict};
use shared::devices::topology::{Layer, Node, NodeKind};
use shared::format::format_size;

use crate::layout::{self, Layout, Mode, Rect};
use crate::state::{HINT, WiringState};

/// The parts and characters kept back for the note and the hint, so the boxes cannot crowd them
/// out.
const RESERVED_ITEMS: usize = 8;
const RESERVED_CHARS: usize = 2_000;

/// A filesystem this full (as a whole percent) is drawn in the danger tone.
const FULL_PERCENT: u8 = 90;

/// The keys the diagram wants: the arrows and their letters, the zoom, and the actions on a node.
const CLAIMS: [&str; 21] = [
    "up", "down", "left", "right", "h", "j", "k", "l", "home", "end", "pageup", "pagedown", "+",
    "=", "-", "_", "0", "enter", "m", "e", "p",
];

fn paint(tone: Tone) -> Paint {
    Paint {
        tone,
        hue: None,
        mark: Mark::None,
    }
}

fn marked(mark: Mark) -> Paint {
    Paint {
        mark,
        ..Paint::default()
    }
}

/// `text` cut to `width` characters, ending in `...` when something was dropped.
fn fit(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width <= 3 {
        return text.chars().take(width).collect();
    }
    let head: String = text.chars().take(width - 3).collect();
    format!("{head}...")
}

/// `text` cut to its last `width` characters behind `...` when it is longer, so the end of a path
/// (the part that tells folders apart) survives.
fn elide_left(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(3);
    let tail: String = text.chars().skip(count - keep).collect();
    format!("...{tail}")
}

fn icon(node: &Node) -> &'static str {
    match node.kind {
        NodeKind::Disk => "[] ",
        NodeKind::Layer(_) | NodeKind::Share => "<> ",
        NodeKind::Bus | NodeKind::Partition | NodeKind::Folder | NodeKind::More => "",
    }
}

/// What the node is, in a few words.
fn kind_text(node: &Node) -> String {
    match node.kind {
        NodeKind::Bus => "bus".to_string(),
        NodeKind::Disk => {
            let removable = if node.removable { " | removable" } else { "" };
            format!("disk{removable}")
        }
        NodeKind::Partition => node
            .fs
            .clone()
            .unwrap_or_else(|| "no filesystem".to_string()),
        NodeKind::Layer(layer) => {
            let fs = node.fs.as_deref().filter(|_| node.is_filesystem());
            match (layer, fs) {
                (
                    Layer::Raid | Layer::Lvm | Layer::Crypt | Layer::Multipath | Layer::Other,
                    Some(fs),
                ) => {
                    format!("{} | {fs}", layer.label())
                }
                _ => layer.label().to_string(),
            }
        }
        NodeKind::Share => node.fs.clone().unwrap_or_else(|| "share".to_string()),
        NodeKind::Folder => "folder".to_string(),
        NodeKind::More => String::new(),
    }
}

/// Where it is mounted, or how big it is.
fn place_text(node: &Node, width: usize) -> String {
    match (&node.mount, node.total) {
        (Some(mount), _) => elide_left(&mount.display().to_string(), width),
        (None, 0) => String::new(),
        (None, total) => format_size(total),
    }
}

fn used_percent(node: &Node) -> Option<u8> {
    if node.mount.is_none() || node.total == 0 {
        return None;
    }
    let used = node.total.saturating_sub(node.free);
    Some(u8::try_from(u128::from(used) * 100 / u128::from(node.total)).unwrap_or(100))
}

/// How fast data is moving, in a few words.
fn rate_text(read: u64, written: u64) -> String {
    if read == 0 && written == 0 {
        return "idle".to_string();
    }
    format!("r {} w {}", format_size(read), format_size(written))
}

/// The sign in front of a name that can be unfolded: `+` when folded, `-` when open.
fn fold_sign(state: &WiringState, index: usize) -> &'static str {
    if state.topology().nodes[index].folder_path().is_none() {
        return "";
    }
    if state.is_unfolded(index) { "- " } else { "+ " }
}

/// The one line inside a folder's box (or a note's): how many folders it holds, or what to do.
fn folder_line(state: &WiringState, index: usize) -> String {
    let node = &state.topology().nodes[index];
    match node.kind {
        NodeKind::More if node.title().starts_with('+') => "Enter opens it".to_string(),
        NodeKind::More => String::new(),
        _ => match state.subfolders(index) {
            None => "folder".to_string(),
            Some(0) => "no folders".to_string(),
            Some(1) => "1 folder".to_string(),
            Some(n) => format!("{n} folders"),
        },
    }
}

/// Whether the load cannot go on this node, which dims it.
fn refused(state: &WiringState, index: usize) -> bool {
    matches!(
        state.verdict(index),
        Some(Verdict::Short { .. } | Verdict::TooBig { .. })
    )
}

fn verdict_tone(verdict: &Verdict) -> Tone {
    match verdict {
        Verdict::Fits { .. } | Verdict::SameDisk => Tone::Accent,
        Verdict::Short { .. } | Verdict::TooBig { .. } => Tone::Danger,
        Verdict::Tight { .. } | Verdict::Unmounted | Verdict::Unsure => Tone::Dim,
    }
}

/// What the parts added so far cost against the limits.
struct Out {
    items: Vec<Item>,
    chars: usize,
}

impl Out {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            chars: 0,
        }
    }

    /// Whether another `items` parts of `chars` characters still fit.
    fn room(&self, items: usize, chars: usize) -> bool {
        self.items.len() + items + RESERVED_ITEMS <= MAX_CANVAS_ITEMS
            && self.chars + chars + RESERVED_CHARS <= MAX_CANVAS_CHARS
    }

    fn block(&mut self, at: Cells, paint: Paint, level: Option<u8>) {
        if at.w > 0 && at.h > 0 {
            self.items.push(Item::Block { at, paint, level });
        }
    }

    fn frame(&mut self, at: Cells, paint: Paint, title: String) {
        self.chars += title.chars().count();
        self.items.push(Item::Frame {
            at,
            paint,
            title: Some(title),
        });
    }

    fn text(&mut self, x: u16, y: u16, width: u16, text: String, tone: Tone, bold: bool) {
        if width == 0 || text.is_empty() {
            return;
        }
        let text: String = text.chars().take(MAX_CANVAS_TEXT_CHARS).collect();
        self.chars += text.chars().count();
        self.items.push(Item::Text {
            x,
            y,
            width,
            text,
            paint: paint(tone),
            bold,
        });
    }

    /// A straight line from `from` to `to` in diagram cells, moved by the scroll and cut to the
    /// window; nothing when none of it shows.
    fn line(&mut self, from: (u16, u16), to: (u16, u16), view: View, flow: bool) {
        let Some((a, b)) = view.clip(from, to) else {
            return;
        };
        self.items.push(Item::Path {
            points: vec![a, b],
            paint: Paint::default(),
            flow,
        });
    }
}

/// The part of the diagram in view: its first column and row, and how many cells it has.
#[derive(Debug, Clone, Copy)]
struct View {
    left: u16,
    top: u16,
    width: u16,
    height: u16,
}

impl View {
    /// The part of a straight line (both ends in diagram cells) that is in view, in the picture's
    /// cells, or `None` when none of it is.
    fn clip(self, from: (u16, u16), to: (u16, u16)) -> Option<((u16, u16), (u16, u16))> {
        let at = |(x, y): (u16, u16)| {
            (
                i32::from(x) - i32::from(self.left),
                i32::from(y) - i32::from(self.top),
            )
        };
        let ((x0, y0), (x1, y1)) = (at(from), at(to));
        let (w, h) = (i32::from(self.width), i32::from(self.height));
        let along = |a: i32, b: i32, limit: i32| {
            (a.max(b) >= 0 && a.min(b) < limit)
                .then(|| (a.clamp(0, limit - 1), b.clamp(0, limit - 1)))
        };
        let (a, b) = if y0 == y1 {
            if !(0..h).contains(&y0) {
                return None;
            }
            let (xa, xb) = along(x0, x1, w)?;
            ((xa, y0), (xb, y0))
        } else {
            if x0 != x1 || !(0..w).contains(&x0) {
                return None;
            }
            let (ya, yb) = along(y0, y1, h)?;
            ((x0, ya), (x0, yb))
        };
        let cell = |(x, y): (i32, i32)| Some((u16::try_from(x).ok()?, u16::try_from(y).ok()?));
        Some((cell(a)?, cell(b)?))
    }

    /// The box `rect` in the picture, when all of it is in view.
    fn whole(self, rect: Rect) -> Option<Rect> {
        let x = rect.x.checked_sub(self.left)?;
        let y = rect.y.checked_sub(self.top)?;
        (rect.width > 0
            && rect.height > 0
            && u32::from(x) + u32::from(rect.width) <= u32::from(self.width)
            && u32::from(y) + u32::from(rect.height) <= u32::from(self.height))
        .then(|| Rect::new(x, y, rect.width, rect.height))
    }
}

fn cells(rect: Rect) -> Cells {
    Cells {
        x: rect.x,
        y: rect.y,
        w: rect.width,
        h: rect.height,
    }
}

/// The wiring diagram for a view of `width` by `height` cells, with `here` (the folder the browser
/// is in) lit. Remembers in `state` where the picture starts in the diagram, for the clicks that
/// follow.
pub fn build(state: &mut WiringState, here: &Path, width: u16, height: u16) -> Canvas {
    let mut out = Out::new();
    if state.topology().nodes.is_empty() {
        let note = if state.has_scanned() {
            "no storage found"
        } else {
            "looking at how the storage is wired..."
        };
        out.text(0, 0, width, note.to_string(), Tone::Dim, false);
    } else {
        state.set_size(width, height);
        let layout = state.layout(width);
        match layout.mode {
            Mode::Wide => wide(state, &layout, here, width, height, &mut out),
            Mode::Narrow => narrow(state, &layout, here, width, height, &mut out),
        }
    }
    Canvas {
        width,
        height,
        items: out.items,
        claims: CLAIMS.iter().map(|key| (*key).to_string()).collect(),
        hint: Some(HINT.to_string()),
    }
}

/// Scrolls so the chosen box is whole in view, moving as little as possible: down the page, and
/// sideways when unfolded folders have run out past the window. Returns the new first column and
/// row.
fn scroll_wide(state: &WiringState, layout: &Layout, width: u16, height: u16) -> (u16, u16) {
    let selected_node = state.index();
    let selected = selected_node.and_then(|i| layout.rect_of(i));
    let (mut left, mut top) = state.scrolled();
    if let Some(rect) = selected {
        if rect.y < top {
            top = rect.y;
        } else if rect.bottom() > top.saturating_add(height) {
            top = rect.bottom().saturating_sub(height);
        }
        // Sideways: the chosen box, and the column of whatever is unfolded to its right if there
        // is room for both, so opening a folder shows what is in it.
        let has_children =
            selected_node.is_some_and(|n| layout.links.iter().any(|&(parent, _)| parent == n));
        let want_right = if has_children {
            rect.right() + layout.scale.gap + layout.scale.node_w
        } else {
            rect.right()
        };
        if rect.x < left {
            left = rect.x;
        } else if want_right > left.saturating_add(width) {
            left = want_right.saturating_sub(width).min(rect.x);
        }
    }
    top = top.min(layout.size.1.saturating_sub(height));
    left = left.min(layout.size.0.saturating_sub(width));
    (left, top)
}

fn wide(
    state: &mut WiringState,
    layout: &Layout,
    here: &Path,
    width: u16,
    height: u16,
    out: &mut Out,
) {
    let (left, top) = scroll_wide(state, layout, width, height);
    state.set_scrolled(left, top);
    let view = View {
        left,
        top,
        width,
        height,
    };

    // The lines first, so the boxes sit over them. They run one cell into the boxes at both ends
    // (hidden by them), which is what makes the first and last cell of a line join the trunk.
    let mut by_parent: Vec<(usize, Vec<usize>)> = Vec::new();
    for &(parent, child) in &layout.links {
        match by_parent.iter_mut().find(|(p, _)| *p == parent) {
            Some((_, kids)) => kids.push(child),
            None => by_parent.push((parent, vec![child])),
        }
    }
    for (parent, kids) in &by_parent {
        let Some(from) = layout.rect_of(*parent) else {
            continue;
        };
        let start_y = from.y + from.height / 2;
        let trunk = from.right() + (layout.scale.gap / 2).max(1) - 1;
        out.line((from.right() - 1, start_y), (trunk, start_y), view, false);
        let mut ys = vec![start_y];
        for &kid in kids {
            let Some(to) = layout.rect_of(kid) else {
                continue;
            };
            let y = to.y + to.height / 2;
            ys.push(y);
            out.line((trunk, y), (to.x, y), view, false);
        }
        let top_y = ys.iter().copied().min().unwrap_or(start_y);
        let bottom_y = ys.iter().copied().max().unwrap_or(start_y);
        if top_y != bottom_y {
            out.line((trunk, top_y), (trunk, bottom_y), view, false);
        }
        // A mark travels along the line into each node that is busy.
        for &kid in kids {
            let rates = state.rates(kid);
            let Some(to) = layout.rect_of(kid) else {
                continue;
            };
            if (rates.read == 0 && rates.written == 0) || to.x == 0 {
                continue;
            }
            let y = to.y + to.height / 2;
            out.line((trunk, y), (to.x - 1, y), view, true);
        }
    }

    let here = state.here(here);
    for placed in &layout.placed {
        let Some(rect) = view.whole(placed.rect) else {
            continue;
        };
        if !out.room(8, 400) {
            break;
        }
        box_parts(state, placed.node, rect, here, layout, out);
    }
}

/// The parts of one node's box, drawn at `rect` in the picture.
fn box_parts(
    state: &WiringState,
    index: usize,
    rect: Rect,
    here: Option<usize>,
    layout: &Layout,
    out: &mut Out,
) {
    let node = &state.topology().nodes[index];
    let chosen = state.index() == Some(index);
    let lit = here == Some(index);
    // A mount or an eject started from the chosen node is still running.
    let working = if chosen && state.is_working() {
        " ..."
    } else {
        ""
    };
    let name = format!(
        "{}{}{}{working}",
        fold_sign(state, index),
        icon(node),
        node.title()
    );
    if chosen {
        out.block(cells(rect), marked(Mark::Chosen), None);
    }
    if !layout.scale.bordered {
        // The smallest boxes are one line of text, the frame told by bold type.
        let tone = if lit || chosen {
            Tone::Accent
        } else if refused(state, index) {
            Tone::Dim
        } else {
            Tone::Normal
        };
        out.text(
            rect.x,
            rect.y,
            rect.width,
            fit(&name, usize::from(rect.width)),
            tone,
            lit,
        );
        return;
    }
    let frame = if lit {
        marked(Mark::Here)
    } else if refused(state, index) {
        paint(Tone::Dim)
    } else {
        Paint::default()
    };
    out.frame(
        cells(rect),
        frame,
        fit(&name, usize::from(layout.scale.node_w.saturating_sub(4))),
    );
    let inner = Rect::new(
        rect.x + 1,
        rect.y + 1,
        rect.width.saturating_sub(2),
        rect.height.saturating_sub(2),
    );
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    box_lines(state, index, inner, out);
}

/// The lines inside a node's box, as many as fit.
fn box_lines(state: &WiringState, index: usize, inner: Rect, out: &mut Out) {
    let node = &state.topology().nodes[index];
    let room = usize::from(inner.width);
    let line = |out: &mut Out, row: u16, text: String, tone: Tone| {
        if row < inner.height {
            out.text(
                inner.x,
                inner.y + row,
                inner.width,
                fit(&text, room),
                tone,
                false,
            );
        }
    };
    if matches!(node.kind, NodeKind::Folder | NodeKind::More) {
        line(out, 0, folder_line(state, index), Tone::Dim);
        return;
    }
    line(out, 0, kind_text(node), Tone::Dim);
    line(out, 1, place_text(node, room), Tone::Dim);
    if inner.height < 3 {
        return;
    }
    let rates = state.rates(index);
    if let Some(verdict) = state.verdict(index) {
        line(
            out,
            2,
            cargo_fit::card_text(&verdict, true),
            verdict_tone(&verdict),
        );
    } else if let Some(percent) = used_percent(node) {
        gauge(out, inner, 2, percent);
    } else if node.kind == NodeKind::Bus || node.kind == NodeKind::Disk || node.is_filesystem() {
        let busy = rates.read > 0 || rates.written > 0;
        line(
            out,
            2,
            rate_text(rates.read, rates.written),
            if busy { Tone::Accent } else { Tone::Dim },
        );
    }
}

/// How full the filesystem is: a bar across the box, the part in use bright and the rest dimmed,
/// then the percentage.
fn gauge(out: &mut Out, inner: Rect, row: u16, percent: u8) {
    let tail = format!(" {percent:>3}%");
    let bar = usize::from(inner.width).saturating_sub(tail.chars().count());
    let y = inner.y + row;
    if bar > 0 {
        let bar = u16::try_from(bar).unwrap_or(inner.width);
        let filled = u16::try_from((u32::from(bar) * u32::from(percent.min(100)) + 50) / 100)
            .unwrap_or(bar)
            .min(bar);
        let tone = if percent >= FULL_PERCENT {
            Tone::Danger
        } else {
            Tone::Accent
        };
        out.block(
            Cells {
                x: inner.x,
                y,
                w: filled,
                h: 1,
            },
            paint(tone),
            None,
        );
        // A block at level 0 is all dimmed: the room left in the filesystem.
        out.block(
            Cells {
                x: inner.x + filled,
                y,
                w: bar - filled,
                h: 1,
            },
            paint(tone),
            Some(0),
        );
    }
    let used = u16::try_from(tail.chars().count()).unwrap_or(0);
    out.text(
        inner.x + inner.width.saturating_sub(used),
        y,
        used,
        tail,
        Tone::Normal,
        false,
    );
}

fn narrow(
    state: &mut WiringState,
    layout: &Layout,
    here: &Path,
    width: u16,
    height: u16,
    out: &mut Out,
) {
    let selected_row = state.index().and_then(|i| layout.row_of(i)).unwrap_or(0);
    let rows_in_view = usize::from(height);
    let (left, top) = state.scrolled();
    let mut top = usize::from(top);
    if selected_row < top {
        top = selected_row;
    } else if selected_row >= top + rows_in_view {
        top = selected_row + 1 - rows_in_view;
    }
    top = top.min(layout.rows.len().saturating_sub(rows_in_view));
    state.set_scrolled(left, u16::try_from(top).unwrap_or(u16::MAX));

    let here = state.here(here);
    let room = usize::from(width);
    // Deep folders are indented a step for every level, so the name column grows to hold the
    // longest row (up to three fifths of the width) instead of cutting the deepest names short.
    let name_of = |row: &layout::Row| {
        let node = &state.topology().nodes[row.node];
        format!(
            "{}{}{}{}",
            ascii_prefix(&row.prefix),
            fold_sign(state, row.node),
            icon(node),
            node.title()
        )
    };
    let longest = layout
        .rows
        .iter()
        .map(|row| name_of(row).chars().count())
        .max()
        .unwrap_or(0);
    let name_width = longest
        .clamp(18, (room * 3 / 5).max(40))
        .min(room.saturating_sub(2));

    for (y, row) in layout.rows.iter().enumerate().skip(top).take(rows_in_view) {
        if !out.room(6, 300) {
            break;
        }
        let y = u16::try_from(y - top).unwrap_or(u16::MAX);
        let node = &state.topology().nodes[row.node];
        if state.index() == Some(row.node) {
            out.block(
                Cells {
                    x: 0,
                    y,
                    w: width,
                    h: 1,
                },
                marked(Mark::Chosen),
                None,
            );
        }
        if here == Some(row.node) {
            out.text(0, y, 1, "|".to_string(), Tone::Accent, true);
        }
        let name_cells = u16::try_from(name_width).unwrap_or(width);
        out.text(
            2,
            y,
            name_cells,
            fit(&name_of(row), name_width),
            if refused(state, row.node) {
                Tone::Dim
            } else {
                Tone::Normal
            },
            false,
        );
        let rates = state.rates(row.node);
        let busy = rates.read > 0 || rates.written > 0;
        let mut detail = vec![match node.kind {
            NodeKind::Folder => state
                .subfolders(row.node)
                .map_or_else(String::new, |n| format!("{n} folders")),
            NodeKind::More => String::new(),
            _ => kind_text(node),
        }];
        detail.retain(|d| !d.is_empty());
        let place = place_text(node, 24);
        if !place.is_empty() {
            detail.push(place);
        }
        if let Some(p) = used_percent(node) {
            detail.push(format!("{p}%"));
        }
        let verdict = state.verdict(row.node);
        let (tail, tail_tone) = match &verdict {
            Some(v) => (cargo_fit::card_text(v, true), verdict_tone(v)),
            None if busy || node.kind == NodeKind::Bus => (
                rate_text(rates.read, rates.written),
                if busy { Tone::Accent } else { Tone::Dim },
            ),
            None => (String::new(), Tone::Dim),
        };
        let rest = room.saturating_sub(2 + name_width + 1);
        let tail_width = tail.chars().count();
        let details_room = rest.saturating_sub(tail_width + 2);
        let x = 2 + name_width + 1;
        out.text(
            u16::try_from(x).unwrap_or(width),
            y,
            u16::try_from(details_room).unwrap_or(width),
            fit(&detail.join("  "), details_room),
            Tone::Dim,
            false,
        );
        if tail_width > 0 && rest >= tail_width + 2 {
            out.text(
                u16::try_from(x + details_room + 2).unwrap_or(width),
                y,
                u16::try_from(tail_width).unwrap_or(width),
                tail,
                tail_tone,
                false,
            );
        }
    }
}

/// The tree's branch drawing in ASCII, since a plugin names no glyphs.
fn ascii_prefix(prefix: &str) -> String {
    prefix
        .chars()
        .map(|c| match c {
            '│' => '|',
            '├' | '└' => '+',
            '─' => '-',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use plugin_api::{SetCanvas, Surface};
    use proptest::prelude::*;
    use shared::cargo_fit::{Carry, Load};
    use shared::devices::activity::Counters;
    use shared::devices::topology::Topology;

    use super::*;
    use crate::layout::Zoom;

    /// bus -> nvme -> {boot (/boot), pv -> lvm -> root (/, 94% full)}.
    fn machine() -> Topology {
        let mut t = Topology::default();
        let mut node = |id: &str,
                        kind: NodeKind,
                        mount: Option<&str>,
                        fs: Option<&str>,
                        name: Option<&str>| {
            let mut n = Node::new(id, kind);
            n.mount = mount.map(PathBuf::from);
            n.fs = fs.map(str::to_string);
            n.usage = fs
                .filter(|f| !f.contains("member") && !f.contains("crypto"))
                .map(|_| "filesystem".to_string());
            n.name = name.map(str::to_string);
            // About a terabyte each, the root nearly full: tiny sizes would all be "tight".
            n.total = 1000 << 30;
            n.free = if id == "dm-1" { 60 << 30 } else { 600 << 30 };
            t.add(n)
        };
        let bus = node("PCIe 02:00.0", NodeKind::Bus, None, None, None);
        let disk = node("nvme0n1", NodeKind::Disk, None, None, None);
        let boot = node(
            "nvme0n1p1",
            NodeKind::Partition,
            Some("/boot"),
            Some("vfat"),
            None,
        );
        let pv = node(
            "nvme0n1p2",
            NodeKind::Partition,
            None,
            Some("LVM2_member"),
            None,
        );
        let lvm = node(
            "dm-0",
            NodeKind::Layer(Layer::Lvm),
            None,
            Some("crypto_LUKS"),
            Some("Vg-root"),
        );
        let root = node(
            "dm-1",
            NodeKind::Layer(Layer::Crypt),
            Some("/"),
            Some("btrfs"),
            Some("root"),
        );
        for (p, c) in [
            (bus, disk),
            (disk, boot),
            (disk, pv),
            (pv, lvm),
            (lvm, root),
        ] {
            t.link(p, c);
        }
        t
    }

    fn state() -> WiringState {
        WiringState::with_topology(machine())
    }

    fn picture(s: &mut WiringState, width: u16, height: u16) -> Canvas {
        build(s, Path::new("/elsewhere"), width, height)
    }

    fn texts(canvas: &Canvas) -> Vec<&str> {
        canvas
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn titles(canvas: &Canvas) -> Vec<&str> {
        canvas
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Frame { title, .. } => title.as_deref(),
                _ => None,
            })
            .collect()
    }

    fn checked(canvas: Canvas) -> Canvas {
        SetCanvas {
            surface: Surface::Wiring,
            canvas: Some(canvas),
        }
        .checked()
        .expect("the host accepts it")
        .canvas
        .expect("a canvas")
    }

    #[test]
    fn a_roomy_view_is_a_framed_box_for_every_node_and_the_host_accepts_it() {
        let mut s = state();
        let canvas = checked(picture(&mut s, 200, 40));
        let titles = titles(&canvas);
        for want in ["PCIe 02:00.0", "nvme0n1", "nvme0n1p1", "root"] {
            assert!(
                titles.iter().any(|t| t.contains(want)),
                "{want} in {titles:?}"
            );
        }
        assert_eq!(titles.len(), 6);
        assert!(canvas.claims.iter().any(|k| k == "enter"));
        assert!(
            canvas
                .hint
                .as_deref()
                .is_some_and(|h| h.starts_with("wiring:"))
        );
    }

    #[test]
    fn the_chosen_node_is_filled_and_the_one_holding_the_folder_browsed_is_framed_bright() {
        let mut s = state();
        s.select_for_test("nvme0n1p1");
        let canvas = build(&mut s, Path::new("/boot/grub"), 200, 40);
        let chosen = canvas
            .items
            .iter()
            .filter(|i| matches!(i, Item::Block { paint, .. } if paint.mark == Mark::Chosen))
            .count();
        assert_eq!(chosen, 1);
        let lit: Vec<_> = canvas
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Frame { paint, title, .. } if paint.mark == Mark::Here => title.as_deref(),
                _ => None,
            })
            .collect();
        assert_eq!(lit.len(), 1, "{lit:?}");
        assert!(lit[0].contains("nvme0n1p1"));
    }

    #[test]
    fn a_nearly_full_filesystem_has_a_gauge_in_the_danger_tone_and_says_how_full() {
        let mut s = state();
        let canvas = picture(&mut s, 200, 40);
        assert!(texts(&canvas).iter().any(|t| t.contains("94%")));
        let danger = canvas.items.iter().any(|item| {
            matches!(item, Item::Block { paint, level: None, .. } if paint.tone == Tone::Danger)
        });
        assert!(danger, "the part in use is the danger tone");
        let rest = canvas.items.iter().any(|item| {
            matches!(item, Item::Block { paint, level: Some(0), .. } if paint.tone == Tone::Danger)
        });
        assert!(rest, "and the room left is the same tone dimmed");
    }

    #[test]
    fn a_load_that_does_not_fit_is_said_in_the_danger_tone_and_dims_the_node() {
        let mut s = state();
        s.set_load(Some(Load {
            bytes: 900 << 30,
            exact: true,
            largest: 1,
            carry: Carry::Copy,
            source: "/home/me".into(),
        }));
        let canvas = picture(&mut s, 200, 40);
        let said = canvas.items.iter().any(|item| {
            matches!(item, Item::Text { text, paint, .. }
                if text.contains("needs") && paint.tone == Tone::Danger)
        });
        assert!(said, "{:?}", texts(&canvas));
        assert!(
            canvas
                .items
                .iter()
                .any(|item| matches!(item, Item::Frame { paint, .. } if paint.tone == Tone::Dim))
        );
    }

    #[test]
    fn a_busy_disk_gets_a_line_with_a_mark_travelling_along_it_and_an_idle_one_does_not() {
        let mut s = state();
        let flows = |canvas: &Canvas| {
            canvas
                .items
                .iter()
                .filter(|i| matches!(i, Item::Path { flow: true, .. }))
                .count()
        };
        assert_eq!(flows(&picture(&mut s, 200, 40)), 0);
        let mut counters = HashMap::new();
        counters.insert(
            "nvme0n1".to_string(),
            Counters {
                read: 0,
                written: 0,
            },
        );
        s.activity_update(&counters);
        counters.insert(
            "nvme0n1".to_string(),
            Counters {
                read: 5000,
                written: 0,
            },
        );
        s.activity_update(&counters);
        assert_eq!(flows(&picture(&mut s, 200, 40)), 1);
    }

    #[test]
    fn a_narrow_view_is_the_same_thing_as_a_tree_with_the_chosen_row_filled() {
        let mut s = state();
        s.select_for_test("nvme0n1");
        let canvas = checked(build(&mut s, Path::new("/boot"), 40, 20));
        assert!(titles(&canvas).is_empty(), "rows, not boxes");
        let names = texts(&canvas);
        assert!(names.iter().any(|t| t.contains("[] nvme0n1")), "{names:?}");
        assert!(names.iter().any(|t| t.contains("+- ") || t.contains("+-")));
        let filled: Vec<_> = canvas
            .items
            .iter()
            .filter(|i| {
                matches!(i, Item::Block { at, paint, .. }
                if paint.mark == Mark::Chosen && at.w == 40 && at.h == 1)
            })
            .collect();
        assert_eq!(filled.len(), 1);
        assert!(
            names.contains(&"|"),
            "the row holding the folder you are in has a stripe"
        );
    }

    #[test]
    fn nothing_found_and_nothing_yet_say_so() {
        let mut s = WiringState::with_topology(Topology::default());
        assert_eq!(texts(&picture(&mut s, 80, 10)), ["no storage found"]);
    }

    #[test]
    fn a_chosen_box_far_down_the_page_is_scrolled_into_view_whole() {
        let mut s = state();
        s.select_for_test("dm-1");
        let canvas = checked(picture(&mut s, 200, 7));
        assert!(
            titles(&canvas).iter().any(|t| t.contains("root")),
            "{:?}",
            titles(&canvas)
        );
    }

    #[test]
    fn a_line_is_cut_to_the_window_and_dropped_when_none_of_it_shows() {
        let view = View {
            left: 10,
            top: 5,
            width: 20,
            height: 10,
        };
        assert_eq!(view.clip((12, 6), (18, 6)), Some(((2, 1), (8, 1))));
        assert_eq!(
            view.clip((0, 6), (50, 6)),
            Some(((0, 1), (19, 1))),
            "cut at both edges"
        );
        assert_eq!(view.clip((12, 2), (18, 2)), None, "above");
        assert_eq!(view.clip((40, 6), (50, 6)), None, "to the right");
        assert_eq!(view.clip((12, 0), (12, 8)), Some(((2, 0), (2, 3))));
        assert_eq!(
            view.clip((3, 0), (3, 8)),
            None,
            "a column left of the window"
        );
        assert_eq!(
            view.whole(Rect::new(10, 5, 20, 10)),
            Some(Rect::new(0, 0, 20, 10))
        );
        assert_eq!(
            view.whole(Rect::new(9, 5, 5, 3)),
            None,
            "cut by the left edge"
        );
        assert_eq!(
            view.whole(Rect::new(25, 5, 6, 3)),
            None,
            "cut by the right edge"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Whatever the size and the zoom, what is built is a canvas the host accepts: every
        /// part inside it, every path straight.
        #[test]
        fn any_view_size_and_zoom_builds_a_canvas_the_host_accepts(
            width in 1u16..300,
            height in 1u16..120,
            level in 0usize..5,
            chosen in 0usize..6,
        ) {
            let mut s = state();
            let ids = ["PCIe 02:00.0", "nvme0n1", "nvme0n1p1", "nvme0n1p2", "dm-0", "dm-1"];
            s.select_for_test(ids[chosen]);
            s.zoom_for_test(if level == 4 { Zoom::Fit } else { Zoom::Level(level) });
            let canvas = picture(&mut s, width, height);
            let set = SetCanvas {
                surface: Surface::Wiring,
                canvas: Some(canvas),
            };
            prop_assert!(set.checked().is_ok());
        }
    }
}
