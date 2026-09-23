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

//! The heads-up display around the file lists: the header (breadcrumb and status pills), the
//! size/age columns, the scrollbar, and the powerline-style status bar. The formatting and
//! layout decisions are pure functions so they can be unit-tested without a terminal; the
//! `render_*` functions only turn their results into widgets.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use shared::DirEntryInfo;
use theming::{Action, Config};

use crate::app::{ClipboardMode, Progress};
use crate::git_status::{FileState, Head, Repo};
use crate::glyphs::{self, Glyphs};
use crate::marked_size::Total;
use crate::style;

// ---------------------------------------------------------------------------------------------
// Text measuring and formatting
// ---------------------------------------------------------------------------------------------

/// Display width in terminal cells (wide characters count double), not `char` count.
pub fn text_width(s: &str) -> usize {
    Span::raw(s).width()
}

/// `s` cut to at most `width` cells, ending in `…` when something was dropped.
pub fn fit_width(s: &str, width: usize) -> String {
    if text_width(s) <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let w = text_width(ch.encode_utf8(&mut [0; 4]));
        // Reserve one cell for the ellipsis.
        if used + w + 1 > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

/// `fit_width`, then right-padded with spaces to exactly `width` cells.
pub fn pad_to(s: &str, width: usize) -> String {
    let mut out = fit_width(s, width);
    out.push_str(&" ".repeat(width.saturating_sub(text_width(&out))));
    out
}

/// A byte count in at most five cells: `512B`, `1.2K`, `14K`, `3.4M`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    if bytes < 1024 {
        return format!("{bytes}B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    // One decimal below 10, none above — and rounding up to 1024 must roll into the next unit
    // rather than print "1024K".
    let rounded = if value < 10.0 {
        (value * 10.0).round() / 10.0
    } else {
        value.round()
    };
    if rounded >= 1024.0 && unit < UNITS.len() - 1 {
        return format!("1.0{}", UNITS[unit + 1]);
    }
    if value < 10.0 {
        format!("{rounded:.1}{}", UNITS[unit])
    } else {
        format!("{rounded:.0}{}", UNITS[unit])
    }
}

/// How long ago `then` was, in at most three cells: `now`, `5m`, `2h`, `3d`, `4w`, `8mo`, `2y`.
/// A timestamp in the future (clock skew) reads as `now`; a missing one as `—`.
pub fn format_age(now: SystemTime, then: Option<SystemTime>) -> String {
    let Some(then) = then else {
        return "—".into();
    };
    let secs = now.duration_since(then).map_or(0, |d| d.as_secs());
    match secs {
        0..=59 => "now".into(),
        60..=3_599 => format!("{}m", secs / 60),
        3_600..=86_399 => format!("{}h", secs / 3_600),
        86_400..=604_799 => format!("{}d", secs / 86_400),
        604_800..=2_591_999 => format!("{}w", secs / 604_800),
        2_592_000..=31_535_999 => format!("{}mo", secs / 2_592_000),
        _ => format!("{}y", secs / 31_536_000),
    }
}

/// The nine `rwxr-xr-x` permission characters for a Unix mode.
pub fn format_perms(mode: u32) -> String {
    const BITS: [(u32, char); 9] = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    BITS.iter()
        .map(|&(bit, ch)| if mode & bit != 0 { ch } else { '-' })
        .collect()
}

/// The path as breadcrumb segments — `~` for the home directory, `/` for the root — shortened
/// from the left with a `…` segment until the joined result fits `max_width` cells.
pub fn breadcrumb(path: &Path, home: Option<&Path>, max_width: usize) -> Vec<String> {
    let names = |p: &Path| -> Vec<String> {
        p.components()
            .filter_map(|c| match c {
                std::path::Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect()
    };
    let mut segments = match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => std::iter::once("~".to_string())
            .chain(names(rest))
            .collect(),
        None => std::iter::once("/".to_string())
            .chain(names(path))
            .collect::<Vec<_>>(),
    };

    let joined_width = |segs: &[String]| {
        segs.iter().map(|s| text_width(s)).sum::<usize>() + 3 * segs.len().saturating_sub(1)
    };
    while joined_width(&segments) > max_width && segments.len() > 2 {
        // Drop the segment right after the first (or after an existing ellipsis) so the root and
        // the deepest directories — the parts that orient you — stay.
        if segments[1] == "…" {
            segments.remove(2);
        } else {
            segments[1] = "…".into();
        }
    }
    if joined_width(&segments) > max_width {
        // Still too wide with only the ends left: shorten the last segment itself.
        let before_last = match segments.len() {
            0 | 1 => 0,
            n => joined_width(&segments[..n - 1]) + 3,
        };
        if let Some(last) = segments.last_mut() {
            *last = fit_width(last, max_width.saturating_sub(before_last).max(1));
        }
    }
    segments
}

// ---------------------------------------------------------------------------------------------
// File-list columns and scrollbar
// ---------------------------------------------------------------------------------------------

/// The two-cell gutter in front of every current-pane name (selection stripe, mark). A file-type
/// icon, when the glyph set has them, adds to it.
pub const BASE_GUTTER: usize = 2;
const SIZE_WIDTH: usize = 5;
const AGE_WIDTH: usize = 3;
/// A name never gets squeezed below this to make room for a column.
const MIN_NAME_WIDTH: usize = 12;

/// Which right-hand columns fit a pane, and how wide the name column then is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Columns {
    pub name_width: usize,
    pub size: bool,
    pub age: bool,
}

/// Picks the richest layout that still leaves a name `MIN_NAME_WIDTH` cells — dropping the age
/// column first, then size — so a narrow terminal loses detail, never the file name. `gutter` is
/// everything in front of the name (stripe, mark, and any icon).
pub fn plan_columns(inner_width: usize, gutter: usize) -> Columns {
    let tail = |size: bool, age: bool| {
        (if size { 1 + SIZE_WIDTH } else { 0 }) + (if age { 1 + AGE_WIDTH } else { 0 })
    };
    for (size, age) in [(true, true), (true, false)] {
        let used = gutter + tail(size, age);
        if inner_width >= used + MIN_NAME_WIDTH {
            return Columns {
                name_width: inner_width - used,
                size,
                age,
            };
        }
    }
    Columns {
        name_width: inner_width.saturating_sub(gutter),
        size: false,
        age: false,
    }
}

/// The size column's text: a file's size, or the glyph set's "none" mark for a directory (whose
/// size is meaningless; its item count shows in the status bar once selected).
pub fn size_cell(entry: &DirEntryInfo, none: &str) -> String {
    if entry.is_dir {
        none.into()
    } else {
        format_size(entry.size)
    }
}

/// A thin scrollbar drawn over the pane's right border, only when the list overflows the pane.
pub fn render_scrollbar(
    frame: &mut Frame<'_>,
    pane: Rect,
    total: usize,
    selected: usize,
    config: &Config,
) {
    let visible = pane.height.saturating_sub(2) as usize;
    if visible == 0 || total <= visible {
        return;
    }
    let theme = &config.theme;
    let g = glyphs::of(config);
    let mut state = ScrollbarState::new(total)
        .position(selected)
        .viewport_content_length(visible);
    let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_symbol(Some(g.track))
        .thumb_symbol(g.thumb)
        .track_style(Style::default().fg(style::color(&theme.border_focused_fg)))
        .thumb_style(Style::default().fg(style::color(&theme.accent_fg)));
    frame.render_stateful_widget(
        bar,
        pane.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut state,
    );
}

// ---------------------------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------------------------

pub struct HeaderView<'a> {
    pub path: &'a Path,
    pub home: Option<PathBuf>,
    pub marks: usize,
    /// What the marks add up to, once known; shown after the count.
    pub marks_total: Option<Total>,
    pub clipboard: Option<(ClipboardMode, usize)>,
    pub progress: Option<Progress>,
}

/// A `cells`-wide gauge with `filled` cells lit (`▰▰▱▱▱` in the unicode set).
pub fn gauge(filled: usize, cells: usize, g: &Glyphs) -> String {
    let filled = filled.min(cells);
    format!(
        "{}{}",
        g.gauge_on.repeat(filled),
        g.gauge_off.repeat(cells - filled)
    )
}

/// How many of `cells` to light. A multi-item paste fills in step with its items; a single
/// item has no known total, so its gauge just cycles as files complete.
pub fn progress_fill(progress: &Progress, cells: usize) -> usize {
    match progress.batch {
        Some((index, total)) if total > 0 => ((index + 1) * cells).div_ceil(total).clamp(1, cells),
        _ => progress.done as usize % cells + 1,
    }
}

pub fn render_header(frame: &mut Frame<'_>, area: Rect, view: &HeaderView<'_>, config: &Config) {
    let theme = &config.theme;
    let g = glyphs::of(config);
    let pill_bg = style::color(&theme.bar_bg);
    let styles = &config.styles;
    let pill = |text: String, fg: &str| {
        Span::styled(
            format!(" {text} "),
            style::styled(
                Style::default().fg(style::color(fg)).bg(pill_bg),
                styles.pill,
            ),
        )
    };

    let mut pills: Vec<Span<'static>> = Vec::new();
    if let Some(progress) = &view.progress {
        let text = format!(
            "{} {} {}",
            gauge(progress_fill(progress, 5), 5, g),
            progress.label,
            progress.done
        );
        pills.push(pill(text, &theme.border_focused_fg));
    }
    if let Some((mode, count)) = view.clipboard {
        let text = match mode {
            ClipboardMode::Copy => g.pill(g.yanked, &format!("{count} yanked")),
            ClipboardMode::Move => g.pill(g.cut, &format!("{count} cut")),
        };
        pills.push(pill(text, &theme.config_fg));
    }
    if view.marks > 0 {
        let mut text = format!("{} marked", view.marks);
        if let Some(total) = view.marks_total {
            text.push_str(&format!(" {} {}", g.divider, total.label()));
        }
        pills.push(pill(g.pill(g.marked, &text), &theme.accent_fg));
    }
    let mut right = Vec::with_capacity(pills.len() * 2);
    for (i, p) in pills.into_iter().enumerate() {
        if i > 0 {
            right.push(Span::raw(" "));
        }
        right.push(p);
    }
    let right_line = Line::from(right);
    let right_width = right_line.width().min(area.width as usize);

    let left_width = (area.width as usize).saturating_sub(right_width + 1);
    let prefix = g.header_prefix;
    let segments = breadcrumb(
        view.path,
        view.home.as_deref(),
        left_width.saturating_sub(text_width(prefix)),
    );
    let dim = style::styled(
        Style::default().fg(style::color(&theme.title_fg)),
        styles.breadcrumb,
    );
    let sep = Style::default().fg(style::color(&theme.border_fg));
    let mut left = vec![Span::styled(
        prefix,
        Style::default().fg(style::color(&theme.accent_fg)),
    )];
    let last = segments.len().saturating_sub(1);
    for (i, segment) in segments.into_iter().enumerate() {
        if i > 0 {
            left.push(Span::styled(g.crumb_sep, sep));
        }
        left.push(if i == last {
            Span::styled(
                segment,
                style::styled(
                    Style::default().fg(style::color(&theme.border_focused_fg)),
                    styles.breadcrumb_current,
                ),
            )
        } else {
            Span::styled(segment, dim)
        });
    }
    frame.render_widget(
        Paragraph::new(Line::from(left)),
        Rect::new(area.x, area.y, left_width as u16, area.height),
    );
    frame.render_widget(
        Paragraph::new(right_line),
        Rect::new(
            area.x + area.width - right_width as u16,
            area.y,
            right_width as u16,
            area.height,
        ),
    );
}

// ---------------------------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------------------------

/// What the app is doing right now, shown as the status bar's colored pill and used to pick
/// the key hints that make sense at this moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Keystrokes are going to a mini-shell.
    Shell,
    /// `space` was just pressed with a shell open; the next key is a pane command.
    Leader,
    Resize,
    Move,
    /// A background copy/move/delete is running.
    Busy,
    Prompt {
        label: &'static str,
        destructive: bool,
        text_input: bool,
    },
}

pub struct StatusView<'a> {
    pub mode: Mode,
    pub shell_open: bool,
    pub entry: Option<&'a DirEntryInfo>,
    /// Item count of the selected directory, when it's one.
    pub dir_items: Option<usize>,
    /// `(1-based position of the selection, entries in the directory)`.
    pub position: (usize, usize),
    pub message: &'a str,
    /// The repository the browsed directory is in, when git answered.
    pub git: Option<&'a Repo>,
    /// The selected entry's state in that repository; `None` for a clean one.
    pub git_entry: Option<FileState>,
}

/// The git segment's text: `⎇ main ↑2 ↓1 +3 ~2 ?1`. Counts that are zero are left out, so a clean
/// branch is just its name.
pub fn git_summary(repo: &Repo, g: &Glyphs) -> String {
    let head = match &repo.head {
        Head::Branch(name) => name.clone(),
        Head::Detached(id) => format!("@{id}"),
    };
    let mut parts = vec![g.pill(g.branch, &head)];
    if let Some((ahead, behind)) = repo.ahead_behind {
        if ahead > 0 {
            parts.push(format!("{}{ahead}", g.ahead));
        }
        if behind > 0 {
            parts.push(format!("{}{behind}", g.behind));
        }
    }
    let counts = repo.counts;
    for (symbol, count) in [
        (g.staged, counts.staged),
        (g.modified, counts.modified),
        (g.untracked, counts.untracked),
        (g.conflicted, counts.conflicted),
    ] {
        if count > 0 {
            parts.push(format!("{symbol}{count}"));
        }
    }
    parts.join(" ")
}

pub fn key_label(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "space".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "enter".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::Backspace => "bksp".into(),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// The user's own shortest binding for `action`, if any is bound.
fn best_label(config: &Config, action: Action) -> Option<String> {
    config
        .keys
        .keys_for(action)
        .into_iter()
        .map(key_label)
        .min_by_key(|l| (l.chars().count(), l.clone()))
}

/// The `(key, what it does)` pairs worth showing in `mode`, most useful first — the tail is
/// what gets dropped when the bar is too narrow for all of them.
pub fn hints(view: &StatusView<'_>, config: &Config) -> Vec<(String, &'static str)> {
    let bound =
        |action: Action, what: &'static str| best_label(config, action).map(|key| (key, what));
    let fixed = |key: &str, what: &'static str| Some((key.to_string(), what));
    let list: Vec<Option<(String, &'static str)>> = match view.mode {
        Mode::Normal if view.shell_open => vec![
            bound(Action::Leader, "leader"),
            bound(Action::Quit, "quit"),
            bound(Action::QuitToCwd, "quit+cd"),
        ],
        Mode::Normal => vec![
            bound(Action::Shell, "shell"),
            bound(Action::Search, "search"),
            bound(Action::Command, "cmd"),
            bound(Action::Select, "mark"),
            bound(Action::ToggleHidden, "hidden"),
            bound(Action::Quit, "quit"),
            bound(Action::QuitToCwd, "quit+cd"),
            bound(Action::Cancel, "cancel"),
        ],
        Mode::Leader => vec![
            fixed("hjkl", "focus"),
            fixed("|", "split"),
            fixed("-", "stack"),
            fixed("x", "close"),
            fixed("r", "resize"),
            fixed("m", "move"),
            fixed("t", "flip"),
            bound(Action::Leader, "type"),
        ],
        Mode::Shell => vec![fixed("alt", "browse")],
        Mode::Resize | Mode::Move => vec![fixed("hjkl", "nudge"), fixed("esc", "done")],
        Mode::Busy => vec![fixed("esc", "cancel"), bound(Action::Cancel, "cancel all")],
        Mode::Prompt {
            text_input: true, ..
        } => vec![fixed("enter", "ok"), fixed("esc", "cancel")],
        Mode::Prompt { .. } => vec![],
    };
    list.into_iter().flatten().collect()
}

fn mode_pill(mode: Mode, theme: &theming::Theme) -> (String, Color) {
    let (label, color) = match mode {
        Mode::Normal => ("NORMAL", &theme.border_focused_fg),
        Mode::Shell => ("SHELL", &theme.accent_fg),
        Mode::Leader => ("LEADER", &theme.accent_fg),
        Mode::Resize => ("RESIZE", &theme.config_fg),
        Mode::Move => ("MOVE", &theme.config_fg),
        Mode::Busy => ("BUSY", &theme.archive_fg),
        Mode::Prompt {
            label, destructive, ..
        } => (
            label,
            if destructive {
                &theme.danger_fg
            } else {
                &theme.doc_fg
            },
        ),
    };
    (label.to_string(), style::color(color))
}

/// One block of the status bar: text on a background.
struct Segment {
    text: String,
    style: Style,
}

/// Cells a segment takes beyond its text: a space each side and one separator.
const SEGMENT_PADDING: usize = 3;

fn segment_cost(seg: &Segment) -> usize {
    text_width(&seg.text) + SEGMENT_PADDING
}

fn segment_bg(seg: &Segment) -> Color {
    seg.style.bg.unwrap_or(Color::Reset)
}

/// Segments laid out left to right. Between them: a Powerline arrow if `arrows`, else a thin
/// divider where two neighbours share a background (so they don't blur into one block).
fn left_segments(segs: &[Segment], arrows: bool, divider: Style, g: &Glyphs) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, seg) in segs.iter().enumerate() {
        spans.push(Span::styled(format!(" {} ", seg.text), seg.style));
        match segs.get(i + 1) {
            Some(next) if arrows && segment_bg(seg) == segment_bg(next) => {
                spans.push(Span::styled(g.arrow_right_thin, divider));
            }
            Some(next) if arrows => spans.push(Span::styled(
                g.arrow_right,
                Style::default().fg(segment_bg(seg)).bg(segment_bg(next)),
            )),
            Some(next) if segment_bg(seg) == segment_bg(next) => {
                spans.push(Span::styled(g.divider, divider));
            }
            None if arrows => spans.push(Span::styled(
                g.arrow_right,
                Style::default().fg(segment_bg(seg)).bg(Color::Reset),
            )),
            _ => {}
        }
    }
    Line::from(spans)
}

/// Segments for the right edge: arrows point left, drawn before each segment.
fn right_segments(segs: &[Segment], arrows: bool, g: &Glyphs) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, seg) in segs.iter().enumerate() {
        if arrows {
            let prev_bg = if i == 0 {
                Color::Reset
            } else {
                segment_bg(&segs[i - 1])
            };
            spans.push(Span::styled(
                g.arrow_left,
                Style::default().fg(segment_bg(seg)).bg(prev_bg),
            ));
        }
        spans.push(Span::styled(format!(" {} ", seg.text), seg.style));
    }
    Line::from(spans)
}

/// Hints as `key label  key label`, keys in the accent color; whole hints are dropped from the
/// end until the rest fit `width`.
fn fit_hints(all: &[(String, &'static str)], width: usize, config: &Config) -> Line<'static> {
    let theme = &config.theme;
    let key_style = style::styled(
        Style::default().fg(style::color(&theme.accent_fg)),
        config.styles.hint_key,
    );
    let label_style = style::styled(
        Style::default().fg(style::color(&theme.status_fg)),
        config.styles.hint_label,
    );
    let build = |n: usize| {
        let mut spans = Vec::new();
        for (i, (key, what)) in all.iter().take(n).enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(key.clone(), key_style));
            spans.push(Span::styled(format!(" {what}"), label_style));
        }
        Line::from(spans)
    };
    (0..=all.len())
        .rev()
        .map(build)
        .find(|line| line.width() <= width)
        .unwrap_or_default()
}

/// Whether segment edges are Powerline arrows: as the theme's `separator` says, or, for `auto`,
/// whenever the glyph set is one with a Nerd Font behind it.
fn use_arrows(config: &Config, g: &Glyphs) -> bool {
    match config.theme.separator.to_lowercase().as_str() {
        "arrow" => true,
        "flat" => false,
        _ => g.powerline,
    }
}

pub fn render_status_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &StatusView<'_>,
    config: &Config,
) {
    let theme = &config.theme;
    let width = area.width as usize;
    let g = glyphs::of(config);
    let arrows = use_arrows(config, g);
    let bar_bg = style::color(&theme.bar_bg);
    let styles = &config.styles;
    let seg_base = Style::default().fg(style::color(&theme.file_fg)).bg(bar_bg);
    let seg_style = style::styled(seg_base, styles.status);
    let divider = Style::default()
        .fg(style::color(&theme.border_fg))
        .bg(bar_bg);

    let (label, mode_color) = mode_pill(view.mode, theme);
    let mut left = vec![Segment {
        text: label,
        style: style::styled(
            Style::default().fg(Color::Black).bg(mode_color),
            styles.mode,
        ),
    }];
    let mut right = Vec::new();

    if view.mode == Mode::Normal {
        if let Some(entry) = view.entry {
            let kind = if entry.is_dir {
                "dir".to_string()
            } else {
                std::path::Path::new(&entry.name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map_or_else(|| "file".to_string(), str::to_lowercase)
            };
            let size = match (entry.is_dir, view.dir_items) {
                (true, Some(1)) => "1 item".to_string(),
                (true, Some(n)) => format!("{n} items"),
                (true, None) => "dir".to_string(),
                (false, _) => format_size(entry.size),
            };
            let name_style = style::styled(seg_base, styles.status_name);
            left.push(Segment {
                text: fit_width(&entry.name, 28),
                style: name_style,
            });
            left.push(Segment {
                text: entry.mode.map_or_else(|| g.none.to_string(), format_perms),
                style: seg_style,
            });
            left.push(Segment {
                text: size,
                style: seg_style,
            });
            left.push(Segment {
                text: kind,
                style: seg_style,
            });
        }
        let position = Segment {
            text: format!("{}/{}", view.position.0, view.position.1),
            style: seg_style,
        };

        // The git segments are the first thing to go on a narrow bar: they are added only if they
        // fit beside everything that is always shown, so they never crowd out the file details.
        let mut used: usize = left
            .iter()
            .chain(std::iter::once(&position))
            .map(segment_cost)
            .sum();
        if let (Some(state), Some(_)) = (view.git_entry, view.git) {
            let color = match state {
                FileState::Conflicted => &theme.danger_fg,
                _ => &theme.accent_fg,
            };
            let text = state.label().to_string();
            if used + text_width(&text) + SEGMENT_PADDING <= width {
                used += text_width(&text) + SEGMENT_PADDING;
                left.push(Segment {
                    text,
                    style: style::styled(
                        Style::default().fg(style::color(color)).bg(bar_bg),
                        styles.status,
                    ),
                });
            }
        }
        if let Some(repo) = view.git {
            let text = git_summary(repo, g);
            if used + text_width(&text) + SEGMENT_PADDING <= width {
                right.push(Segment {
                    text,
                    style: style::styled(
                        Style::default()
                            .fg(style::color(&theme.accent_fg))
                            .bg(bar_bg),
                        styles.status,
                    ),
                });
            }
        }
        right.push(position);
    }

    let left_line = left_segments(&left, arrows, divider, g);
    let right_line = right_segments(&right, arrows, g);
    let left_width = left_line.width().min(width);
    let right_width = right_line.width().min(width.saturating_sub(left_width));

    let free = width.saturating_sub(left_width + right_width);
    let message = fit_width(view.message, free.saturating_sub(2));
    let message_width = text_width(&message);
    let message_used = if message_width > 0 {
        message_width + 2
    } else {
        0
    };
    let hint_space = free.saturating_sub(message_used + 1);
    let hint_line = fit_hints(&hints(view, config), hint_space, config);
    let hint_width = hint_line.width();

    let message_color = match view.mode {
        Mode::Prompt {
            destructive: true, ..
        } => &theme.danger_fg,
        Mode::Prompt { .. } => &theme.file_fg,
        _ => &theme.accent_fg,
    };
    let at = |x: usize, w: usize| Rect::new(area.x + x as u16, area.y, w as u16, area.height);

    frame.render_widget(Paragraph::new(left_line), at(0, left_width));
    if message_width > 0 {
        frame.render_widget(
            Paragraph::new(Span::styled(
                message,
                style::styled(
                    Style::default().fg(style::color(message_color)),
                    styles.message,
                ),
            )),
            at(left_width + 1, message_width),
        );
    }
    if hint_width > 0 {
        frame.render_widget(
            Paragraph::new(hint_line),
            at(width - right_width - hint_width - 1, hint_width),
        );
    }
    frame.render_widget(
        Paragraph::new(right_line),
        at(width - right_width, right_width),
    );
}

// ---------------------------------------------------------------------------------------------
// Transient messages
// ---------------------------------------------------------------------------------------------

/// How long a status message stays in the bar before it clears itself.
pub const STATUS_TTL: Duration = Duration::from_secs(5);

/// Expires `App::status` after `STATUS_TTL`. Statuses are assigned from many places, so rather
/// than timestamp each one this notices when the text changes and times it from then.
pub struct StatusClock {
    seen: Option<String>,
    since: Instant,
}

impl StatusClock {
    pub fn new(now: Instant) -> Self {
        Self {
            seen: None,
            since: now,
        }
    }

    pub fn tick(&mut self, status: &mut Option<String>, now: Instant) {
        if *status != self.seen {
            self.seen = status.clone();
            self.since = now;
        } else if status.is_some() && now.duration_since(self.since) >= STATUS_TTL {
            *status = None;
            self.seen = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// The built-in keys and neon theme, without reading anyone's real config file.
    fn test_config() -> Config {
        Config {
            alt_tap: true,
            browser_mouse: true,
            git_status: true,
            show_hidden: false,
            interactive_commands: Vec::new(),
            open_with: Vec::new(),
            plugins: Vec::new(),
            keys: theming::keymap::RawKeyMap::default().into(),
            theme: theming::Theme::default(),
            theme_is_customized: false,
            local_theme: theming::RawTheme::default(),
            ui: theming::Ui::default(),
            local_ui: theming::RawUi::default(),
            styles: theming::Styles::default(),
            font: theming::Font::default(),
            panels: theming::PanelsConfig::default(),
            local_panels: theming::RawPanels::default(),
            local_custom_themes: Vec::new(),
            local_active_custom_theme: None,
        }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn sizes_stay_within_five_cells_and_never_print_1024() {
        assert_eq!(format_size(0), "0B");
        assert_eq!(format_size(1023), "1023B");
        assert_eq!(format_size(1024), "1.0K");
        assert_eq!(format_size(14 * 1024), "14K");
        assert_eq!(format_size(1_048_575), "1.0M");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0G");
        // Up to just under 1 EiB — beyond any real file, and past the largest unit.
        for exp in 0..60 {
            let n = 1u64 << exp;
            for bytes in [n - 1, n, n + 1] {
                let s = format_size(bytes);
                assert!(text_width(&s) <= 5, "{bytes} -> {s}");
                assert!(!s.starts_with("1024"), "{bytes} -> {s}");
            }
        }
    }

    #[test]
    fn ages_pick_the_largest_whole_unit() {
        let now = SystemTime::UNIX_EPOCH + secs(10_000_000_000);
        let ago = |s: u64| format_age(now, Some(now - secs(s)));
        assert_eq!(ago(5), "now");
        assert_eq!(ago(60), "1m");
        assert_eq!(ago(2 * 3600), "2h");
        assert_eq!(ago(3 * 86_400), "3d");
        assert_eq!(ago(2 * 604_800), "2w");
        assert_eq!(ago(90 * 86_400), "3mo");
        assert_eq!(ago(2 * 31_536_000), "2y");
        assert_eq!(format_age(now, None), "—");
        assert_eq!(format_age(now, Some(now + secs(500))), "now");
    }

    #[test]
    fn perms_render_the_nine_permission_characters() {
        assert_eq!(format_perms(0o644), "rw-r--r--");
        assert_eq!(format_perms(0o755), "rwxr-xr-x");
        assert_eq!(format_perms(0o100600), "rw-------");
        assert_eq!(format_perms(0), "---------");
    }

    #[test]
    fn fit_width_cuts_with_an_ellipsis_and_counts_wide_characters() {
        assert_eq!(fit_width("short", 10), "short");
        assert_eq!(fit_width("abcdefghij", 5), "abcd…");
        assert_eq!(fit_width("abc", 0), "");
        // Each of these takes two cells.
        assert_eq!(text_width("日本"), 4);
        assert!(text_width(&fit_width("日本語日本語", 5)) <= 5);
        assert_eq!(text_width(&pad_to("ab", 6)), 6);
    }

    #[test]
    fn breadcrumb_uses_home_and_root_markers() {
        let home = Path::new("/home/me");
        assert_eq!(
            breadcrumb(Path::new("/home/me/dev/mm"), Some(home), 80),
            ["~", "dev", "mm"]
        );
        assert_eq!(
            breadcrumb(Path::new("/usr/lib"), Some(home), 80),
            ["/", "usr", "lib"]
        );
        assert_eq!(breadcrumb(Path::new("/"), None, 80), ["/"]);
    }

    #[test]
    fn breadcrumb_elides_the_middle_and_keeps_the_ends() {
        let path = Path::new("/home/me/one/two/three/four");
        let segs = breadcrumb(path, Some(Path::new("/home/me")), 16);
        assert_eq!(segs.first().map(String::as_str), Some("~"));
        assert_eq!(segs.last().map(String::as_str), Some("four"));
        assert!(segs.contains(&"…".to_string()));
        let width: usize = segs.iter().map(|s| text_width(s)).sum::<usize>() + 3 * (segs.len() - 1);
        assert!(width <= 16, "{segs:?} is {width} wide");
    }

    #[test]
    fn breadcrumb_never_exceeds_its_width_even_for_one_huge_segment() {
        let segs = breadcrumb(Path::new("/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"), None, 10);
        let width: usize = segs.iter().map(|s| text_width(s)).sum::<usize>() + 3 * (segs.len() - 1);
        assert!(width <= 10, "{segs:?} is {width} wide");
    }

    #[test]
    fn columns_drop_age_then_size_but_keep_the_name() {
        let wide = plan_columns(40, BASE_GUTTER);
        assert!(wide.size && wide.age);
        assert_eq!(wide.name_width, 40 - BASE_GUTTER - 6 - 4);

        let medium = plan_columns(22, BASE_GUTTER);
        assert!(medium.size && !medium.age);

        let narrow = plan_columns(15, BASE_GUTTER);
        assert!(!narrow.size && !narrow.age);
        assert_eq!(narrow.name_width, 15 - BASE_GUTTER);
        assert_eq!(plan_columns(0, BASE_GUTTER).name_width, 0);
    }

    #[test]
    fn an_icon_widens_the_gutter_and_so_narrows_the_name() {
        let plain = plan_columns(40, BASE_GUTTER);
        let with_icon = plan_columns(40, BASE_GUTTER + 2);
        assert_eq!(with_icon.name_width, plain.name_width - 2);
        // The icon can tip a borderline pane into dropping a column.
        assert!(plan_columns(24, BASE_GUTTER).age);
        assert!(!plan_columns(24, BASE_GUTTER + 2).age);
    }

    #[test]
    fn gauge_and_fill_track_the_batch_or_cycle_without_one() {
        let unicode = Glyphs::for_set(theming::GlyphSet::Unicode);
        assert_eq!(gauge(2, 5, unicode), "▰▰▱▱▱");
        assert_eq!(gauge(9, 5, unicode), "▰▰▰▰▰");
        assert_eq!(
            gauge(2, 5, Glyphs::for_set(theming::GlyphSet::Ascii)),
            "##---"
        );
        let batch = |index, total| Progress {
            label: "copying",
            done: 0,
            batch: Some((index, total)),
        };
        assert_eq!(progress_fill(&batch(0, 4), 5), 2);
        assert_eq!(progress_fill(&batch(3, 4), 5), 5);
        let single = |done| Progress {
            label: "copying",
            done,
            batch: None,
        };
        assert_eq!(progress_fill(&single(0), 5), 1);
        assert_eq!(progress_fill(&single(7), 5), 3);
    }

    #[test]
    fn status_messages_expire_after_the_ttl_but_not_before() {
        let t0 = Instant::now();
        let mut clock = StatusClock::new(t0);
        let mut status = Some("pane closed".to_string());
        clock.tick(&mut status, t0);
        clock.tick(&mut status, t0 + STATUS_TTL - Duration::from_millis(1));
        assert!(status.is_some());
        clock.tick(&mut status, t0 + STATUS_TTL);
        assert_eq!(status, None);
    }

    #[test]
    fn a_new_status_restarts_the_clock() {
        let t0 = Instant::now();
        let mut clock = StatusClock::new(t0);
        let mut status = Some("one".to_string());
        clock.tick(&mut status, t0);
        status = Some("two".to_string());
        clock.tick(&mut status, t0 + secs(4));
        clock.tick(&mut status, t0 + secs(8));
        assert_eq!(status.as_deref(), Some("two"));
        clock.tick(&mut status, t0 + secs(9));
        assert_eq!(status, None);
    }

    fn entry(name: &str, is_dir: bool) -> DirEntryInfo {
        DirEntryInfo {
            name: name.into(),
            path: name.into(),
            is_dir,
            size: 14_336,
            modified: None,
            mode: Some(0o644),
        }
    }

    fn render_to_text(width: u16, draw: impl Fn(&mut Frame<'_>, Rect)) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).expect("test backend");
        terminal
            .draw(|frame| draw(frame, frame.area()))
            .expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    fn view<'a>(mode: Mode, entry: Option<&'a DirEntryInfo>, message: &'a str) -> StatusView<'a> {
        StatusView {
            mode,
            shell_open: false,
            entry,
            dir_items: Some(3),
            position: (2, 9),
            message,
            git: None,
            git_entry: None,
        }
    }

    #[test]
    fn the_status_bar_shows_the_mode_file_details_and_position() {
        let config = test_config();
        let file = entry("main.rs", false);
        let text = render_to_text(100, |f, a| {
            render_status_bar(f, a, &view(Mode::Normal, Some(&file), ""), &config)
        });
        for expected in ["NORMAL", "main.rs", "rw-r--r--", "14K", "rs", "2/9", "quit"] {
            assert!(text.contains(expected), "missing {expected:?} in {text:?}");
        }
    }

    #[test]
    fn a_selected_directory_shows_its_item_count() {
        let config = test_config();
        let dir = entry("src", true);
        let text = render_to_text(100, |f, a| {
            render_status_bar(f, a, &view(Mode::Normal, Some(&dir), ""), &config)
        });
        assert!(text.contains("3 items"), "{text:?}");
    }

    #[test]
    fn hints_use_the_users_own_bindings() {
        let config = test_config();
        let file = entry("a", false);
        let v = view(Mode::Normal, Some(&file), "");
        let hint_text: Vec<String> = hints(&v, &config).into_iter().map(|(k, _)| k).collect();
        assert!(hint_text.contains(&"q".to_string()));
        assert!(hint_text.contains(&"Q".to_string()));
    }

    #[test]
    fn the_leader_mode_lists_the_pane_commands() {
        let config = test_config();
        let text = render_to_text(120, |f, a| {
            render_status_bar(f, a, &view(Mode::Leader, None, ""), &config)
        });
        for expected in [
            "LEADER", "focus", "split", "close", "resize", "flip", "space",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in {text:?}");
        }
    }

    #[test]
    fn a_prompt_replaces_the_file_details_with_its_text() {
        let config = test_config();
        let file = entry("main.rs", false);
        let mode = Mode::Prompt {
            label: "RENAME",
            destructive: false,
            text_input: true,
        };
        let text = render_to_text(100, |f, a| {
            render_status_bar(f, a, &view(mode, Some(&file), "rename: new▏"), &config)
        });
        assert!(
            text.contains("RENAME") && text.contains("rename: new▏"),
            "{text:?}"
        );
        assert!(!text.contains("rw-r--r--"), "{text:?}");
    }

    #[test]
    fn arrow_separators_are_only_drawn_when_asked_for() {
        let mut config = test_config();
        let file = entry("a", false);
        let flat = render_to_text(80, |f, a| {
            render_status_bar(f, a, &view(Mode::Normal, Some(&file), ""), &config)
        });
        let g = Glyphs::for_set(theming::GlyphSet::Unicode);
        assert!(!flat.contains(g.arrow_right) && !flat.contains(g.arrow_left));

        config.theme.separator = "arrow".into();
        let arrows = render_to_text(80, |f, a| {
            render_status_bar(f, a, &view(Mode::Normal, Some(&file), ""), &config)
        });
        assert!(arrows.contains(g.arrow_right) && arrows.contains(g.arrow_left));
        // Name, permissions, size and type share a background, so they get the hollow arrow.
        assert!(arrows.contains(g.arrow_right_thin));
    }

    fn config_with(glyphs: theming::GlyphSet) -> Config {
        Config {
            ui: theming::Ui { glyphs },
            ..test_config()
        }
    }

    #[test]
    fn auto_separator_draws_arrows_only_for_the_nerd_glyph_set() {
        let file = entry("a", false);
        let bar = |set| {
            let config = config_with(set);
            render_to_text(80, |f, a| {
                render_status_bar(f, a, &view(Mode::Normal, Some(&file), ""), &config)
            })
        };
        let g = Glyphs::for_set(theming::GlyphSet::Nerd);
        assert!(bar(theming::GlyphSet::Nerd).contains(g.arrow_right));
        assert!(!bar(theming::GlyphSet::Unicode).contains(g.arrow_right));
        assert!(!bar(theming::GlyphSet::Ascii).contains(g.arrow_right));
    }

    #[test]
    fn an_explicit_separator_overrides_the_glyph_set() {
        let file = entry("a", false);
        let mut config = config_with(theming::GlyphSet::Nerd);
        config.theme.separator = "flat".into();
        let text = render_to_text(80, |f, a| {
            render_status_bar(f, a, &view(Mode::Normal, Some(&file), ""), &config)
        });
        assert!(!text.contains(Glyphs::for_set(theming::GlyphSet::Nerd).arrow_right));
    }

    #[test]
    fn the_ascii_set_draws_the_header_and_status_bar_in_ascii_only() {
        let config = config_with(theming::GlyphSet::Ascii);
        let file = entry("main.rs", false);
        let repo = sample_repo();
        let mut with_git = view(Mode::Normal, Some(&file), "msg");
        with_git.git = Some(&repo);
        with_git.git_entry = Some(FileState::Modified);
        let status = render_to_text(140, |f, a| render_status_bar(f, a, &with_git, &config));
        assert!(
            status.contains("modified"),
            "the git segments were not drawn: {status:?}"
        );
        let header = render_to_text(120, |f, a| {
            render_header(
                f,
                a,
                &HeaderView {
                    path: Path::new("/home/me/dev"),
                    home: Some("/home/me".into()),
                    marks: 2,
                    marks_total: Some(Total {
                        bytes: 3 * 1024 * 1024,
                        exact: true,
                    }),
                    clipboard: Some((ClipboardMode::Copy, 1)),
                    progress: Some(Progress {
                        label: "copying",
                        done: 1,
                        batch: None,
                    }),
                },
                &config,
            )
        });
        for text in [status, header] {
            assert!(text.is_ascii(), "non-ASCII in {text:?}");
        }
    }

    #[test]
    fn nerd_pills_carry_their_symbols() {
        let config = config_with(theming::GlyphSet::Nerd);
        let text = render_to_text(100, |f, a| {
            render_header(
                f,
                a,
                &HeaderView {
                    path: Path::new("/x"),
                    home: None,
                    marks: 1,
                    marks_total: None,
                    clipboard: Some((ClipboardMode::Move, 2)),
                    progress: None,
                },
                &config,
            )
        });
        let g = Glyphs::for_set(theming::GlyphSet::Nerd);
        assert!(text.contains(g.cut) && text.contains(g.marked), "{text:?}");
    }

    #[test]
    fn the_header_and_status_bar_never_panic_at_any_width() {
        let mut config = test_config();
        let file = entry("a-rather-long-file-name.txt", false);
        let path = Path::new("/home/me/some/deeply/nested/directory/tree");
        for separator in ["flat", "arrow"] {
            config.theme.separator = separator.into();
            for width in 1..=140u16 {
                render_to_text(width, |f, a| {
                    render_header(
                        f,
                        a,
                        &HeaderView {
                            path,
                            home: Some("/home/me".into()),
                            marks: 12,
                            marks_total: None,
                            clipboard: Some((ClipboardMode::Move, 3)),
                            progress: Some(Progress {
                                label: "copying",
                                done: 4,
                                batch: Some((1, 3)),
                            }),
                        },
                        &config,
                    )
                });
                let repo = sample_repo();
                let mut with_git = view(Mode::Normal, Some(&file), "a message that is long");
                with_git.git = Some(&repo);
                with_git.git_entry = Some(FileState::StagedModified);
                render_to_text(width, |f, a| render_status_bar(f, a, &with_git, &config));
                for mode in [
                    Mode::Normal,
                    Mode::Shell,
                    Mode::Leader,
                    Mode::Busy,
                    Mode::Prompt {
                        label: "CONFLICT",
                        destructive: true,
                        text_input: false,
                    },
                ] {
                    render_to_text(width, |f, a| {
                        render_status_bar(
                            f,
                            a,
                            &view(mode, Some(&file), "a message that is fairly long indeed"),
                            &config,
                        )
                    });
                }
            }
        }
    }
    fn sample_repo() -> Repo {
        use crate::git_status::{Counts, Parsed};
        Repo::new(
            PathBuf::from("/repo"),
            Parsed {
                head: Head::Branch("main".into()),
                ahead_behind: Some((2, 1)),
                counts: Counts {
                    staged: 3,
                    modified: 2,
                    untracked: 1,
                    conflicted: 0,
                },
                entries: Vec::new(),
            },
        )
    }

    #[test]
    fn the_git_summary_lists_the_branch_then_only_the_nonzero_counts() {
        let g = Glyphs::for_set(theming::GlyphSet::Unicode);
        assert_eq!(git_summary(&sample_repo(), g), "⎇ main ↑2 ↓1 +3 ~2 ?1");

        let mut clean = sample_repo();
        clean.ahead_behind = Some((0, 0));
        clean.counts = Default::default();
        assert_eq!(git_summary(&clean, g), "⎇ main");

        let mut detached = sample_repo();
        detached.head = Head::Detached("0123456".into());
        detached.ahead_behind = None;
        detached.counts = Default::default();
        assert_eq!(git_summary(&detached, g), "⎇ @0123456");
    }

    #[test]
    fn the_ascii_git_summary_has_no_branch_symbol_and_plain_arrows() {
        let g = Glyphs::for_set(theming::GlyphSet::Ascii);
        assert_eq!(git_summary(&sample_repo(), g), "main ^2 v1 +3 ~2 ?1");
    }

    #[test]
    fn the_status_bar_shows_the_branch_and_the_selected_files_state() {
        let config = test_config();
        let file = entry("main.rs", false);
        let repo = sample_repo();
        let mut v = view(Mode::Normal, Some(&file), "");
        v.git = Some(&repo);
        v.git_entry = Some(FileState::Modified);
        let text = render_to_text(140, |f, a| render_status_bar(f, a, &v, &config));
        assert!(text.contains("modified"), "{text:?}");
        assert!(text.contains("⎇ main ↑2 ↓1 +3 ~2 ?1"), "{text:?}");
        assert!(text.contains("2/9"), "the position must stay: {text:?}");
    }

    #[test]
    fn a_clean_selection_shows_the_branch_but_no_state() {
        let config = test_config();
        let file = entry("main.rs", false);
        let repo = sample_repo();
        let mut v = view(Mode::Normal, Some(&file), "");
        v.git = Some(&repo);
        let text = render_to_text(140, |f, a| render_status_bar(f, a, &v, &config));
        assert!(text.contains("⎇ main"), "{text:?}");
        assert!(
            !text.contains("modified") && !text.contains("untracked"),
            "{text:?}"
        );
    }

    #[test]
    fn a_narrow_bar_drops_the_git_segments_before_the_file_details() {
        let config = test_config();
        let file = entry("main.rs", false);
        let repo = sample_repo();
        let mut v = view(Mode::Normal, Some(&file), "");
        v.git = Some(&repo);
        v.git_entry = Some(FileState::Modified);

        let text = render_to_text(60, |f, a| render_status_bar(f, a, &v, &config));
        assert!(
            !text.contains("⎇"),
            "the branch does not fit beside the details: {text:?}"
        );
        for kept in ["main.rs", "rw-r--r--", "14K", "rs", "2/9"] {
            assert!(text.contains(kept), "{kept:?} was crowded out: {text:?}");
        }
        let roomy = render_to_text(140, |f, a| render_status_bar(f, a, &v, &config));
        assert!(roomy.contains("⎇ main"), "{roomy:?}");
    }

    #[test]
    fn git_is_not_shown_in_modes_that_have_their_own_status() {
        let config = test_config();
        let file = entry("main.rs", false);
        let repo = sample_repo();
        let mut v = view(Mode::Leader, Some(&file), "");
        v.git = Some(&repo);
        let text = render_to_text(140, |f, a| render_status_bar(f, a, &v, &config));
        assert!(!text.contains("⎇"), "{text:?}");
    }

    #[test]
    fn the_marked_pill_gains_the_total_once_it_is_known() {
        let config = test_config();
        let render = |total: Option<Total>| {
            render_to_text(100, |f, a| {
                render_header(
                    f,
                    a,
                    &HeaderView {
                        path: Path::new("/x"),
                        home: None,
                        marks: 3,
                        marks_total: total,
                        clipboard: None,
                        progress: None,
                    },
                    &config,
                )
            })
        };
        assert!(render(None).contains("3 marked"));
        assert!(!render(None).contains('│'));
        let sized = render(Some(Total {
            bytes: 1536,
            exact: true,
        }));
        assert!(
            sized.contains(&format!("3 marked │ {}", format_size(1536))),
            "{sized:?}"
        );
        let bound = render(Some(Total {
            bytes: 1536,
            exact: false,
        }));
        assert!(bound.contains("at least"), "{bound:?}");
    }
}
