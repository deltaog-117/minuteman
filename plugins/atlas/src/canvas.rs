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

//! The atlas as a canvas: the picture the host draws, built from the state and the size of the
//! view. Everything is said in meaning (a tone, a hue number, a mark) and never in color or
//! symbol, so the picture follows the user's theme and glyph set; the one cost is that nothing
//! here can use a non-ASCII symbol, so the volume icons, the "you are here" glyph and the
//! separators of the core's version are plain ASCII.
//!
//! The view is a frame around everything, a line of totals at the top, a line of hints at the
//! bottom (when there is room for one) and the map between. A tile is a block of its kind's hue
//! with its label written over it; a volume's block is split by how full the volume is, free part
//! dimmed at the top, and the chosen tile is drawn in the selection color with a frame round it.
//! The host cuts and cleans text and refuses a canvas over its limits, so the builder also keeps
//! within them: past the item or character budget the smallest tiles are left bare.

use std::path::Path;

use plugin_api::{
    Canvas, Cells, Item as Part, MAX_CANVAS_CHARS, MAX_CANVAS_ITEMS, Mark, Paint, Tone,
};
use shared::devices::Device;
use shared::format::{format_size, group_thousands};
use shared::usage::{End, Kind, Measure, Row};

use crate::layout::{Item, Rect, Tile};
use crate::state::{AtlasState, HINT};

/// A volume this full (as a whole percent) is drawn in the danger tone.
const FULL_PERCENT: u8 = 90;

/// The parts kept back for the frame, the header, the footer and the note, so the tiles cannot
/// crowd them out.
const RESERVED_ITEMS: usize = 8;

/// The characters kept back for the same.
const RESERVED_CHARS: usize = 2_000;

/// The regions of the view within its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Places {
    pub frame: Rect,
    /// The line of totals.
    pub header: Rect,
    /// Where the tiles go.
    pub map: Rect,
    /// The key hints.
    pub footer: Rect,
}

/// Where everything sits in a view of `width` by `height` cells: a frame around all of it, a line
/// of totals at the top, a line of hints at the bottom (when there is room for one), and the map
/// between.
pub fn places(width: u16, height: u16) -> Places {
    let frame = Rect::new(0, 0, width, height);
    let inner = Rect::new(
        u16::from(width > 0),
        u16::from(height > 0),
        width.saturating_sub(2),
        height.saturating_sub(2),
    );
    let header = Rect {
        height: inner.height.min(1),
        ..inner
    };
    let footer_height = u16::from(inner.height >= 4);
    let footer = Rect {
        y: inner.bottom().saturating_sub(footer_height),
        height: footer_height,
        ..inner
    };
    let map = Rect {
        y: inner.y + header.height,
        height: inner
            .height
            .saturating_sub(header.height)
            .saturating_sub(footer_height),
        ..inner
    };
    Places {
        frame,
        header,
        map,
        footer,
    }
}

/// The rectangle of a tile that is painted: the right column and bottom row are left bare when
/// the tile is big enough to spare them, so neighbouring tiles of one color stay apart.
pub fn painted(rect: Rect) -> Rect {
    Rect {
        width: if rect.width >= 3 {
            rect.width - 1
        } else {
            rect.width
        },
        height: if rect.height >= 3 {
            rect.height - 1
        } else {
            rect.height
        },
        ..rect
    }
}

/// How many of `rows` painted rows a volume `percent` full leaves free, at the top: the used part
/// grows up from the bottom.
pub fn free_rows(rows: u16, percent: u8) -> u16 {
    let used = (u32::from(rows) * u32::from(percent.min(100)) + 50) / 100;
    rows - u16::try_from(used).unwrap_or(rows).min(rows)
}

/// `share` of `total` as a whole percent, at most 100.
pub fn share_percent(size: u64, total: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    u64::try_from((u128::from(size) * 100) / u128::from(total))
        .unwrap_or(100)
        .min(100)
}

/// `text` cut to its last `width` characters behind `...` when it is longer, so the end of a path
/// (the part that tells folders apart) survives.
pub fn elide_left(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(3);
    let tail: String = text.chars().skip(count - keep).collect();
    format!("...{tail}")
}

fn pin_mark(pinned: bool) -> &'static str {
    if pinned { "* " } else { "" }
}

/// How a tile looks and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Spec {
    paint: Paint,
    /// How many of the painted rows, counting from the top, are dimmed (the free part of a
    /// volume).
    dim_rows: u16,
    lines: Vec<String>,
}

fn paint(tone: Tone, hue: Option<u8>) -> Paint {
    Paint {
        tone,
        hue,
        mark: Mark::None,
    }
}

/// A volume's block: split by how full it is, in the danger tone when nearly full, and dimmed
/// when it is not mounted (nothing to look into).
fn volume_spec(device: &Device, pinned: bool, rows: u16, width: usize) -> Spec {
    let percent = device.used_percent().unwrap_or(0);
    let mut lines = vec![format!("{}{}", pin_mark(pinned), device.title())];
    lines.push(match device.used_percent() {
        Some(p) => format!("{p}% of {}", format_size(device.total)),
        None => format_size(device.total),
    });
    lines.push(match &device.mount {
        Some(mount) => elide_left(&mount.display().to_string(), width),
        None => "not mounted".to_string(),
    });
    lines.push(format!(
        "{} | {} free",
        device.fs.as_deref().unwrap_or("?"),
        format_size(device.free)
    ));
    if !device.is_mounted() {
        return Spec {
            paint: paint(Tone::Dim, None),
            dim_rows: 0,
            lines,
        };
    }
    let tone = if percent >= FULL_PERCENT {
        Tone::Danger
    } else {
        Tone::Accent
    };
    Spec {
        paint: paint(tone, None),
        dim_rows: free_rows(rows, percent),
        lines,
    }
}

/// An entry of the folder zoomed into: colored by what it is, named, sized, and given its share
/// of the folder.
fn row_spec(row: &Row, pinned: bool, measure: Measure, total: u64) -> Spec {
    let size = row.sizes.of(measure);
    let (paint, dim_rows) = match row.kind {
        Kind::Dir => (paint(Tone::Normal, Some(0)), 0),
        Kind::File => (paint(Tone::Normal, Some(1)), 0),
        Kind::Link | Kind::Other | Kind::OtherFilesystem => (paint(Tone::Dim, None), 0),
        Kind::Rest => (paint(Tone::Dim, None), u16::MAX),
    };
    let name = match row.kind {
        Kind::Dir => format!("{}/", row.name),
        Kind::OtherFilesystem => format!("{}/ (other filesystem)", row.name),
        Kind::Link => format!("{} (link)", row.name),
        Kind::File | Kind::Other | Kind::Rest => row.name.clone(),
    };
    let flag = if row.partial { " !" } else { "" };
    let mut lines = vec![
        format!("{}{name}", pin_mark(pinned)),
        format!(
            "{} | {}%{flag}",
            format_size(size),
            share_percent(size, total)
        ),
    ];
    if row.kind == Kind::Dir {
        lines.push(format!("{} items", row.items));
    }
    Spec {
        paint,
        dim_rows,
        lines,
    }
}

/// The block that stands for entries too small to have their own.
fn small_spec(count: usize) -> Spec {
    Spec {
        paint: paint(Tone::Dim, None),
        dim_rows: u16::MAX,
        lines: vec![format!(
            "+{count} smaller {}",
            if count == 1 { "entry" } else { "entries" }
        )],
    }
}

/// What to say instead of a map when there are no tiles.
fn empty_note(state: &AtlasState) -> &'static str {
    match state.zoomed() {
        None if !state.has_scanned() => "looking for volumes...",
        None => "no volume with a known size",
        Some(view) => match view.end() {
            None => "scanning...",
            Some(End::Unreadable) => "cannot read this folder",
            Some(_) => "nothing here takes any space",
        },
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

/// What the parts added so far cost against the limits.
struct Budget {
    items: usize,
    chars: usize,
}

impl Budget {
    fn fits(&self, items: usize, chars: usize) -> bool {
        self.items + items + RESERVED_ITEMS <= MAX_CANVAS_ITEMS
            && self.chars + chars + RESERVED_CHARS <= MAX_CANVAS_CHARS
    }

    fn spend(&mut self, parts: &[Part]) {
        self.items += parts.len();
        self.chars += parts.iter().map(chars_of).sum::<usize>();
    }
}

fn chars_of(part: &Part) -> usize {
    match part {
        Part::Text { text, .. } => text.chars().count(),
        Part::Frame { title, .. } => title.as_ref().map_or(0, |t| t.chars().count()),
        Part::Block { .. } | Part::Path { .. } => 0,
    }
}

/// The parts of one tile: its block (two, for a volume with a free part), its label inset by one
/// cell so a frame fits round it, and, for the chosen tile, that frame.
fn tile_parts(rect: Rect, spec: &Spec, chosen: bool) -> Vec<Part> {
    let body = painted(rect);
    if body.width == 0 || body.height == 0 {
        return Vec::new();
    }
    let paint = Paint {
        mark: if chosen { Mark::Chosen } else { Mark::None },
        ..spec.paint
    };
    let dim = spec.dim_rows.min(body.height);
    let mut parts = Vec::new();
    let dimmed = Paint { ..paint };
    if dim > 0 {
        // A block with no level is all one color; one at level 0 is all dimmed.
        parts.push(Part::Block {
            at: cells(Rect::new(body.x, body.y, body.width, dim)),
            paint: dimmed,
            level: Some(0),
        });
    }
    if dim < body.height {
        parts.push(Part::Block {
            at: cells(Rect::new(
                body.x,
                body.y + dim,
                body.width,
                body.height - dim,
            )),
            paint,
            level: None,
        });
    }
    let room = if body.width >= 3 && body.height >= 3 {
        Rect::new(body.x + 1, body.y + 1, body.width - 2, body.height - 2)
    } else {
        body
    };
    if room.width >= 3 {
        for (i, line) in spec.lines.iter().enumerate().take(usize::from(room.height)) {
            let row = u16::try_from(i).unwrap_or(0);
            parts.push(Part::Text {
                x: room.x,
                y: room.y + row,
                width: room.width,
                text: line.clone(),
                paint: Paint {
                    tone: spec.paint.tone,
                    hue: None,
                    mark: Mark::None,
                },
                bold: i == 0,
            });
        }
    }
    if chosen && body.width >= 3 && body.height >= 3 {
        parts.push(Part::Frame {
            at: cells(body),
            paint: Paint {
                mark: Mark::Chosen,
                ..Paint::default()
            },
            title: None,
        });
    }
    parts
}

fn text(rect: Rect, text: String, tone: Tone) -> Part {
    Part::Text {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        text,
        paint: paint(tone, None),
        bold: false,
    }
}

fn header_text(state: &AtlasState) -> String {
    if let Some(view) = state.zoomed() {
        let entries: u64 = view
            .rows()
            .iter()
            .map(|row| row.items + u64::from(row.kind == Kind::Dir))
            .sum();
        let scan = match view.end() {
            None => format!("scanning {}", group_thousands(view.seen())),
            Some(End::Complete) => String::new(),
            Some(End::Truncated) => "stopped early, sizes are lower bounds".to_string(),
            Some(End::Cancelled) => "stopped".to_string(),
            Some(End::Unreadable) => "cannot read this folder".to_string(),
        };
        let mut parts = vec![
            format_size(view.total()),
            format!("{} entries", group_thousands(entries)),
            view.measure().label().to_string(),
        ];
        if !scan.is_empty() {
            parts.push(scan);
        }
        return parts.join(" | ");
    }
    let capacity: u64 = state.devices().iter().map(|d| d.total).sum();
    let free: u64 = state
        .devices()
        .iter()
        .filter(|d| d.is_mounted())
        .map(|d| d.free)
        .sum();
    let count = state.devices().len();
    format!(
        "{count} {} | {} in all | {} free",
        if count == 1 { "volume" } else { "volumes" },
        format_size(capacity),
        format_size(free)
    )
}

fn title(state: &AtlasState, width: u16) -> String {
    match state.zoomed() {
        Some(view) => format!(
            "atlas: {}{}",
            state
                .zoomed_device()
                .map_or(String::new(), |d| format!("{} ", d.title())),
            elide_left(
                &view.path().display().to_string(),
                usize::from(width).saturating_sub(20)
            )
        ),
        None => "atlas".to_string(),
    }
}

/// The keys the atlas wants right now. `Esc` and `Backspace` only once zoomed into a volume (at
/// the map of volumes `Esc` closes the view, which is the browser's), and `a` likewise (on the map
/// it is the appearance popup).
fn claims(state: &AtlasState) -> Vec<String> {
    let mut keys: Vec<&str> = vec![
        "up", "down", "left", "right", "h", "j", "k", "l", "tab", "backtab", "enter", "o", "r",
    ];
    if state.is_zoomed() {
        keys.extend(["backspace", "esc", "a"]);
    }
    keys.into_iter().map(String::from).collect()
}

/// The atlas for a view of `width` by `height` cells, with `here` (the folder the browser is in)
/// marked.
pub fn build(state: &AtlasState, here: &Path, width: u16, height: u16) -> Canvas {
    let places = places(width, height);
    let mut items = Vec::new();
    let mut budget = Budget { items: 0, chars: 0 };
    if width >= 2 && height >= 2 {
        items.push(Part::Frame {
            at: cells(places.frame),
            paint: Paint::default(),
            title: Some(title(state, width)),
        });
    }
    let line = |rect: Rect, label: String, tone: Tone| {
        let cut = label.chars().take(usize::from(rect.width)).collect();
        (rect.width > 0 && rect.height > 0).then(|| text(rect, cut, tone))
    };
    items.extend(line(places.header, header_text(state), Tone::Accent));

    let tiles = state.tiles(places.map);
    if tiles.is_empty() {
        let note = empty_note(state).to_string();
        items.extend(line(
            Rect {
                height: places.map.height.min(1),
                ..places.map
            },
            note,
            Tone::Dim,
        ));
    } else {
        budget.spend(&items);
        items.extend(map_parts(state, here, &tiles, &mut budget));
    }

    if places.footer.height > 0 {
        let toggle = match state.zoomed().map(|view| view.measure()) {
            Some(Measure::OnDisk) => "  a apparent size",
            Some(Measure::Apparent) => "  a size on disk",
            None => "",
        };
        let hints = format!(
            "arrows move  Enter zoom in  Backspace out  o open  Tab next{toggle}  r rescan  * you are here"
        );
        items.extend(line(places.footer, hints, Tone::Dim));
    }
    Canvas {
        width,
        height,
        items,
        claims: claims(state),
        hint: Some(HINT.to_string()),
    }
}

/// The parts of every tile, the chosen one first and then the biggest, so a map too crowded for
/// the canvas's limits loses its smallest tiles and never the one the cursor is on.
fn map_parts(state: &AtlasState, here: &Path, tiles: &[Tile], budget: &mut Budget) -> Vec<Part> {
    let (chosen, pin) = (state.selected(), state.pin(here));
    let area = |tile: &Tile| u32::from(tile.rect.width) * u32::from(tile.rect.height);
    let is_chosen = |tile: &Tile| matches!(tile.item, Item::Entry(i) if chosen == Some(i));
    let mut order: Vec<&Tile> = tiles.iter().collect();
    order.sort_by_key(|tile| (!is_chosen(tile), std::cmp::Reverse(area(tile))));
    let mut parts = Vec::new();
    for tile in order {
        let rows = painted(tile.rect).height;
        let width = usize::from(painted(tile.rect).width.saturating_sub(2));
        let spec = match tile.item {
            Item::Small(count) => small_spec(count),
            Item::Entry(i) => {
                let pinned = pin == Some(i);
                match state.zoomed() {
                    None => volume_spec(&state.devices()[i], pinned, rows, width),
                    Some(view) => row_spec(&view.rows()[i], pinned, view.measure(), view.total()),
                }
            }
        };
        let made = tile_parts(tile.rect, &spec, is_chosen(tile));
        let chars = made.iter().map(chars_of).sum();
        if budget.fits(made.len(), chars) {
            budget.spend(&made);
            parts.extend(made);
        }
    }
    parts
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use plugin_api::{Mods, SetCanvas, Surface};
    use proptest::prelude::*;
    use shared::LocalVfs;
    use shared::devices::Kind as DeviceKind;

    use super::*;
    use crate::state::Machine;

    struct Offline;

    impl Machine for Offline {
        fn scan(&self) -> Vec<Device> {
            Vec::new()
        }
    }

    fn volume(node: &str, mount: Option<&str>, total: u64, free: u64) -> Device {
        Device {
            node: node.into(),
            disk: node.trim_end_matches(char::is_numeric).into(),
            label: Some(node.to_uppercase()),
            fs: Some("ext4".into()),
            mount: mount.map(PathBuf::from),
            total,
            free,
            kind: DeviceKind::Internal,
        }
    }

    fn atlas(devices: Vec<Device>) -> AtlasState {
        let mut state = AtlasState::with_devices(Arc::new(LocalVfs), Arc::new(Offline), devices);
        state.poll(Instant::now(), Path::new("/"));
        state
    }

    fn texts(canvas: &Canvas) -> Vec<&str> {
        canvas
            .items
            .iter()
            .filter_map(|part| match part {
                Part::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn accepted(canvas: Canvas) -> Canvas {
        SetCanvas {
            surface: Surface::Atlas,
            canvas: Some(canvas),
        }
        .checked()
        .expect("the host accepts it")
        .canvas
        .expect("it is not empty")
    }

    #[test]
    fn the_map_sits_between_the_totals_line_and_the_hints_inside_a_frame() {
        let places = places(80, 24);
        assert_eq!(places.frame, Rect::new(0, 0, 80, 24));
        assert_eq!(places.header, Rect::new(1, 1, 78, 1));
        assert_eq!(places.map, Rect::new(1, 2, 78, 20));
        assert_eq!(places.footer, Rect::new(1, 22, 78, 1));
        // Too short for the hints line: the map takes the row.
        let short = super::places(80, 5);
        assert_eq!(short.footer.height, 0);
        assert_eq!(short.map, Rect::new(1, 2, 78, 2));
    }

    #[test]
    fn a_volume_says_its_name_fill_mount_and_free_space() {
        let state = atlas(vec![
            volume("sda2", Some("/"), 900_000_000, 100_000_000),
            volume("sdb1", None, 100_000_000, 50_000_000),
        ]);
        let canvas = accepted(build(&state, Path::new("/home/me"), 100, 30));
        let said = texts(&canvas).join("\n");
        assert!(
            said.contains("* SDA2"),
            "the volume holding the folder is pinned: {said}"
        );
        assert!(said.contains("88% of"), "{said}");
        assert!(said.contains("not mounted"), "{said}");
        assert!(said.contains("2 volumes"), "{said}");
        assert_eq!(canvas.hint.as_deref(), Some(HINT));
        assert!(
            texts(&canvas)[0].starts_with("2 volumes"),
            "the totals line comes first"
        );
    }

    #[test]
    fn a_nearly_full_volume_is_danger_and_an_unmounted_one_is_dim() {
        let state = atlas(vec![
            volume("full", Some("/"), 1000, 20),
            volume("away", None, 500, 400),
        ]);
        let canvas = accepted(build(&state, Path::new("/"), 100, 30));
        let tones: Vec<Tone> = canvas
            .items
            .iter()
            .filter_map(|part| match part {
                Part::Block { paint, .. } => Some(paint.tone),
                _ => None,
            })
            .collect();
        assert!(tones.contains(&Tone::Danger), "{tones:?}");
        assert!(tones.contains(&Tone::Dim), "{tones:?}");
    }

    #[test]
    fn the_chosen_tile_is_marked_and_framed() {
        let state = atlas(vec![
            volume("a", Some("/"), 800, 100),
            volume("b", Some("/b"), 200, 100),
        ]);
        let canvas = accepted(build(&state, Path::new("/"), 100, 30));
        let chosen_blocks = canvas
            .items
            .iter()
            .filter(|part| matches!(part, Part::Block { paint, .. } if paint.mark == Mark::Chosen))
            .count();
        assert!(chosen_blocks >= 1);
        let frames = canvas
            .items
            .iter()
            .filter(|part| matches!(part, Part::Frame { paint, .. } if paint.mark == Mark::Chosen))
            .count();
        assert_eq!(frames, 1, "one frame round the one chosen tile");
    }

    #[test]
    fn keys_the_atlas_wants_depend_on_whether_it_is_zoomed() {
        let root =
            std::env::temp_dir().join(format!("minuteman-atlas-canvas-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/big"), vec![b'x'; 300_000]).unwrap();
        let mount = root.to_str().unwrap();
        let mut state = atlas(vec![volume("sda1", Some(mount), 1_000_000, 400_000)]);
        let map = places(100, 30).map;

        let before = build(&state, Path::new("/"), 100, 30).claims;
        assert!(before.iter().any(|k| k == "enter"));
        assert!(
            !before
                .iter()
                .any(|k| k == "esc" || k == "backspace" || k == "a"),
            "at the map of volumes `Esc` closes the view and `a` is the appearance popup: {before:?}"
        );

        state.key("enter", Mods::default(), map);
        let deadline = Instant::now() + Duration::from_secs(10);
        while state.zoomed().is_none_or(|view| view.end().is_none()) {
            state.poll(Instant::now(), Path::new("/"));
            assert!(Instant::now() < deadline, "the scan never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
        let canvas = accepted(build(&state, Path::new("/"), 100, 30));
        for key in ["esc", "backspace", "a"] {
            assert!(canvas.claims.iter().any(|k| k == key), "{key}");
        }
        let said = texts(&canvas).join("\n");
        assert!(
            said.contains("sub/"),
            "a folder is named with a slash: {said}"
        );
        assert!(said.contains("on disk"), "{said}");
        assert!(matches!(
            &canvas.items[0],
            Part::Frame { title: Some(title), .. } if title.starts_with("atlas: SDA1 ")
        ));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_view_with_nothing_to_map_says_why() {
        let state = AtlasState::new(Arc::new(LocalVfs), Arc::new(Offline));
        let canvas = accepted(build(&state, Path::new("/"), 60, 12));
        assert!(texts(&canvas).contains(&"looking for volumes..."));
        let state = atlas(vec![]);
        let canvas = accepted(build(&state, Path::new("/"), 60, 12));
        assert!(texts(&canvas).contains(&"no volume with a known size"));
    }

    #[test]
    fn a_crowd_of_tiny_volumes_still_fits_the_limits_and_keeps_the_chosen_one() {
        let volumes: Vec<Device> = (0..6000)
            .map(|i| volume(&format!("v{i:05}"), None, 1000 + i, 10))
            .collect();
        let state = atlas(volumes);
        let canvas = accepted(build(&state, Path::new("/"), 400, 150));
        assert!(canvas.items.len() <= plugin_api::MAX_CANVAS_ITEMS);
        let chosen = canvas
            .items
            .iter()
            .any(|part| matches!(part, Part::Block { paint, .. } if paint.mark == Mark::Chosen));
        assert!(chosen, "the chosen tile is never the one dropped");
    }

    #[test]
    fn free_rows_leaves_the_unused_part_at_the_top() {
        assert_eq!(free_rows(10, 0), 10);
        assert_eq!(free_rows(10, 100), 0);
        assert_eq!(free_rows(10, 50), 5);
        assert_eq!(free_rows(0, 50), 0);
        assert_eq!(free_rows(10, 250), 0, "a percent over 100 is clamped");
    }

    #[test]
    fn elide_left_keeps_the_end_of_a_path() {
        assert_eq!(elide_left("/short", 20), "/short");
        let cut = elide_left("/very/long/path/to/the/folder", 12);
        assert_eq!(cut.chars().count(), 12);
        assert!(cut.starts_with("...") && cut.ends_with("folder"), "{cut}");
        assert_eq!(elide_left("abcdef", 2), "...", "no room: only the mark");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(40))]

        /// For any size of view and any volumes, the canvas the host would receive passes the
        /// host's own check, and nothing it draws lies outside the map or the frame.
        #[test]
        fn any_view_of_any_volumes_passes_the_hosts_check(
            width in 1u16..300,
            height in 1u16..120,
            volumes in proptest::collection::vec((1u64..1_000_000_000, 0u64..1_000_000_000, any::<bool>()), 0..40),
        ) {
            let devices: Vec<Device> = volumes
                .iter()
                .enumerate()
                .map(|(i, (total, free, mounted))| {
                    volume(&format!("v{i}"), mounted.then_some("/"), *total, (*free).min(*total))
                })
                .collect();
            let state = atlas(devices);
            let canvas = build(&state, Path::new("/"), width, height);
            let checked = SetCanvas { surface: Surface::Atlas, canvas: Some(canvas) }.checked();
            prop_assert!(checked.is_ok(), "{:?}", checked.err());
        }

        #[test]
        fn a_share_is_a_percent_and_a_fill_never_exceeds_its_rows(
            size in any::<u64>(), total in any::<u64>(), rows in 0u16..500, percent in 0u8..=255,
        ) {
            prop_assert!(share_percent(size, total) <= 100);
            prop_assert!(free_rows(rows, percent) <= rows);
        }

        #[test]
        fn an_elided_path_never_exceeds_its_width(text in "\\PC{0,60}", width in 3usize..40) {
            prop_assert!(elide_left(&text, width).chars().count() <= width);
        }
    }
}
