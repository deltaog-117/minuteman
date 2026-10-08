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

//! Draws the deck: a strip of cards above the columns, one per device, each with its name, where
//! it is mounted, a gauge of how full it is and a sparkline of how busy it has been. The card
//! holding the folder being browsed has the bright frame ("you are here"); with the keys on the
//! deck, the chosen card is filled. A card that has just appeared grows in from nothing.
//!
//! The layout (`cards`) is one function that both the drawing and the mouse ask, so a click lands
//! on the card that was drawn there.

use std::path::Path;
use std::time::Instant;

use devices::{Device, Kind};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use theming::Config;

use crate::cargo_fit::{self, Verdict};
use crate::deck::DeckState;
use crate::glyphs;
use crate::hud::{fit_width, format_size, pad_to, text_width};
use crate::style;
use crate::trash_view::elide_left;

/// The width of a full card.
pub const CARD_WIDTH: u16 = 28;

/// Blank columns between two cards.
const GAP: u16 = 1;

/// The least width worth drawing a card at while it grows in.
const MIN_CARD: u16 = 4;

/// The least width at which a card has room for what is in it.
const CONTENT_MIN: u16 = 16;

/// A sparkline never scales its tallest bar below this many bytes a second, so a trickle of
/// background writes does not look like a flood.
const SPARK_FLOOR: u64 = 256 * 1024;

/// How many cards fit across `width`.
fn capacity(width: u16) -> usize {
    usize::from(((width + GAP) / (CARD_WIDTH + GAP)).max(1))
}

/// The first card to draw so the selected one is on screen with as little scrolling as possible.
fn follow(selected: usize, first: usize, count: usize, capacity: usize) -> usize {
    let mut first = first.min(count.saturating_sub(capacity));
    if selected < first {
        first = selected;
    } else if selected >= first + capacity {
        first = selected + 1 - capacity;
    }
    first
}

/// Where each visible card is and which device it is, left to right from `area`'s edge. A card
/// that is still sliding in is only as wide as it has grown; the one at the edge is cut to what
/// room is left, or left out if that is too little to read.
pub fn cards(area: Rect, deck: &DeckState, now: Instant) -> Vec<(usize, Rect)> {
    let devices = deck.devices();
    let first = follow(
        deck.selected(),
        deck.first_card().get(),
        devices.len(),
        capacity(area.width),
    );
    deck.first_card().set(first);
    let mut placed = Vec::new();
    let mut x = area.x;
    for (index, device) in devices.iter().enumerate().skip(first) {
        let left = area.right().saturating_sub(x);
        if left < MIN_CARD {
            break;
        }
        let grown = deck.slide(device, now);
        // `grown` is between 0 and 1 and the width is at most `CARD_WIDTH`, so this is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let wanted = (f64::from(CARD_WIDTH) * grown).round() as u16;
        let width = wanted.max(MIN_CARD);
        // A card that does not fit is cut to the room left only if that is enough to read it;
        // a stub with nothing in it is left out, and the mark at the edge says there are more.
        if width > left && left < CONTENT_MIN {
            break;
        }
        let width = width.min(left);
        placed.push((index, Rect::new(x, area.y, width, area.height)));
        x += width + GAP;
    }
    placed
}

/// The device whose card is under `pointer`, as drawn in `area`.
pub fn card_at(area: Rect, deck: &DeckState, pointer: Position) -> Option<usize> {
    cards(area, deck, Instant::now())
        .into_iter()
        .find(|(_, rect)| rect.contains(pointer))
        .map(|(index, _)| index)
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
    if deck.devices().is_empty() {
        let note = if deck.has_scanned() {
            "no devices found"
        } else {
            "looking for devices…"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(note, dim))
                .block(style::themed_block(config, "devices", false)),
            area,
        );
        return;
    }
    let g = glyphs::of(config);
    let ascii = g.gauge_on.is_ascii();
    let here = deck.holding(current_dir);
    let placed = cards(area, deck, now);
    for &(index, rect) in &placed {
        let device = &deck.devices()[index];
        let chosen = deck.focused() && index == deck.selected();
        // A mount or an eject started from the chosen card is still running.
        let working = if index == deck.selected() && deck.is_working() {
            " …"
        } else {
            ""
        };
        let title = fit_width(
            &format!("{} {}{working}", icon(device.kind, ascii), device.title()),
            usize::from(rect.width.saturating_sub(4)),
        );
        let verdict = deck.verdict(index);
        let mut block = style::themed_block(config, &title, here == Some(index));
        let mut base = Style::default();
        if chosen {
            base = base.bg(style::color(&theme.selection_bg));
        }
        // A card the load cannot go on is dimmed, so what can take it stands out at a glance.
        if matches!(
            verdict,
            Some(Verdict::Short { .. } | Verdict::TooBig { .. })
        ) {
            base = base.add_modifier(Modifier::DIM);
        }
        block = block.style(base);
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        if rect.width < CONTENT_MIN || inner.height == 0 {
            continue;
        }
        let lines = card_lines(
            device,
            deck,
            verdict.as_ref(),
            usize::from(inner.width),
            config,
            ascii,
        );
        frame.render_widget(Paragraph::new(lines), inner);
    }
    // A mark at each edge where cards are hidden beyond it.
    let accent = Style::default().fg(style::color(&theme.accent_fg));
    let middle = area.y + area.height / 2;
    if deck.first_card().get() > 0 {
        frame.render_widget(
            Paragraph::new(Span::styled("‹", accent)),
            Rect::new(area.x, middle, 1, 1),
        );
    }
    if placed
        .last()
        .is_some_and(|(i, _)| i + 1 < deck.devices().len())
    {
        frame.render_widget(
            Paragraph::new(Span::styled("›", accent)),
            Rect::new(area.right().saturating_sub(1), middle, 1, 1),
        );
    }
}

/// The three lines inside a card: where it is, how full it is, how busy it is.
fn card_lines(
    device: &Device,
    deck: &DeckState,
    verdict: Option<&Verdict>,
    width: usize,
    config: &Config,
    ascii: bool,
) -> Vec<Line<'static>> {
    let theme = &config.theme;
    let g = glyphs::of(config);
    let dim = Style::default().fg(style::color(&theme.status_fg));
    let accent = Style::default().fg(style::color(&theme.accent_fg));
    let danger = Style::default().fg(style::color(&theme.danger_fg));

    let place = match (&device.mount, device.kind) {
        (Some(mount), _) => elide_left(&mount.display().to_string(), width),
        (None, _) => format!("not mounted · {}", device.fs.as_deref().unwrap_or("?")),
    };
    let first = Line::from(Span::styled(pad_to(&fit_width(&place, width), width), dim));

    let second = match (device.used_percent(), device.is_mounted()) {
        (Some(percent), true) => {
            let free = format!(" {percent:>3}% {} free", format_size(device.free));
            let bar = width.saturating_sub(text_width(&free));
            if bar >= 4 {
                let (on, off) = gauge(percent, bar, g.gauge_on, g.gauge_off);
                let fill = if percent >= 90 { danger } else { accent };
                Line::from(vec![
                    Span::styled(on, fill),
                    Span::styled(off, dim),
                    Span::raw(free),
                ])
            } else {
                Line::from(Span::raw(fit_width(free.trim_start(), width)))
            }
        }
        (Some(_) | None, false) if device.total > 0 => Line::from(Span::styled(
            fit_width(&format!("{} to mount", format_size(device.total)), width),
            dim,
        )),
        _ => Line::from(Span::styled("size unknown", dim)),
    };

    let third = if let Some(verdict) = verdict {
        let style = match verdict {
            Verdict::Fits { .. } | Verdict::SameDisk => accent,
            Verdict::Short { .. } | Verdict::TooBig { .. } => danger,
            Verdict::Tight { .. } | Verdict::Unmounted | Verdict::Unsure => dim,
        };
        Line::from(Span::styled(
            fit_width(&cargo_fit::card_text(verdict, ascii), width),
            style,
        ))
    } else if device.is_mounted() && device.kind != Kind::Network {
        let rates = deck.rates(device);
        let text = rate_text(rates.read, rates.written);
        let room = width.saturating_sub(text_width(&text) + 1);
        let history = deck.history(device);
        if room >= 3 {
            let busy = rates.read > 0 || rates.written > 0;
            Line::from(vec![
                Span::styled(
                    sparkline(&history, room.min(devices::activity::HISTORY), ascii),
                    if busy { accent } else { dim },
                ),
                Span::raw(" "),
                Span::styled(text, if busy { accent } else { dim }),
            ])
        } else {
            Line::from(Span::styled(fit_width(&text, width), dim))
        }
    } else if device.kind == Kind::Network {
        Line::from(Span::styled(fit_width(&device.node, width), dim))
    } else {
        Line::from(Span::styled("Enter to mount", dim))
    };
    vec![first, second, third]
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

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

    fn deck(runtime: &tokio::runtime::Runtime, devices: Vec<Device>) -> DeckState {
        DeckState::with_devices(runtime.handle().clone(), devices)
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
    fn cards_sit_side_by_side_with_a_gap_and_as_many_as_fit() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let deck = deck(&runtime, many(5));
        let area = Rect::new(2, 3, 100, 5);
        let placed = cards(area, &deck, Instant::now());
        assert_eq!(
            placed.len(),
            3,
            "100 columns hold three cards of 28 and gaps"
        );
        assert_eq!(placed[0].1, Rect::new(2, 3, CARD_WIDTH, 5));
        assert_eq!(placed[1].1.x, 2 + CARD_WIDTH + GAP);
        assert!(
            placed
                .iter()
                .all(|(_, r)| area.contains(Position::new(r.x, r.y)))
        );
    }

    #[test]
    fn scrolling_follows_the_selected_card_and_stays_put_otherwise() {
        assert_eq!(follow(0, 0, 10, 3), 0);
        assert_eq!(follow(2, 0, 10, 3), 0, "still on screen");
        assert_eq!(follow(3, 0, 10, 3), 1, "one past the edge scrolls one");
        assert_eq!(follow(9, 0, 10, 3), 7);
        assert_eq!(follow(1, 7, 10, 3), 1, "back to the left");
        assert_eq!(follow(5, 99, 10, 3), 5, "a stale position is pulled in");
        assert_eq!(follow(0, 0, 0, 3), 0);
    }

    #[test]
    fn a_click_finds_the_card_that_was_drawn_there() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let deck = deck(&runtime, many(3));
        let area = Rect::new(0, 0, 100, 5);
        assert_eq!(card_at(area, &deck, Position::new(3, 2)), Some(0));
        assert_eq!(
            card_at(area, &deck, Position::new(CARD_WIDTH + GAP + 1, 2)),
            Some(1)
        );
        assert_eq!(
            card_at(area, &deck, Position::new(CARD_WIDTH, 2)),
            None,
            "the gap"
        );
        assert_eq!(card_at(area, &deck, Position::new(3, 9)), None);
    }

    #[test]
    fn a_card_sliding_in_is_narrow_and_grows_to_full_width() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut deck = deck(&runtime, many(1));
        let now = Instant::now();
        deck.mark_arrived("sd01", now);
        let area = Rect::new(0, 0, 100, 5);
        let at = |t: Duration| cards(area, &deck, now + t)[0].1.width;
        assert_eq!(at(Duration::ZERO), MIN_CARD);
        assert!(at(crate::deck::SLIDE_IN / 2) > MIN_CARD);
        assert_eq!(at(crate::deck::SLIDE_IN), CARD_WIDTH);
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
    fn each_card_shows_its_name_where_it_is_how_full_and_how_busy() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let deck = deck(
            &runtime,
            vec![
                device("sda1", "", Some("/"), Kind::Internal, 250),
                device(
                    "sdb1",
                    "HOLIDAY",
                    Some("/run/media/me/HOLIDAY"),
                    Kind::Removable,
                    900,
                ),
                device("sdc1", "BACKUP", None, Kind::Removable, 0),
            ],
        );
        let shown = drawn(&deck, "/run/media/me/HOLIDAY/photos", 120, 5);
        for text in [
            "System",
            "HOLIDAY",
            "BACKUP",
            "75%",
            "10%",
            "not mounted",
            "to mount",
            "idle",
        ] {
            assert!(shown.contains(text), "{text:?} missing:\n{shown}");
        }
    }

    #[test]
    fn the_card_holding_the_browsed_folder_is_the_one_with_the_bright_frame() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let deck = deck(&runtime, many(2));
        let config = Config::from_sources(None, None, None);
        let here_color = style::color(&config.theme.border_focused_fg);
        let mut terminal = Terminal::new(TestBackend::new(60, 5)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &deck,
                    Path::new("/mnt/1/deep"),
                    &config,
                    Instant::now(),
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let corner = |x: u16| buffer[Position::new(x, 0)].fg;
        assert_ne!(corner(0), here_color, "the first card is not where we are");
        assert_eq!(corner(CARD_WIDTH + GAP), here_color, "the second is");
    }

    #[test]
    fn an_empty_deck_says_it_is_looking_and_then_that_it_found_nothing() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut deck = deck(&runtime, Vec::new());
        deck.set_scanned(false);
        assert!(drawn(&deck, "/", 60, 5).contains("looking for devices"));
        deck.set_scanned(true);
        assert!(drawn(&deck, "/", 60, 5).contains("no devices found"));
    }

    #[test]
    fn while_files_are_carried_each_card_says_whether_they_fit() {
        use crate::cargo_fit::{Carry, Load};
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut deck = deck(
            &runtime,
            vec![
                device("sda1", "", Some("/"), Kind::Internal, 900),
                device(
                    "sdb1",
                    "HOLIDAY",
                    Some("/run/media/me/HOLIDAY"),
                    Kind::Removable,
                    10,
                ),
                device("sdc1", "BACKUP", None, Kind::Removable, 0),
            ],
        );
        for d in 0..2 {
            let device = &mut deck.devices_mut()[d];
            device.total = 1 << 40;
            device.free = [1u64 << 39, 1 << 20][d];
        }
        assert!(
            !drawn(&deck, "/", 120, 5).contains("fits"),
            "nothing carried yet"
        );
        deck.set_load(Some(Load {
            bytes: 1 << 30,
            exact: true,
            largest: 1 << 20,
            carry: Carry::Copy,
            source: "/home/me/photos".into(),
        }));
        let shown = drawn(&deck, "/", 120, 5);
        for text in ["✓ fits", "✗ needs", "mount to check"] {
            assert!(shown.contains(text), "{text:?} missing:\n{shown}");
        }
        assert!(
            !shown.contains("idle"),
            "the verdict takes the activity line:\n{shown}"
        );
    }

    #[test]
    fn it_draws_at_any_size_without_panicking() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let deck = deck(&runtime, many(6));
        for (w, h) in [
            (0, 0),
            (1, 1),
            (3, 5),
            (10, 2),
            (27, 5),
            (28, 5),
            (60, 3),
            (300, 5),
        ] {
            drawn(&deck, "/mnt/2", w, h);
        }
    }

    #[test]
    fn hidden_cards_are_marked_at_the_edges() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let deck = deck(&runtime, many(6));
        let shown = drawn(&deck, "/mnt/0", 100, 5);
        assert!(shown.contains('›') && !shown.contains('‹'), "{shown}");
    }
}
