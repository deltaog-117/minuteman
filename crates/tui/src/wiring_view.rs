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

//! Draws the wiring diagram over the browser's body: when the window is wide enough, a box for
//! every layer of the storage in columns with lines between them, the lines carrying a moving mark
//! where data is flowing; when it is not, the same thing as an indented tree, one row a node. The
//! node holding the folder being browsed has the bright frame ("you are here"), the chosen node is
//! filled, and while files are carried each filesystem says whether they fit, as on the deck.
//!
//! Where things go comes from `wiring_layout`, which the mouse also asks, so a click lands on the
//! node that was drawn there.

use std::path::Path;
use std::time::Instant;

use devices::topology::{Layer, Node, NodeKind};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Widget};
use theming::Config;

use crate::cargo_fit::{self, Verdict};
use crate::deck_view::rate_text;
use crate::glyphs;
use crate::hud::{fit_width, format_size, pad_to, text_width};
use crate::style;
use crate::trash_view::elide_left;
use crate::wiring::WiringState;
use crate::wiring_layout::{self, Mode};

fn icon(node: &Node, ascii: bool) -> &'static str {
    match (node.kind, node.removable, ascii) {
        (NodeKind::Bus, _, _) => "",
        (NodeKind::Disk, true, false) => "⏏ ",
        (NodeKind::Disk, false, false) => "▣ ",
        (NodeKind::Layer(_), _, false) => "◈ ",
        (NodeKind::Share, _, false) => "⇄ ",
        (NodeKind::Disk, _, true) => "[] ",
        (NodeKind::Layer(_), _, true) => "<> ",
        (NodeKind::Share, _, true) => "<> ",
        (NodeKind::Partition | NodeKind::Folder | NodeKind::More, _, _) => "",
    }
}

/// What the node is, in a few words.
fn kind_text(node: &Node) -> String {
    match node.kind {
        NodeKind::Bus => "bus".to_string(),
        NodeKind::Disk => {
            let removable = if node.removable { " · removable" } else { "" };
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
                    format!("{} · {fs}", layer.label())
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

/// A gauge `width` cells wide with `percent` of it filled.
fn gauge(percent: u8, width: usize, on: &str, off: &str) -> (String, String) {
    let filled = (width * usize::from(percent.min(100)) + 50) / 100;
    (on.repeat(filled), off.repeat(width - filled))
}

/// The sign in front of a name that can be unfolded: pointing right when folded, down when open.
fn fold_sign(state: &WiringState, index: usize, ascii: bool) -> &'static str {
    if state.topology().nodes[index].folder_path().is_none() {
        return "";
    }
    match (state.is_unfolded(index), ascii) {
        (true, false) => "▾ ",
        (false, false) => "▸ ",
        (true, true) => "- ",
        (false, true) => "+ ",
    }
}

/// The one line inside a folder's box (or a note's): how many folders it holds, or what to do.
fn folder_line(state: &WiringState, index: usize, width: usize, config: &Config) -> Line<'static> {
    let node = &state.topology().nodes[index];
    let dim = Style::default().fg(style::color(&config.theme.status_fg));
    let text = match node.kind {
        NodeKind::More if node.title().starts_with('+') => "Enter opens it".to_string(),
        NodeKind::More => String::new(),
        _ => match state.subfolders(index) {
            None => "folder".to_string(),
            Some(0) => "no folders".to_string(),
            Some(1) => "1 folder".to_string(),
            Some(n) => format!("{n} folders"),
        },
    };
    Line::from(Span::styled(pad_to(&fit_width(&text, width), width), dim))
}

/// The lines inside a node's box.
fn box_lines(
    state: &WiringState,
    index: usize,
    width: usize,
    config: &Config,
    ascii: bool,
) -> Vec<Line<'static>> {
    let node = &state.topology().nodes[index];
    if matches!(node.kind, NodeKind::Folder | NodeKind::More) {
        return vec![folder_line(state, index, width, config)];
    }
    let theme = &config.theme;
    let g = glyphs::of(config);
    let dim = Style::default().fg(style::color(&theme.status_fg));
    let accent = Style::default().fg(style::color(&theme.accent_fg));
    let danger = Style::default().fg(style::color(&theme.danger_fg));

    let first = Line::from(Span::styled(
        pad_to(&fit_width(&kind_text(node), width), width),
        dim,
    ));
    let second = Line::from(Span::styled(
        pad_to(&fit_width(&place_text(node, width), width), width),
        dim,
    ));
    let rates = state.rates(index);
    let third = if let Some(verdict) = state.verdict(index) {
        let style = match verdict {
            Verdict::Fits { .. } | Verdict::SameDisk => accent,
            Verdict::Short { .. } | Verdict::TooBig { .. } => danger,
            Verdict::Tight { .. } | Verdict::Unmounted | Verdict::Unsure => dim,
        };
        Line::from(Span::styled(
            fit_width(&cargo_fit::card_text(&verdict, ascii), width),
            style,
        ))
    } else if let Some(percent) = used_percent(node) {
        let tail = format!(" {percent:>3}%");
        let bar = width.saturating_sub(text_width(&tail));
        let (on, off) = gauge(percent, bar, g.gauge_on, g.gauge_off);
        let fill = if percent >= 90 { danger } else { accent };
        Line::from(vec![
            Span::styled(on, fill),
            Span::styled(off, dim),
            Span::raw(tail),
        ])
    } else if node.kind == NodeKind::Bus || node.kind == NodeKind::Disk || node.is_filesystem() {
        let busy = rates.read > 0 || rates.written > 0;
        Line::from(Span::styled(
            fit_width(&rate_text(rates.read, rates.written), width),
            if busy { accent } else { dim },
        ))
    } else {
        Line::from(Span::raw(String::new()))
    };
    vec![first, second, third]
}

/// Whether the load cannot go on this node, which dims it.
fn refused(state: &WiringState, index: usize) -> bool {
    matches!(
        state.verdict(index),
        Some(Verdict::Short { .. } | Verdict::TooBig { .. })
    )
}

pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &WiringState,
    current_dir: &Path,
    config: &Config,
    now: Instant,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(Clear, area);
    let theme = &config.theme;
    let dim = Style::default().fg(style::color(&theme.status_fg));
    if state.topology().nodes.is_empty() {
        let note = if state.has_scanned() {
            "no storage found"
        } else {
            "looking at how the storage is wired…"
        };
        frame.render_widget(Paragraph::new(Span::styled(note, dim)), area);
        return;
    }
    state.set_height(area.height);
    let layout = state.layout(area.width);
    match layout.mode {
        Mode::Wide => render_wide(frame, area, state, &layout, current_dir, config, now),
        Mode::Narrow => render_narrow(frame, area, state, &layout, current_dir, config),
    }
}

fn render_wide(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &WiringState,
    layout: &wiring_layout::Layout,
    current_dir: &Path,
    config: &Config,
    now: Instant,
) {
    let theme = &config.theme;
    let g = glyphs::of(config);
    let ascii = g.gauge_on.is_ascii();
    let accent = Style::default().fg(style::color(&theme.accent_fg));
    let line_style = Style::default().fg(style::color(&theme.border_fg));

    // Scroll so the chosen box is whole on screen, moving as little as possible: down the page,
    // and sideways when unfolded folders have run out past the window.
    let selected_node = state.index();
    let selected = selected_node.and_then(|i| layout.rect_of(i));
    let (mut top, mut left) = (state.top().get(), state.left().get());
    if let Some(rect) = selected {
        if rect.y < top {
            top = rect.y;
        } else if rect.bottom() > top + area.height {
            top = rect.bottom().saturating_sub(area.height);
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
        } else if want_right > left + area.width {
            left = want_right.saturating_sub(area.width).min(rect.x);
        }
    }
    top = top.min(layout.size.1.saturating_sub(area.height));
    left = left.min(layout.size.0.saturating_sub(area.width));
    state.top().set(top);
    state.left().set(left);

    // The lines first, so the boxes sit over them.
    let buf = frame.buffer_mut();
    let mut put = |x: u16, y: u16, symbol: &str, style: Style| {
        if y < top || y >= top + area.height || x < left || x >= left + area.width {
            return;
        }
        buf[(area.x + (x - left), area.y + (y - top))]
            .set_symbol(symbol)
            .set_style(style);
    };
    for ((x, y), ch) in wiring_layout::connectors(layout) {
        put(x, y, &ch.to_string(), line_style);
    }
    // A mark travels along the line into each node that is busy.
    let phase = state.phase(now);
    for &(parent, child) in &layout.links {
        let rates = state.rates(child);
        if rates.read == 0 && rates.written == 0 {
            continue;
        }
        let path = wiring_layout::path_into(layout, parent, child);
        if path.is_empty() {
            continue;
        }
        let at = usize::try_from(phase % path.len() as u64).unwrap_or(0);
        let (x, y) = path[at];
        put(x, y, if ascii { "*" } else { "●" }, accent);
    }

    let here = state.here(current_dir);
    let window = Rect::new(left, top, area.width, area.height);
    for placed in &layout.placed {
        let rect = placed.rect;
        let visible = rect.intersection(window);
        if visible.is_empty() {
            continue;
        }
        let look = BoxLook {
            state,
            node: placed.node,
            here,
            config,
            scale: layout.scale,
            ascii,
        };
        if visible == rect {
            let screen = Rect::new(
                area.x + (rect.x - left),
                area.y + (rect.y - top),
                rect.width,
                rect.height,
            );
            paint_box(frame.buffer_mut(), screen, &look);
            continue;
        }
        // Only part of the box is on screen. Draw all of it somewhere off screen and copy over the
        // part that is, so it is cut off by the window's edge like a picture, not left out.
        let mut scratch = Buffer::empty(Rect::new(0, 0, rect.width, rect.height));
        paint_box(
            &mut scratch,
            Rect::new(0, 0, rect.width, rect.height),
            &look,
        );
        let buf = frame.buffer_mut();
        for y in visible.y..visible.bottom() {
            for x in visible.x..visible.right() {
                buf[(area.x + (x - left), area.y + (y - top))] =
                    scratch[(x - rect.x, y - rect.y)].clone();
            }
        }
    }
}

/// What `paint_box` needs to draw one node's box.
struct BoxLook<'a> {
    state: &'a WiringState,
    node: usize,
    here: Option<usize>,
    config: &'a Config,
    scale: wiring_layout::Scale,
    ascii: bool,
}

/// Draws the box of one node into `buf` at `screen`: a frame round its lines, or just its name
/// when the boxes are too small for frames.
fn paint_box(buf: &mut Buffer, screen: Rect, look: &BoxLook<'_>) {
    let BoxLook {
        state,
        node: index,
        here,
        config,
        scale,
        ascii,
    } = *look;
    let theme = &config.theme;
    let node = &state.topology().nodes[index];
    // A mount or an eject started from the chosen node is still running.
    let working = if state.index() == Some(index) && state.is_working() {
        " …"
    } else {
        ""
    };
    let name = format!(
        "{}{}{}{working}",
        fold_sign(state, index, ascii),
        icon(node, ascii),
        node.title()
    );
    let mut base = Style::default();
    if state.index() == Some(index) {
        base = base.bg(style::color(&theme.selection_bg));
    }
    if refused(state, index) {
        base = base.add_modifier(Modifier::DIM);
    }
    if !scale.bordered {
        // The smallest boxes are one line of text, the frame told by bold type.
        let plain = fit_width(&name, usize::from(screen.width));
        let mut style = base;
        if here == Some(index) {
            style = style
                .fg(style::color(&theme.accent_fg))
                .add_modifier(Modifier::BOLD);
        }
        let padded = pad_to(&plain, usize::from(screen.width));
        Paragraph::new(Span::styled(padded, style)).render(screen, buf);
        return;
    }
    let title = fit_width(&name, usize::from(scale.node_w.saturating_sub(4)));
    let block = style::themed_block(config, &title, here == Some(index)).style(base);
    let inner = block.inner(screen);
    block.render(screen, buf);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let lines = box_lines(state, index, usize::from(inner.width), config, ascii);
    Paragraph::new(lines).render(inner, buf);
}

fn render_narrow(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &WiringState,
    layout: &wiring_layout::Layout,
    current_dir: &Path,
    config: &Config,
) {
    let theme = &config.theme;
    let g = glyphs::of(config);
    let ascii = g.gauge_on.is_ascii();
    let dim = Style::default().fg(style::color(&theme.status_fg));
    let accent = Style::default().fg(style::color(&theme.accent_fg));
    let danger = Style::default().fg(style::color(&theme.danger_fg));

    let selected_row = state.index().and_then(|i| layout.row_of(i)).unwrap_or(0);
    let height = usize::from(area.height);
    let mut top = usize::from(state.top().get());
    if selected_row < top {
        top = selected_row;
    } else if selected_row >= top + height {
        top = selected_row + 1 - height;
    }
    top = top.min(layout.rows.len().saturating_sub(height));
    state.top().set(u16::try_from(top).unwrap_or(u16::MAX));

    let here = state.here(current_dir);
    let width = usize::from(area.width);
    // Deep folders are indented a step for every level, so the name column grows to hold the
    // longest row (up to three fifths of the width) instead of cutting the deepest names short.
    let name_of = |row: &wiring_layout::Row| {
        let node = &state.topology().nodes[row.node];
        format!(
            "{}{}{}{}",
            row.prefix,
            fold_sign(state, row.node, ascii),
            icon(node, ascii),
            node.title()
        )
    };
    let longest = layout
        .rows
        .iter()
        .map(|row| text_width(&name_of(row)))
        .max()
        .unwrap_or(0);
    let name_width = longest
        .clamp(18, (width * 3 / 5).max(40))
        .min(width.saturating_sub(2));
    let lines: Vec<Line<'static>> = layout
        .rows
        .iter()
        .enumerate()
        .skip(top)
        .take(height)
        .map(|(_, row)| {
            let node = &state.topology().nodes[row.node];
            let chosen = state.index() == Some(row.node);
            let stripe = if here == Some(row.node) {
                format!("{} ", g.stripe)
            } else {
                "  ".to_string()
            };
            let name = name_of(row);
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
            let tail = match &verdict {
                Some(v) => cargo_fit::card_text(v, ascii),
                None if busy || node.kind == NodeKind::Bus => rate_text(rates.read, rates.written),
                None => String::new(),
            };
            let tail_style = match &verdict {
                Some(Verdict::Short { .. } | Verdict::TooBig { .. }) => danger,
                Some(Verdict::Fits { .. } | Verdict::SameDisk) => accent,
                _ if busy => accent,
                _ => dim,
            };
            let rest = width.saturating_sub(2 + name_width + 1);
            let details = fit_width(
                &detail.join("  "),
                rest.saturating_sub(text_width(&tail) + 2),
            );
            let mut spans = vec![
                Span::styled(stripe, accent),
                Span::raw(pad_to(&fit_width(&name, name_width), name_width)),
                Span::raw(" "),
                Span::styled(
                    pad_to(&details, rest.saturating_sub(text_width(&tail) + 2)),
                    dim,
                ),
            ];
            if !tail.is_empty() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(tail, tail_style));
            }
            let line = Line::from(spans);
            if chosen {
                line.style(Style::default().bg(style::color(&theme.selection_bg)))
            } else if refused(state, row.node) {
                line.style(Style::default().add_modifier(Modifier::DIM))
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use devices::topology::{Node, Topology};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Position;

    use super::*;
    use crate::cargo_fit::{Carry, Load};

    /// bus -> nvme -> {boot (/boot), pv -> lvm -> root (/, 94% full)} and a share.
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
        let nas = node(
            "nas:/export",
            NodeKind::Share,
            Some("/mnt/nas"),
            Some("nfs4"),
            None,
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
        let _ = nas;
        t
    }

    fn state() -> (tokio::runtime::Runtime, WiringState) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let state = WiringState::with_topology(runtime.handle().clone(), machine());
        (runtime, state)
    }

    fn drawn(state: &WiringState, here: &str, width: u16, height: u16, now: Instant) -> String {
        let config = Config::from_sources(None, None, None);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state, Path::new(here), &config, now))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[Position::new(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_wide_diagram_draws_every_layer_in_a_box_joined_by_lines() {
        let (_rt, s) = state();
        let shown = drawn(&s, "/home/me", 150, 30, Instant::now());
        for text in [
            "PCIe 02:00.0",
            "nvme0n1",
            "nvme0n1p1",
            "Vg-root",
            "root",
            "vfat",
            "LVM2_member",
            "btrfs",
            "/boot",
            "94%",
            "LVM",
            "LUKS",
        ] {
            assert!(shown.contains(text), "{text:?} missing:\n{shown}");
        }
        assert!(
            shown.contains('├') || shown.contains('┬') || shown.contains('┤'),
            "no branch drawn:\n{shown}"
        );
        assert!(shown.contains('╭'), "boxes are framed:\n{shown}");
    }

    #[test]
    fn a_window_too_narrow_for_the_columns_gets_the_indented_tree() {
        let (_rt, s) = state();
        s.zoom_for_test(crate::wiring_layout::Zoom::Level(0));
        let shown = drawn(&s, "/", 70, 14, Instant::now());
        assert!(!shown.contains('╭'), "no boxes in the tree:\n{shown}");
        for text in [
            "PCIe 02:00.0",
            "└─",
            "├─",
            "nvme0n1p1",
            "/boot",
            "root",
            "94%",
        ] {
            assert!(shown.contains(text), "{text:?} missing:\n{shown}");
        }
    }

    #[test]
    fn the_node_holding_the_browsed_folder_has_the_bright_frame_in_the_wide_diagram() {
        let (_rt, s) = state();
        let config = Config::from_sources(None, None, None);
        let bright = style::color(&config.theme.border_focused_fg);
        let layout = s.layout(150);
        let find = |id: &str| s.topology().nodes.iter().position(|n| n.id == id).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(150, 30)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &s,
                    Path::new("/home/me"),
                    &config,
                    Instant::now(),
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let corner = |id: &str| {
            let r = layout.rect_of(find(id)).unwrap();
            buffer[Position::new(r.x, r.y)].fg
        };
        assert_eq!(corner("dm-1"), bright, "the root filesystem holds /home/me");
        assert_ne!(corner("nvme0n1p1"), bright);
    }

    #[test]
    fn a_busy_disk_has_a_mark_that_moves_along_the_line_into_it() {
        let (_rt, mut s) = state();
        let mut counters = std::collections::HashMap::new();
        counters.insert(
            "nvme0n1".to_string(),
            devices::activity::Counters {
                read: 0,
                written: 0,
            },
        );
        s.activity_update(&counters);
        counters.insert(
            "nvme0n1".to_string(),
            devices::activity::Counters {
                read: 4096,
                written: 0,
            },
        );
        s.activity_update(&counters);
        let t0 = s.epoch_for_test();
        let marks = |dt: u64| {
            drawn(&s, "/", 150, 30, t0 + Duration::from_millis(dt))
                .lines()
                .map(|l| l.matches('●').count())
                .sum::<usize>()
        };
        assert_eq!(marks(0), 1, "one mark on the line into the busy disk");
        let at = |dt: u64| {
            drawn(&s, "/", 150, 30, t0 + Duration::from_millis(dt))
                .lines()
                .enumerate()
                .find_map(|(y, l)| l.chars().position(|c| c == '●').map(|x| (x, y)))
        };
        assert_ne!(at(0), at(100), "it moves");
        assert_eq!(
            at(0).unwrap().1,
            at(100).unwrap().1,
            "along the line, not off it"
        );
        let quiet = state().1;
        let none = drawn(&quiet, "/", 150, 30, t0).matches('●').count();
        assert_eq!(none, 0, "an idle machine has no marks");
    }

    #[test]
    fn while_files_are_carried_each_filesystem_says_whether_they_fit_and_a_refused_one_is_dimmed() {
        let (_rt, mut s) = state();
        s.zoom_for_test(crate::wiring_layout::Zoom::Level(0));
        s.set_load(Some(Load {
            bytes: 300 << 30,
            exact: true,
            largest: 10,
            carry: Carry::Copy,
            source: "/home/me/photos".into(),
        }));
        let shown = drawn(&s, "/", 150, 30, Instant::now());
        assert!(shown.contains("✓ fits"), "/boot has room:\n{shown}");
        assert!(
            shown.contains("✗ needs"),
            "the nearly full root does not:\n{shown}"
        );
        let narrow = drawn(&s, "/", 70, 14, Instant::now());
        assert!(
            narrow.contains("✓ fits") && narrow.contains("✗ needs"),
            "{narrow}"
        );
    }

    #[test]
    fn the_chosen_box_stays_on_screen_when_there_are_more_than_fit() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut t = Topology::default();
        for i in 0..8 {
            let d = t.add(Node::new(&format!("sd{i}"), NodeKind::Disk));
            let p = t.add(Node::new(&format!("sd{i}1"), NodeKind::Partition));
            t.link(d, p);
        }
        let mut s = WiringState::with_topology(runtime.handle().clone(), t);
        s.select_for_test("sd71");
        let shown = drawn(&s, "/", 150, 14, Instant::now());
        assert!(
            shown.contains("sd71"),
            "scrolled to the chosen box:\n{shown}"
        );
        assert!(!shown.contains("sd01"), "and away from the first:\n{shown}");
        s.select_for_test("sd01");
        assert!(
            drawn(&s, "/", 150, 14, Instant::now()).contains("sd01"),
            "and back"
        );
        let narrow = {
            s.select_for_test("sd71");
            drawn(&s, "/", 30, 6, Instant::now())
        };
        assert!(narrow.contains("sd71"), "{narrow}");
    }

    /// The machine with a real folder tree to unfold, browsed at `here`.
    fn folders_state(here: &str) -> (tokio::runtime::Runtime, WiringState) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let vfs = shared::MemVfs::new();
        for dir in [
            "/usr/lib",
            "/etc",
            "/home/me/projects/minuteman",
            "/boot/grub",
            "/boot/efi",
        ] {
            vfs.add_dir(dir);
        }
        let mut s = WiringState::with_vfs(runtime.handle().clone(), machine(), Arc::new(vfs));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll(Instant::now(), Path::new(here));
            if s.topology().nodes.iter().all(|n| n.title() != "loading…") {
                return (runtime, s);
            }
            assert!(Instant::now() < deadline, "folders were never read");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn unfolded_folders_are_short_boxes_with_a_sign_and_a_count_to_the_right_of_their_filesystem() {
        let (_rt, s) = folders_state("/home/me/projects");
        let shown = drawn(&s, "/home/me/projects", 150, 40, Instant::now());
        for text in [
            "▾ ◈ root",
            "▾ home",
            "▾ me",
            "▾ projects",
            "▸ etc",
            "1 folder",
        ] {
            assert!(shown.contains(text), "{text:?} missing:\n{shown}");
        }
        assert!(
            shown.contains("minuteman"),
            "the folder we are in has its own child:\n{shown}"
        );
    }

    #[test]
    fn the_folder_being_browsed_has_the_bright_frame_and_not_its_filesystem() {
        let (_rt, s) = folders_state("/home/me/projects");
        let config = Config::from_sources(None, None, None);
        let bright = style::color(&config.theme.border_focused_fg);
        let mut terminal = Terminal::new(TestBackend::new(150, 40)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &s,
                    Path::new("/home/me/projects"),
                    &config,
                    Instant::now(),
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let lit = (0..buffer.area.width)
            .flat_map(|x| (0..buffer.area.height).map(move |y| (x, y)))
            .filter(|&(x, y)| {
                buffer[Position::new(x, y)].symbol() == "╭"
                    && buffer[Position::new(x, y)].fg == bright
            })
            .count();
        assert_eq!(lit, 1, "exactly one box is lit: the folder");
    }

    #[test]
    fn the_diagram_scrolls_sideways_to_keep_the_chosen_deep_folder_on_screen() {
        let (_rt, s) = folders_state("/home/me/projects/minuteman");
        let narrow_ish = 130;
        let layout = s.layout(narrow_ish);
        assert_eq!(layout.mode, Mode::Wide, "the hardware fits");
        assert!(
            layout.size.0 > narrow_ish,
            "but the folders run out past the window"
        );
        let shown = drawn(
            &s,
            "/home/me/projects/minuteman",
            narrow_ish,
            40,
            Instant::now(),
        );
        assert!(
            shown.contains("minuteman"),
            "the chosen folder is on screen:\n{shown}"
        );
        assert!(
            !shown.contains("PCIe 02:00.0"),
            "and the far left has scrolled away:\n{shown}"
        );
        assert!(s.left().get() > 0);
    }

    #[test]
    fn the_narrow_tree_lists_folders_under_their_filesystem() {
        let (_rt, s) = folders_state("/home/me/projects");
        s.zoom_for_test(crate::wiring_layout::Zoom::Level(0));
        let shown = drawn(&s, "/home/me/projects", 70, 40, Instant::now());
        for text in ["▾ ◈ root", "▾ home", "▾ projects", "▸ etc", "minuteman"] {
            assert!(shown.contains(text), "{text:?} missing:\n{shown}");
        }
    }

    #[test]
    fn a_note_about_more_folders_says_how_to_open_them() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let vfs = shared::MemVfs::new();
        for i in 0..20 {
            vfs.add_dir(format!("/big/d{i:02}"));
        }
        let mut t = Topology::default();
        let mut n = Node::new("sdz1", NodeKind::Partition);
        n.mount = Some("/big".into());
        n.usage = Some("filesystem".into());
        n.total = 1000;
        n.free = 500;
        t.add(n);
        let mut s = WiringState::with_vfs(runtime.handle().clone(), t, Arc::new(vfs));
        s.select_for_test("sdz1");
        s.key(
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Right,
                crossterm::event::KeyModifiers::NONE,
            ),
            150,
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while s.topology().nodes.iter().all(|n| n.title() != "+8 more") {
            s.poll(Instant::now(), Path::new("elsewhere"));
            assert!(Instant::now() < deadline, "never read");
            std::thread::sleep(Duration::from_millis(2));
        }
        let shown = drawn(&s, "elsewhere", 150, 80, Instant::now());
        assert!(
            shown.contains("+8 more") && shown.contains("Enter opens it"),
            "{shown}"
        );
    }

    #[test]
    fn an_empty_machine_says_it_is_looking_and_then_that_it_found_nothing() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut s = WiringState::new(
            runtime.handle().clone(),
            Arc::new(shared::MemVfs::new()),
            false,
        );
        assert!(drawn(&s, "/", 60, 8, Instant::now()).contains("looking at how"));
        s.mark_scanned_for_test();
        assert!(drawn(&s, "/", 60, 8, Instant::now()).contains("no storage found"));
    }

    /// The machine with forty sibling folders under `/home/me`, browsed at one of them.
    fn crowded_state(here: &str) -> (tokio::runtime::Runtime, WiringState) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let vfs = shared::MemVfs::new();
        for i in 0..40 {
            vfs.add_dir(format!("/home/me/d{i:02}/inner"));
        }
        let mut s = WiringState::with_vfs(runtime.handle().clone(), machine(), Arc::new(vfs));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll(Instant::now(), Path::new(here));
            if s.topology().nodes.iter().all(|n| n.title() != "loading…") {
                return (runtime, s);
            }
            assert!(Instant::now() < deadline, "folders were never read");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn once_the_size_is_known_the_hardware_and_the_deep_folder_are_on_screen_together() {
        let here = "/home/me/projects/minuteman";
        let (_rt, mut s) = folders_state(here);
        let before = drawn(&s, here, 130, 40, Instant::now());
        assert!(
            !before.contains("PCIe 02:00.0"),
            "unsized, the far left scrolls away:\n{before}"
        );
        s.set_size(130, 40);
        let shown = drawn(&s, here, 130, 40, Instant::now());
        // The boxes are squeezed to fit, so long names are cut short with an ellipsis.
        for text in ["PCIe", "▣ nvm", "▾ ◈ r", "▾ pro", "▾ min"] {
            assert!(shown.contains(text), "{text:?} missing:\n{shown}");
        }
        assert_eq!((s.left().get(), s.top().get()), (0, 0), "nothing scrolled");
    }

    #[test]
    fn a_shorter_window_shows_fewer_siblings_and_still_the_one_being_browsed() {
        let here = "/home/me/d27";
        let (_rt, mut s) = crowded_state(here);
        let siblings = |s: &WiringState, height: u16| {
            let shown = drawn(s, here, 200, height, Instant::now());
            (
                shown.matches("▸ d").count() + shown.matches("▾ d").count(),
                shown,
            )
        };
        s.set_size(200, 80);
        let (tall, shown) = siblings(&s, 80);
        assert!(shown.contains("d27"), "{shown}");
        s.set_size(200, 14);
        let (short, shown) = siblings(&s, 14);
        assert!(short < tall, "{short} < {tall}:\n{shown}");
        assert!(shown.contains("d27"), "the way down is kept:\n{shown}");
        assert!(shown.contains("more"), "and the cut says so:\n{shown}");
        s.set_size(200, 80);
        assert_eq!(
            siblings(&s, 80).0,
            tall,
            "growing the window brings them back"
        );
    }

    #[test]
    fn a_folder_chosen_in_the_diagram_survives_the_window_shrinking() {
        let here = "/home/me/d02";
        let (_rt, mut s) = crowded_state(here);
        s.set_size(200, 80);
        s.select_for_test("/home/me/d30");
        s.set_size(200, 12);
        assert!(
            s.index().is_some(),
            "d30 is far from the browsed folder, but it is the chosen one"
        );
    }

    #[test]
    fn a_box_cut_by_the_window_edge_is_drawn_cut_not_left_out() {
        let here = "/home/me/projects/minuteman";
        let (_rt, s) = folders_state(here);
        let mut straddlers = 0;
        for width in 100..=140u16 {
            let shown = drawn(&s, here, width, 40, Instant::now());
            let lines: Vec<&str> = shown.lines().collect();
            let layout = s.layout(width);
            let (left, top) = (s.left().get(), s.top().get());
            for placed in &layout.placed {
                let r = placed.rect;
                if r.x < left && r.right() > left && r.y >= top && r.bottom() <= top + 40 {
                    straddlers += 1;
                    // The slice of the box that is on screen, from column 0 to its right edge, is
                    // drawn: its frame is there.
                    let visible = usize::from(r.right() - left);
                    let drawn_cells = (r.y - top..r.bottom() - top)
                        .flat_map(|y| lines[usize::from(y)].chars().take(visible))
                        .filter(|c| *c != ' ')
                        .count();
                    assert!(
                        drawn_cells > 0,
                        "width {width}: the box at x={} is cut by the window's left edge but \
                         nothing of it is drawn:\n{shown}",
                        r.x
                    );
                }
            }
        }
        assert!(straddlers > 0, "no width made a box straddle the edge");
    }

    #[test]
    fn a_sized_diagram_draws_at_any_size_without_panicking() {
        let here = "/home/me/projects/minuteman";
        let (_rt, mut s) = folders_state(here);
        for (w, h) in [
            (0, 0),
            (1, 1),
            (9, 3),
            (30, 6),
            (60, 10),
            (99, 20),
            (130, 40),
            (200, 25),
            (300, 80),
        ] {
            s.set_size(w, h);
            drawn(&s, here, w, h, Instant::now());
        }
    }

    #[test]
    fn it_draws_at_any_size_without_panicking() {
        let (_rt, s) = state();
        for (w, h) in [
            (0, 0),
            (1, 1),
            (5, 3),
            (19, 4),
            (20, 5),
            (40, 2),
            (80, 3),
            (119, 30),
            (120, 30),
            (300, 80),
        ] {
            drawn(&s, "/", w, h, Instant::now());
        }
    }
}
