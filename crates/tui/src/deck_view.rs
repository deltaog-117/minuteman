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

//! Draws the deck: a rail of devices at the left of the columns, the way a file manager's places
//! panel is. The devices are grouped under a heading each (internal, removable, network); a device
//! is two lines, its name over a gauge of how full it is (or, while files are carried, whether
//! they fit). A bar at the left edge of the row holding the folder being browsed says "you are
//! here"; with the keys on the rail, the chosen row is filled. A disk that is reading or writing
//! shows a small live sparkline beside its name. A device that has just appeared grows in from
//! nothing.
//!
//! The layout (`rows`) is one function that both the drawing and the mouse ask, so a click lands
//! on the row that was drawn there.

use std::path::Path;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Margin, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use shared::devices::{Device, Kind};
use theming::Config;

use crate::cargo_fit::{self, Verdict};
use crate::deck::DeckState;
use crate::glyphs;
use crate::hud::{fit_width, format_size, pad_to, text_width};
use crate::style;

/// The lines a device takes: its name, then how full it is.
const ROW_HEIGHT: u16 = 2;

/// The least width at which a row has room for what is in it.
const CONTENT_MIN: u16 = 14;

/// The width of the sparkline beside the name of a busy disk.
const PULSE_WIDTH: usize = 3;

/// A sparkline never scales its tallest bar below this many bytes a second, so a trickle of
/// background writes does not look like a flood.
const SPARK_FLOOR: u64 = 256 * 1024;

/// What a line of the rail is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// The title over a group of devices.
    Heading(Kind),
    /// A device, by index into the deck's devices.
    Device(usize),
}

/// What is drawn in the rail's frame, top to bottom, and whether more is hidden past either end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rows {
    pub placed: Vec<(Slot, Rect)>,
    pub more_above: bool,
    pub more_below: bool,
}

/// Where the rail's lines go: inside its one-cell frame.
fn inner(area: Rect) -> Rect {
    area.inner(Margin::new(1, 1))
}

/// The lines of a device that has grown `grown` (0 to 1) of the way in: never none, so a new
/// device is seen the moment it appears.
fn grown_height(grown: f64) -> u16 {
    // `grown` is between 0 and 1 and the height is at most `ROW_HEIGHT`, so this is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lines = (f64::from(ROW_HEIGHT) * grown).ceil() as u16;
    lines.clamp(1, ROW_HEIGHT)
}

/// The first item to draw so the items `top` to `bottom` are on screen in `room` lines with as
/// little scrolling as possible, and no blank space left at the end when the list could fill it.
fn follow(heights: &[u16], top: usize, bottom: usize, first: usize, room: u16) -> usize {
    if heights.is_empty() {
        return 0;
    }
    let bottom = bottom.min(heights.len() - 1);
    let top = top.min(bottom);
    let span =
        |from: usize, to: usize| -> u32 { heights[from..=to].iter().map(|&h| u32::from(h)).sum() };
    let room = u32::from(room);
    let mut first = first.min(top);
    while first < top && span(first, bottom) > room {
        first += 1;
    }
    while first > 0 && span(first - 1, heights.len() - 1) <= room {
        first -= 1;
    }
    first
}

/// What is on each line of the rail in `area`, from the top. A device that is still sliding in is
/// only as tall as it has grown; the one at the bottom is cut to what room is left.
pub fn rows(area: Rect, deck: &DeckState, now: Instant) -> Rows {
    let body = inner(area);
    let mut items: Vec<(Slot, u16)> = Vec::new();
    let mut group = None;
    for (index, device) in deck.devices().iter().enumerate() {
        if group != Some(device.kind) {
            group = Some(device.kind);
            items.push((Slot::Heading(device.kind), 1));
        }
        items.push((Slot::Device(index), grown_height(deck.slide(device, now))));
    }
    let chosen = items
        .iter()
        .position(|&(slot, _)| slot == Slot::Device(deck.selected()))
        .unwrap_or(0);
    // The heading over the first device of a group stays with it when it is scrolled to.
    let top = match chosen.checked_sub(1).map(|above| items[above].0) {
        Some(Slot::Heading(_)) => chosen - 1,
        _ => chosen,
    };
    let heights: Vec<u16> = items.iter().map(|&(_, height)| height).collect();
    let first = follow(&heights, top, chosen, deck.first_card().get(), body.height);
    deck.first_card().set(first);

    let mut placed = Vec::new();
    let mut y = body.y;
    for &(slot, height) in items.iter().skip(first) {
        let left = body.bottom().saturating_sub(y);
        if left == 0 {
            break;
        }
        let height = height.min(left);
        placed.push((slot, Rect::new(body.x, y, body.width, height)));
        y += height;
    }
    let shown = placed.len();
    Rows {
        placed,
        more_above: first > 0,
        more_below: first + shown < items.len(),
    }
}

/// The device whose row is under `pointer`, as drawn in `area`.
pub fn card_at(area: Rect, deck: &DeckState, pointer: Position) -> Option<usize> {
    rows(area, deck, Instant::now())
        .placed
        .into_iter()
        .find_map(|(slot, rect)| match slot {
            Slot::Device(index) if rect.contains(pointer) => Some(index),
            _ => None,
        })
}

fn icon(kind: Kind, ascii: bool) -> &'static str {
    match (kind, ascii) {
        (Kind::Internal, false) => "▣",
        (Kind::Removable, false) => "⏏",
        (Kind::Network, false) => "⇄",
        (Kind::Internal, true) => "[]",
        (Kind::Removable, true) => "^",
        (Kind::Network, true) => "<>",
    }
}

fn group_title(kind: Kind) -> &'static str {
    match kind {
        Kind::Internal => "Internal",
        Kind::Removable => "Removable",
        Kind::Network => "Network",
    }
}

/// The bars of a sparkline, lowest first.
fn levels(ascii: bool) -> &'static [char] {
    if ascii {
        &['_', '.', ':', '-', '=', '+', '*', '#']
    } else {
        &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']
    }
}

/// `history` drawn in at most `width` cells, newest last, as tall as it is against the busiest
/// moment shown (or `SPARK_FLOOR`, whichever is more).
pub fn sparkline(history: &[u64], width: usize, ascii: bool) -> String {
    let bars = levels(ascii);
    let shown = &history[history.len().saturating_sub(width)..];
    let tallest = shown.iter().copied().max().unwrap_or(0).max(SPARK_FLOOR);
    // Before there is history the line is a flat baseline, so an idle disk still reads as a
    // meter that is watching rather than as a gap.
    let mut line: String = bars[0]
        .to_string()
        .repeat(width.saturating_sub(shown.len()));
    for &sample in shown {
        let step = u128::from(sample) * (bars.len() as u128 - 1) / u128::from(tallest);
        // Below the number of bars by construction.
        line.push(bars[usize::try_from(step).unwrap_or(0).min(bars.len() - 1)]);
    }
    line
}

/// A gauge `width` cells wide with `percent` of it filled.
fn gauge(percent: u8, width: usize, on: &str, off: &str) -> (String, String) {
    let filled = (width * usize::from(percent.min(100)) + 50) / 100;
    (on.repeat(filled), off.repeat(width - filled))
}

pub(crate) fn rate_text(read: u64, written: u64) -> String {
    if read == 0 && written == 0 {
        return "idle".to_string();
    }
    format!("r {} w {}", format_size(read), format_size(written))
}

pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    deck: &DeckState,
    current_dir: &Path,
    config: &Config,
    now: Instant,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = &config.theme;
    let dim = Style::default().fg(style::color(&theme.status_fg));
    let block = style::themed_block(config, "devices", deck.focused());
    if deck.devices().is_empty() {
        let note = match deck.missing() {
            Some(why) => why,
            None if deck.has_scanned() => "no devices found",
            None => "looking for devices…",
        };
        frame.render_widget(
            Paragraph::new(Span::styled(note, dim))
                .wrap(Wrap { trim: true })
                .block(block),
            area,
        );
        return;
    }
    frame.render_widget(block, area);
    let g = glyphs::of(config);
    let ascii = g.gauge_on.is_ascii();
    let here = deck.holding(current_dir);
    let layout = rows(area, deck, now);
    for &(slot, rect) in &layout.placed {
        if rect.width < CONTENT_MIN {
            continue;
        }
        match slot {
            Slot::Heading(kind) => {
                let rule = if ascii { "-" } else { "─" };
                let title = format!(" {} ", group_title(kind));
                let fill = usize::from(rect.width).saturating_sub(text_width(&title));
                let line = Line::from(vec![
                    Span::styled(title, dim.add_modifier(Modifier::BOLD)),
                    Span::styled(rule.repeat(fill), dim),
                ]);
                frame.render_widget(Paragraph::new(line), rect);
            }
            Slot::Device(index) => {
                let verdict = deck.verdict(index);
                let mut base = Style::default();
                if deck.focused() && index == deck.selected() {
                    base = base.bg(style::color(&theme.selection_bg));
                }
                // A device the load cannot go on is dimmed, so what can take it stands out at a
                // glance.
                if matches!(
                    verdict,
                    Some(Verdict::Short { .. } | Verdict::TooBig { .. })
                ) {
                    base = base.add_modifier(Modifier::DIM);
                }
                let lines = row_lines(
                    &deck.devices()[index],
                    deck,
                    index,
                    verdict.as_ref(),
                    here == Some(index),
                    usize::from(rect.width),
                    config,
                    ascii,
                );
                frame.render_widget(Paragraph::new(lines).style(base), rect);
            }
        }
    }
    // A mark on the frame where devices are hidden beyond it.
    let accent = Style::default().fg(style::color(&theme.accent_fg));
    let body = inner(area);
    let edge = area.right().saturating_sub(1);
    if layout.more_above {
        frame.render_widget(
            Paragraph::new(Span::styled("▴", accent)),
            Rect::new(edge, body.y, 1, 1),
        );
    }
    if layout.more_below {
        frame.render_widget(
            Paragraph::new(Span::styled("▾", accent)),
            Rect::new(edge, body.bottom().saturating_sub(1), 1, 1),
        );
    }
}

/// The two lines of a device: its name, then how full it is.
#[allow(clippy::too_many_arguments)]
fn row_lines(
    device: &Device,
    deck: &DeckState,
    index: usize,
    verdict: Option<&Verdict>,
    here: bool,
    width: usize,
    config: &Config,
    ascii: bool,
) -> Vec<Line<'static>> {
    let theme = &config.theme;
    let g = glyphs::of(config);
    let dim = Style::default().fg(style::color(&theme.status_fg));
    let accent = Style::default().fg(style::color(&theme.accent_fg));
    let danger = Style::default().fg(style::color(&theme.danger_fg));

    let mark = if here {
        Span::styled(if ascii { "|" } else { "▌" }, accent)
    } else {
        Span::raw(" ")
    };
    let symbol = icon(device.kind, ascii);
    let indent = 1 + text_width(symbol) + 1;
    // A mount or an eject started from the chosen device is still running.
    let working = if index == deck.selected() && deck.is_working() {
        " …"
    } else {
        ""
    };

    // A disk that is reading or writing right now pulses beside its name.
    let rates = deck.rates(device);
    let busy = device.is_mounted() && (rates.read > 0 || rates.written > 0);
    let pulse = (busy && width >= indent + 8).then(|| {
        sparkline(
            &deck.history(device),
            PULSE_WIDTH.min(shared::devices::activity::HISTORY),
            ascii,
        )
    });
    let name_width = width - indent - pulse.as_ref().map_or(0, |p| text_width(p) + 1);
    let name = pad_to(
        &fit_width(&format!("{}{working}", device.title()), name_width),
        name_width,
    );
    let mut first = vec![
        mark,
        Span::styled(symbol, if device.is_mounted() { accent } else { dim }),
        Span::raw(" "),
        Span::raw(name),
    ];
    if let Some(pulse) = pulse {
        first.push(Span::raw(" "));
        first.push(Span::styled(pulse, accent));
    }

    let room = width - indent;
    let detail = if let Some(verdict) = verdict {
        let style = match verdict {
            Verdict::Fits { .. } | Verdict::SameDisk => accent,
            Verdict::Short { .. } | Verdict::TooBig { .. } => danger,
            Verdict::Tight { .. } | Verdict::Unmounted | Verdict::Unsure => dim,
        };
        vec![Span::styled(
            fit_width(&cargo_fit::card_text(verdict, ascii), room),
            style,
        )]
    } else {
        match (device.used_percent(), device.is_mounted()) {
            (Some(percent), true) => {
                let free = format!(" {percent:>3}% {} free", format_size(device.free));
                let bar = room.saturating_sub(text_width(&free));
                if bar >= 4 {
                    let (on, off) = gauge(percent, bar, g.gauge_on, g.gauge_off);
                    let fill = if percent >= 90 { danger } else { accent };
                    vec![
                        Span::styled(on, fill),
                        Span::styled(off, dim),
                        Span::raw(free),
                    ]
                } else {
                    vec![Span::raw(fit_width(free.trim_start(), room))]
                }
            }
            (_, false) if device.total > 0 => vec![Span::styled(
                fit_width(
                    &format!("not mounted · {}", format_size(device.total)),
                    room,
                ),
                dim,
            )],
            _ => vec![Span::styled("size unknown", dim)],
        }
    };
    let mut second = vec![Span::raw(" ".repeat(indent))];
    second.extend(detail);
    vec![Line::from(first), Line::from(second)]
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn device(node: &str, label: &str, mount: Option<&str>, kind: Kind, free: u64) -> Device {
        Device {
            node: node.into(),
            disk: node.trim_end_matches(char::is_numeric).into(),
            label: Some(label.into()).filter(|l: &String| !l.is_empty()),
            fs: Some("ext4".into()),
            mount: mount.map(PathBuf::from),
            total: 1000,
            free,
            kind,
        }
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn deck(devices: Vec<Device>) -> DeckState {
        DeckState::with_devices(devices)
    }

    fn many(n: usize) -> Vec<Device> {
        (0..n)
            .map(|i| {
                device(
                    &format!("sd{i}1"),
                    &format!("Disk {i}"),
                    Some(&format!("/mnt/{i}")),
                    Kind::Internal,
                    500,
                )
            })
            .collect()
    }

    fn drawn(deck: &DeckState, here: &str, width: u16, height: u16) -> String {
        let config = Config::from_sources(None, None, None);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    deck,
                    Path::new(here),
                    &config,
                    Instant::now(),
                );
            })
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
    fn the_rail_groups_devices_under_headings_and_stacks_two_lines_each() {
        let deck = deck(vec![
            device("sda1", "", Some("/"), Kind::Internal, 250),
            device("sdb1", "STICK", Some("/m/STICK"), Kind::Removable, 900),
        ]);
        let area = Rect::new(2, 3, 26, 12);
        let placed = rows(area, &deck, Instant::now()).placed;
        let slots: Vec<Slot> = placed.iter().map(|&(slot, _)| slot).collect();
        assert_eq!(
            slots,
            [
                Slot::Heading(Kind::Internal),
                Slot::Device(0),
                Slot::Heading(Kind::Removable),
                Slot::Device(1)
            ]
        );
        let ys: Vec<u16> = placed.iter().map(|(_, r)| r.y).collect();
        assert_eq!(
            ys,
            [4, 5, 7, 8],
            "inside the frame, a heading is one line and a device two"
        );
        assert!(placed.iter().all(|(_, r)| r.x == 3 && r.width == 24));
    }

    #[test]
    fn scrolling_follows_the_selected_item_and_stays_put_otherwise() {
        let heights = [1, 2, 2, 2, 2];
        assert_eq!(follow(&heights, 0, 0, 0, 5), 0);
        assert_eq!(follow(&heights, 2, 2, 0, 5), 0, "still on screen");
        assert_eq!(
            follow(&heights, 3, 3, 0, 5),
            2,
            "one past the edge scrolls to fit it"
        );
        assert_eq!(follow(&heights, 1, 1, 3, 5), 1, "back up");
        assert_eq!(
            follow(&heights, 2, 2, 99, 5),
            2,
            "a stale position is pulled in"
        );
        assert_eq!(follow(&heights, 0, 4, 0, 99), 0, "all of it fits");
        assert_eq!(follow(&[], 0, 0, 0, 5), 0);
    }

    #[test]
    fn a_heading_is_kept_with_the_first_device_scrolled_to() {
        let mut devices = many(4);
        devices.push(device(
            "sdz1",
            "STICK",
            Some("/m/STICK"),
            Kind::Removable,
            900,
        ));
        let mut deck = deck(devices);
        deck.key(press(KeyCode::Tab), true);
        for _ in 0..4 {
            deck.key(press(KeyCode::Down), true);
        }
        let layout = rows(Rect::new(0, 0, 26, 8), &deck, Instant::now());
        let slots: Vec<Slot> = layout.placed.iter().map(|&(slot, _)| slot).collect();
        let heading = slots
            .iter()
            .position(|&s| s == Slot::Heading(Kind::Removable));
        assert_eq!(
            heading.map(|at| slots[at + 1]),
            Some(Slot::Device(4)),
            "{slots:?}"
        );
        assert!(layout.more_above);
        assert!(!layout.more_below);
    }

    #[test]
    fn a_click_finds_the_device_that_was_drawn_there() {
        let deck = deck(many(3));
        let area = Rect::new(0, 0, 26, 20);
        assert_eq!(card_at(area, &deck, Position::new(3, 0)), None, "the frame");
        assert_eq!(
            card_at(area, &deck, Position::new(3, 1)),
            None,
            "the heading"
        );
        assert_eq!(card_at(area, &deck, Position::new(3, 2)), Some(0));
        assert_eq!(
            card_at(area, &deck, Position::new(3, 3)),
            Some(0),
            "its second line"
        );
        assert_eq!(card_at(area, &deck, Position::new(3, 4)), Some(1));
        assert_eq!(card_at(area, &deck, Position::new(3, 19)), None);
    }

    #[test]
    fn a_device_sliding_in_is_short_and_grows_to_its_full_height() {
        let mut deck = deck(many(1));
        let now = Instant::now();
        deck.mark_arrived("sd01", now);
        let area = Rect::new(0, 0, 26, 12);
        let at = |t: Duration| rows(area, &deck, now + t).placed[1].1.height;
        assert_eq!(at(Duration::ZERO), 1);
        assert_eq!(at(crate::deck::SLIDE_IN), ROW_HEIGHT);
    }

    #[test]
    fn a_sparkline_is_as_tall_as_the_activity_and_pads_a_short_history() {
        let line = sparkline(&[0, SPARK_FLOOR, SPARK_FLOOR * 2], 5, false);
        assert_eq!(line.chars().count(), 5);
        assert!(line.starts_with("▁▁"), "{line:?}");
        let bars: Vec<char> = line.chars().skip(2).collect();
        assert_eq!(bars[0], '▁');
        assert_eq!(bars[2], '█');
        assert!(bars[1] > bars[0] && bars[1] < bars[2]);
        assert!(sparkline(&[10], 4, true).is_ascii());
        assert_eq!(sparkline(&[], 0, false), "");
    }

    #[test]
    fn a_trickle_does_not_fill_the_sparkline() {
        assert_eq!(sparkline(&[1000], 1, false), "▁");
    }

    #[test]
    fn a_gauge_fills_in_proportion_and_always_adds_up_to_its_width() {
        for (percent, width) in [(0, 10), (50, 10), (99, 10), (100, 10), (33, 7), (250, 4)] {
            let (on, off) = gauge(percent, width, "#", "-");
            assert_eq!(on.len() + off.len(), width, "{percent}% of {width}");
        }
        assert_eq!(gauge(50, 10, "#", "-").0.len(), 5);
    }

    #[test]
    fn each_device_shows_its_name_how_full_and_which_group_it_is_in() {
        let deck = deck(vec![
            device("sda1", "", Some("/"), Kind::Internal, 250),
            device(
                "sdb1",
                "HOLIDAY",
                Some("/run/media/me/HOLIDAY"),
                Kind::Removable,
                900,
            ),
            device("sdc1", "BACKUP", None, Kind::Removable, 0),
        ]);
        let shown = drawn(&deck, "/run/media/me/HOLIDAY/photos", 26, 14);
        for text in [
            "devices",
            "Internal",
            "Removable",
            "System",
            "HOLIDAY",
            "BACKUP",
            "75%",
            "10%",
            "not mounted",
        ] {
            assert!(shown.contains(text), "{text:?} missing:\n{shown}");
        }
        assert!(!shown.contains("Network"), "no empty group:\n{shown}");
    }

    #[test]
    fn the_device_holding_the_browsed_folder_has_the_bar_at_its_edge() {
        let deck = deck(many(2));
        let shown = drawn(&deck, "/mnt/1/deep", 26, 10);
        let lines: Vec<&str> = shown.lines().collect();
        // Row 0 is the frame, row 1 the heading, rows 2-3 the first device, 4-5 the second.
        assert!(lines[4].chars().nth(1) == Some('▌'), "{shown}");
        assert!(!lines[2].contains('▌'), "{shown}");
    }

    #[test]
    fn an_empty_deck_says_it_is_looking_and_then_that_it_found_nothing() {
        let mut deck = deck(Vec::new());
        deck.set_scanned(false);
        assert!(drawn(&deck, "/", 26, 10).contains("looking for devices"));
        deck.set_scanned(true);
        assert!(drawn(&deck, "/", 26, 10).contains("no devices found"));
    }

    #[test]
    fn while_files_are_carried_each_device_says_whether_they_fit() {
        use crate::cargo_fit::{Carry, Load};
        let mut deck = deck(vec![
            device("sda1", "", Some("/"), Kind::Internal, 900),
            device(
                "sdb1",
                "HOLIDAY",
                Some("/run/media/me/HOLIDAY"),
                Kind::Removable,
                10,
            ),
            device("sdc1", "BACKUP", None, Kind::Removable, 0),
        ]);
        for d in 0..2 {
            let device = &mut deck.devices_mut()[d];
            device.total = 1 << 40;
            device.free = [1u64 << 39, 1 << 20][d];
        }
        assert!(
            !drawn(&deck, "/", 26, 14).contains("fits"),
            "nothing carried yet"
        );
        deck.set_load(Some(Load {
            bytes: 1 << 30,
            exact: true,
            largest: 1 << 20,
            carry: Carry::Copy,
            source: "/home/me/photos".into(),
        }));
        let shown = drawn(&deck, "/", 26, 14);
        for text in ["✓ fits", "✗ needs", "mount to check"] {
            assert!(shown.contains(text), "{text:?} missing:\n{shown}");
        }
        assert!(
            !shown.contains("free"),
            "the verdict takes the gauge's line:\n{shown}"
        );
    }

    #[test]
    fn an_idle_disk_shows_no_pulse() {
        let deck = deck(many(1));
        let shown = drawn(&deck, "/mnt/0", 26, 8);
        assert!(!shown.contains('█'), "idle:\n{shown}");
    }

    #[test]
    fn it_draws_at_any_size_without_panicking() {
        let deck = deck(many(6));
        for (w, h) in [
            (0, 0),
            (1, 1),
            (3, 5),
            (10, 2),
            (13, 6),
            (14, 6),
            (26, 3),
            (26, 40),
            (80, 6),
        ] {
            drawn(&deck, "/mnt/2", w, h);
        }
    }

    #[test]
    fn hidden_devices_are_marked_at_the_edges() {
        let deck = deck(many(6));
        let shown = drawn(&deck, "/mnt/0", 26, 8);
        assert!(shown.contains('▾') && !shown.contains('▴'), "{shown}");
    }
}
