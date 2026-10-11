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

//! The trash browser (`T`): everything in the desktop trash, newest first, over the whole screen.
//! Entries are chosen the one way everything else is — marked with `v` — and `r` puts the marked
//! ones back where they were, `x` deletes them for good, and `E` empties the whole trash; both
//! destructive steps ask first. Nothing is ever done to the entry under the cursor unless it is
//! marked.
//!
//! Reading the trash, restoring and deleting all run on the blocking pool (a big trash takes
//! seconds), so the view shows "loading…" or "working…" and the interface never waits. The trash
//! itself is reached through [`TrashBackend`], so the logic here is tested without touching the
//! real one. Everything that decides what a row says and where things sit is a plain function.

use std::cell::Cell;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Margin, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use shared::DirEntryInfo;
use theming::{Action, Config};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use trash::{EntrySize, TrashEntry};

use crate::glyphs;
use crate::hud::{self, fit_width, format_size, modified_cell, pad_to, text_width};
use crate::style;

/// Where the view reads the trash and acts on it. The real one is [`SystemTrash`]; the tests use a
/// fake. Errors are plain text, ready for the status line.
pub trait TrashBackend: Send + Sync {
    fn entries(&self) -> Result<Vec<TrashEntry>, String>;
    /// One result per entry, in order.
    fn restore(&self, entries: &[TrashEntry]) -> Vec<Result<(), String>>;
    /// One result per entry, in order.
    fn purge(&self, entries: &[TrashEntry]) -> Vec<Result<(), String>>;
}

/// The desktop trash.
pub struct SystemTrash;

impl TrashBackend for SystemTrash {
    fn entries(&self) -> Result<Vec<TrashEntry>, String> {
        trash::entries().map_err(|e| e.to_string())
    }

    fn restore(&self, entries: &[TrashEntry]) -> Vec<Result<(), String>> {
        trash::restore_entries(entries)
            .into_iter()
            .map(|r| r.map_err(|e| e.to_string()))
            .collect()
    }

    fn purge(&self, entries: &[TrashEntry]) -> Vec<Result<(), String>> {
        trash::purge_entries(entries)
            .into_iter()
            .map(|r| r.map_err(|e| e.to_string()))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    Loading,
    Ready,
    Failed(String),
}

/// A destructive step waiting for `y`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// Delete the marked entries for good.
    Purge,
    /// Delete everything.
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Work {
    Restore,
    Purge,
    Empty,
}

type Loaded = Result<Vec<TrashEntry>, String>;
type Finished = (Work, Vec<Result<(), String>>);

pub struct TrashView {
    backend: Arc<dyn TrashBackend>,
    handle: tokio::runtime::Handle,
    entries: Vec<TrashEntry>,
    phase: Phase,
    selected: usize,
    /// Marked by the trash's own id, so a mark survives the list being read again.
    marked: HashSet<String>,
    top: Cell<usize>,
    viewport: Cell<usize>,
    ask: Option<Ask>,
    loading: Option<UnboundedReceiver<Loaded>>,
    working: Option<UnboundedReceiver<Finished>>,
    /// The last thing that happened, shown until the next key.
    note: Option<String>,
}

impl TrashView {
    /// Opens the view and starts reading the trash.
    pub fn open(handle: tokio::runtime::Handle, backend: Arc<dyn TrashBackend>) -> Self {
        let mut view = Self {
            backend,
            handle,
            entries: Vec::new(),
            phase: Phase::Loading,
            selected: 0,
            marked: HashSet::new(),
            top: Cell::new(0),
            viewport: Cell::new(0),
            ask: None,
            loading: None,
            working: None,
            note: None,
        };
        view.reload();
        view
    }

    /// Reads the trash again; what was marked stays marked if it is still there.
    fn reload(&mut self) {
        let (tx, rx) = unbounded_channel();
        let backend = Arc::clone(&self.backend);
        self.handle.spawn_blocking(move || {
            let _ = tx.send(backend.entries());
        });
        self.loading = Some(rx);
        if self.entries.is_empty() {
            self.phase = Phase::Loading;
        }
    }

    /// Takes in a finished read or a finished restore or delete. Call once per render tick.
    pub fn poll(&mut self) {
        if let Some(rx) = self.loading.as_mut() {
            match rx.try_recv() {
                Ok(Ok(entries)) => {
                    let ids: HashSet<&str> = entries.iter().map(|e| e.id.as_str()).collect();
                    self.marked.retain(|id| ids.contains(id.as_str()));
                    self.entries = entries;
                    self.selected = self.selected.min(self.entries.len().saturating_sub(1));
                    self.phase = Phase::Ready;
                    self.loading = None;
                }
                Ok(Err(message)) => {
                    self.phase = Phase::Failed(message);
                    self.loading = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.phase = Phase::Failed("reading the trash stopped".into());
                    self.loading = None;
                }
            }
        }
        if let Some(rx) = self.working.as_mut() {
            match rx.try_recv() {
                Ok((work, results)) => {
                    self.working = None;
                    self.note = Some(summary(work, &results));
                    self.reload();
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.working = None;
                    self.note = Some("the operation stopped unexpectedly".into());
                    self.reload();
                }
            }
        }
    }

    pub fn entries(&self) -> &[TrashEntry] {
        &self.entries
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The name of the entry under the cursor, for a pane that has nothing else to say.
    pub fn selected_name(&self) -> Option<&str> {
        self.entries.get(self.selected).map(|e| e.name.as_str())
    }

    pub fn marked_count(&self) -> usize {
        self.marked.len()
    }

    pub fn is_marked(&self, entry: &TrashEntry) -> bool {
        self.marked.contains(&entry.id)
    }

    pub fn top(&self) -> &Cell<usize> {
        &self.top
    }

    pub fn viewport(&self) -> &Cell<usize> {
        &self.viewport
    }

    #[cfg(test)]
    pub fn ask(&self) -> Option<Ask> {
        self.ask
    }

    pub fn is_working(&self) -> bool {
        self.working.is_some()
    }

    pub fn is_loading(&self) -> bool {
        self.phase == Phase::Loading
    }

    pub fn failure(&self) -> Option<&str> {
        match &self.phase {
            Phase::Failed(message) => Some(message),
            _ => None,
        }
    }

    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The marked entries, in the order they are listed.
    pub fn marked_entries(&self) -> Vec<TrashEntry> {
        self.entries
            .iter()
            .filter(|entry| self.marked.contains(&entry.id))
            .cloned()
            .collect()
    }

    /// The entry under the cursor, described the way the browser's own preview pipelines want a
    /// file: the place its contents sit in the trash, so text, images, archives and binaries are
    /// shown by the code that already shows them. `None` when nothing is selected or the trash
    /// does not say where the contents are.
    pub fn preview_subject(&self) -> Option<DirEntryInfo> {
        let entry = self.entries.get(self.selected)?;
        let path = entry.stored_path.clone()?;
        Some(DirEntryInfo {
            name: entry.name.clone(),
            path,
            is_dir: matches!(entry.size, EntrySize::Entries(_)),
            size: match entry.size {
                EntrySize::Bytes(bytes) => bytes,
                _ => 0,
            },
            modified: None,
            mode: None,
        })
    }

    pub fn move_by(&mut self, by: isize) {
        let last = self.entries.len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(by).min(last);
    }

    pub fn move_to(&mut self, row: usize) {
        self.selected = row.min(self.entries.len().saturating_sub(1));
    }

    pub fn page(&mut self, direction: isize) {
        let page = self.viewport.get().saturating_sub(1).max(1) as isize;
        self.move_by(direction * page);
    }

    /// Marks the entry under the cursor, or unmarks it: `v`, the only way to choose entries.
    pub fn toggle_mark(&mut self) {
        let Some(entry) = self.entries.get(self.selected) else {
            return;
        };
        if !self.marked.remove(&entry.id) {
            self.marked.insert(entry.id.clone());
        }
    }

    pub fn clear_marks(&mut self) {
        self.marked.clear();
    }

    /// Handles one key. `action` is what the key means in the browser's own bindings (so `j`/`k`
    /// and any rebinding of them work here); returns whether the view should close.
    pub fn key(&mut self, code: KeyCode, action: Option<Action>) -> bool {
        // A restore or delete is running: nothing to do but wait for it.
        if self.working.is_some() {
            return false;
        }
        self.note = None;
        if let Some(ask) = self.ask.take() {
            if code == KeyCode::Char('y') {
                self.confirm(ask);
            } else {
                self.note = Some("cancelled".into());
            }
            return false;
        }
        match code {
            KeyCode::Esc => return true,
            KeyCode::Char('v') => self.toggle_mark(),
            KeyCode::Char('r') => self.begin_restore(),
            KeyCode::Char('x') => self.begin_purge(),
            KeyCode::Char('E') => self.begin_empty(),
            KeyCode::PageDown => self.page(1),
            KeyCode::PageUp => self.page(-1),
            KeyCode::Home => self.move_to(0),
            KeyCode::End => self.move_to(usize::MAX),
            _ => match action {
                Some(Action::MoveDown) => self.move_by(1),
                Some(Action::MoveUp) => self.move_by(-1),
                Some(Action::Cancel) => self.clear_marks(),
                // Quitting from inside the view closes the view, not Minuteman.
                Some(Action::Quit | Action::QuitToCwd | Action::TrashView) => return true,
                _ => {}
            },
        }
        false
    }

    fn begin_restore(&mut self) {
        let chosen = self.marked_entries();
        if chosen.is_empty() {
            self.note = Some(NOTHING_MARKED.into());
            return;
        }
        self.start(Work::Restore, chosen);
    }

    fn begin_purge(&mut self) {
        match self.marked.is_empty() {
            true => self.note = Some(NOTHING_MARKED.into()),
            false => self.ask = Some(Ask::Purge),
        }
    }

    fn begin_empty(&mut self) {
        match self.entries.is_empty() {
            true => self.note = Some("the trash is already empty".into()),
            false => self.ask = Some(Ask::Empty),
        }
    }

    fn confirm(&mut self, ask: Ask) {
        match ask {
            Ask::Purge => {
                let chosen = self.marked_entries();
                self.start(Work::Purge, chosen);
            }
            Ask::Empty => {
                let all = self.entries.clone();
                self.start(Work::Empty, all);
            }
        }
    }

    /// Runs `work` on `chosen` on the blocking pool; the result arrives through `poll`.
    fn start(&mut self, work: Work, chosen: Vec<TrashEntry>) {
        let (tx, rx) = unbounded_channel();
        let backend = Arc::clone(&self.backend);
        self.handle.spawn_blocking(move || {
            let results = match work {
                Work::Restore => backend.restore(&chosen),
                Work::Purge | Work::Empty => backend.purge(&chosen),
            };
            let _ = tx.send((work, results));
        });
        self.working = Some(rx);
    }

    /// What the confirmation line asks.
    pub fn question(&self) -> Option<String> {
        self.ask.map(|ask| match ask {
            Ask::Purge => format!(
                "delete {} for good? (y/N)",
                plural(self.marked.len(), "marked item")
            ),
            Ask::Empty => format!(
                "empty the whole trash, {}? (y/N)",
                plural(self.entries.len(), "item")
            ),
        })
    }
}

const NOTHING_MARKED: &str = "nothing marked — mark with v first";

fn plural(n: usize, noun: &str) -> String {
    match n {
        1 => format!("1 {noun}"),
        n => format!("{n} {noun}s"),
    }
}

/// What a finished restore or delete says: how many went through and, if any did not, the first
/// reason.
fn summary(work: Work, results: &[Result<(), String>]) -> String {
    let done = results.iter().filter(|r| r.is_ok()).count();
    let failed = results.len() - done;
    let verb = match work {
        Work::Restore => "restored",
        Work::Purge | Work::Empty => "deleted for good:",
    };
    let first_error = results.iter().find_map(|r| r.as_ref().err());
    match (failed, first_error) {
        (0, _) => format!("{verb} {}", plural(done, "item")),
        (_, Some(error)) => format!("{verb} {done} of {} — {error}", results.len()),
        (_, None) => format!("{verb} {done} of {}", results.len()),
    }
}

// ---- drawing ------------------------------------------------------------------------------

/// The regions of the view within the screen: the list's frame with its header, rows and footer,
/// and — on a screen wide enough — a preview pane beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// The frame around the list.
    pub frame: Rect,
    pub header: Rect,
    pub rows: Rect,
    pub footer: Rect,
    /// Where the selected entry's contents are shown; `None` on a narrow screen, which gives the
    /// whole width to the list.
    pub preview: Option<Rect>,
}

/// The least width at which a preview pane is worth its room beside the list.
pub const PREVIEW_MIN_WIDTH: u16 = 100;

pub fn layout(screen: Rect) -> Layout {
    layout_with(screen, true)
}

/// `layout`, with `with_preview` false giving the list the whole width however wide the screen.
pub fn layout_with(screen: Rect, with_preview: bool) -> Layout {
    let (frame, preview) = if with_preview && screen.width >= PREVIEW_MIN_WIDTH {
        let list_width = screen.width * 55 / 100;
        (
            Rect {
                width: list_width,
                ..screen
            },
            Some(Rect {
                x: screen.x + list_width,
                width: screen.width - list_width,
                ..screen
            }),
        )
    } else {
        (screen, None)
    };
    let inner = frame.inner(Margin::new(2, 1));
    let header = Rect {
        height: inner.height.min(1),
        ..inner
    };
    let footer = Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: inner.height.min(1),
        ..inner
    };
    let rows = Rect {
        y: inner.y + inner.height.min(1),
        height: inner.height.saturating_sub(2),
        ..inner
    };
    Layout {
        frame,
        header,
        rows,
        footer,
        preview,
    }
}

/// The row under `pointer`, given which row is at the top and how many there are.
pub fn row_at(layout: &Layout, top: usize, len: usize, pointer: Position) -> Option<usize> {
    if !layout.rows.contains(pointer) {
        return None;
    }
    let index = top + usize::from(pointer.y - layout.rows.y);
    (index < len).then_some(index)
}

/// The first row to draw so `selected` is on screen with as little scrolling as possible.
pub fn scroll_top(selected: usize, viewport: usize, top: usize, len: usize) -> usize {
    if viewport == 0 || len == 0 {
        return 0;
    }
    let mut top = top.min(len.saturating_sub(viewport));
    if selected < top {
        top = selected;
    } else if selected >= top + viewport {
        top = selected + 1 - viewport;
    }
    top
}

const SIZE_WIDTH: usize = 9;
const DATE_WIDTH: usize = hud::MODIFIED_WIDTH;
const MIN_NAME: usize = 12;
const MIN_FOLDER: usize = 10;
/// Stripe, mark and a space.
const PREFIX: usize = 4;

/// Which columns fit `width` cells, dropping the folder first, then the date, then the size, so a
/// narrow terminal loses detail and never the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Columns {
    pub name: usize,
    pub size: bool,
    pub date: bool,
    pub folder: usize,
}

pub fn plan(width: usize) -> Columns {
    let fixed = |size: bool, date: bool| {
        PREFIX + if size { 1 + SIZE_WIDTH } else { 0 } + if date { 1 + DATE_WIDTH } else { 0 }
    };
    for (size, date) in [(true, true), (true, false), (false, false)] {
        let used = fixed(size, date);
        if width >= used + MIN_NAME {
            let left = width - used;
            // The folder gets what is left over beyond a name that is comfortable on its own.
            let folder = if size && date && left >= MIN_NAME * 2 + MIN_FOLDER {
                (left / 2).max(MIN_FOLDER)
            } else {
                0
            };
            let folder_cells = if folder > 0 { folder + 1 } else { 0 };
            return Columns {
                name: left - folder_cells,
                size,
                date,
                folder,
            };
        }
    }
    Columns {
        name: width.saturating_sub(PREFIX),
        size: false,
        date: false,
        folder: 0,
    }
}

/// `3 items`, `12K`, or a dash when the trash could not say.
pub fn size_cell(size: EntrySize) -> String {
    match size {
        EntrySize::Bytes(bytes) => format_size(bytes),
        EntrySize::Entries(1) => "1 item".into(),
        EntrySize::Entries(n) => format!("{n} items"),
        EntrySize::Unknown => "—".into(),
    }
}

/// When it was trashed, in this machine's time zone to the minute, or a dash.
pub fn deleted_cell(secs: i64) -> String {
    match u64::try_from(secs) {
        Ok(secs) => modified_cell(Some(UNIX_EPOCH + Duration::from_secs(secs)), "—"),
        Err(_) => "—".into(),
    }
}

/// `path` cut from the left to fit `width`, keeping the end, which is the part that tells folders
/// apart.
pub(crate) fn elide_left(path: &str, width: usize) -> String {
    if text_width(path) <= width {
        return path.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut kept: Vec<char> = Vec::new();
    let mut used = 1;
    for c in path.chars().rev() {
        let w = text_width(&c.to_string());
        if used + w > width {
            break;
        }
        kept.push(c);
        used += w;
    }
    kept.reverse();
    format!("…{}", kept.into_iter().collect::<String>())
}

pub fn render(frame: &mut Frame<'_>, view: &TrashView, config: &Config) {
    let screen = frame.area();
    let places = layout(screen);
    let theme = &config.theme;
    frame.render_widget(Clear, screen);
    frame.render_widget(style::themed_block(config, "trash", true), places.frame);

    let dim = Style::default().fg(style::color(&theme.status_fg));
    let accent = Style::default().fg(style::color(&theme.accent_fg));
    let danger = Style::default().fg(style::color(&theme.danger_fg));
    let width = usize::from(places.header.width);

    let header = if view.is_working() {
        "working…".to_string()
    } else if let Some(failure) = view.failure() {
        format!("cannot read the trash: {failure}")
    } else if view.is_loading() {
        "loading…".to_string()
    } else {
        let count = plural(view.entries().len(), "item");
        match view.marked_count() {
            0 => format!("{count}, newest first"),
            marked => format!("{count}, {marked} marked"),
        }
    };
    frame.render_widget(
        Paragraph::new(Span::styled(fit_width(&header, width), accent)),
        places.header,
    );

    let viewport = usize::from(places.rows.height);
    let top = scroll_top(
        view.selected(),
        viewport,
        view.top().get(),
        view.entries().len(),
    );
    view.top().set(top);
    view.viewport().set(viewport);

    let columns = plan(usize::from(places.rows.width));
    let lines: Vec<Line<'static>> = view
        .entries()
        .iter()
        .enumerate()
        .skip(top)
        .take(viewport)
        .map(|(index, entry)| row_line(entry, index == view.selected(), view, columns, config))
        .collect();
    if view.entries().is_empty() {
        let note = if view.is_loading() {
            "reading the trash…"
        } else if view.failure().is_some() {
            ""
        } else {
            "the trash is empty"
        };
        frame.render_widget(Paragraph::new(Span::styled(note, dim)), places.rows);
    } else {
        frame.render_widget(Paragraph::new(lines), places.rows);
    }

    let footer = match (view.question(), view.note()) {
        (Some(question), _) => Span::styled(fit_width(&question, width), danger),
        (None, Some(note)) => Span::styled(fit_width(note, width), accent),
        (None, None) => Span::styled(
            fit_width(
                "j/k move  v mark  r restore  x delete for good  E empty  q close",
                width,
            ),
            dim,
        ),
    };
    frame.render_widget(Paragraph::new(footer), places.footer);

    hud::render_scrollbar(
        frame,
        places.frame,
        view.entries().len(),
        view.selected(),
        config,
    );
}

fn row_line(
    entry: &TrashEntry,
    selected: bool,
    view: &TrashView,
    columns: Columns,
    config: &Config,
) -> Line<'static> {
    let (theme, g) = (&config.theme, glyphs::of(config));
    let dim = Style::default().fg(style::color(&theme.status_fg));
    let marked = view.is_marked(entry);
    let background = if selected {
        Style::default().bg(style::color(&theme.selection_bg))
    } else {
        Style::default()
    };
    let name_color = match entry.size {
        EntrySize::Entries(_) => &theme.dir_fg,
        _ => &theme.file_fg,
    };
    let mut name_style = Style::default().fg(style::color(name_color));
    if marked {
        name_style = style::styled(name_style, config.styles.mark);
    }
    let mut spans = vec![
        Span::styled(
            if selected {
                format!("{} ", g.stripe)
            } else {
                "  ".to_string()
            },
            Style::default().fg(style::color(&theme.accent_fg)),
        ),
        Span::styled(
            if marked { "* " } else { "  " },
            Style::default().fg(style::color(&theme.accent_fg)),
        ),
        Span::styled(pad_to(&entry.name, columns.name), name_style),
    ];
    if columns.size {
        spans.push(Span::styled(
            format!(" {}", pad_left(&size_cell(entry.size), SIZE_WIDTH)),
            dim,
        ));
    }
    if columns.date {
        spans.push(Span::styled(
            format!(" {}", pad_to(&deleted_cell(entry.deleted), DATE_WIDTH)),
            dim,
        ));
    }
    if columns.folder > 0 {
        spans.push(Span::styled(
            format!(
                " {}",
                pad_to(
                    &elide_left(&entry.original_dir.display().to_string(), columns.folder),
                    columns.folder
                )
            ),
            dim,
        ));
    }
    Line::from(spans).style(background)
}

fn pad_left(s: &str, width: usize) -> String {
    let fitted = fit_width(s, width);
    format!(
        "{}{fitted}",
        " ".repeat(width.saturating_sub(text_width(&fitted)))
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::Instant;

    use proptest::prelude::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn entry(id: &str, name: &str, secs: i64, size: EntrySize) -> TrashEntry {
        TrashEntry {
            id: id.into(),
            name: name.into(),
            original_dir: PathBuf::from("/home/me/docs"),
            deleted: secs,
            size,
            stored_path: Some(PathBuf::from("/trash/files").join(name)),
        }
    }

    /// A trash held in memory. An entry named `clash` cannot be restored, one named `stuck` cannot
    /// be deleted; reading can be made to fail.
    struct Fake {
        items: Mutex<Vec<TrashEntry>>,
        unreadable: bool,
        calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(items: Vec<TrashEntry>) -> Arc<Self> {
            Arc::new(Self {
                items: Mutex::new(items),
                unreadable: false,
                calls: Mutex::new(Vec::new()),
            })
        }

        fn names(&self) -> Vec<String> {
            self.items
                .lock()
                .unwrap()
                .iter()
                .map(|e| e.name.clone())
                .collect()
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn take(
            &self,
            entries: &[TrashEntry],
            fails: &str,
            label: &str,
        ) -> Vec<Result<(), String>> {
            self.calls.lock().unwrap().push(format!(
                "{label}:{}",
                entries
                    .iter()
                    .map(|e| e.name.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
            entries
                .iter()
                .map(|entry| {
                    if entry.name == fails {
                        return Err(format!("{} is in the way", entry.name));
                    }
                    self.items.lock().unwrap().retain(|e| e.id != entry.id);
                    Ok(())
                })
                .collect()
        }
    }

    impl TrashBackend for Fake {
        fn entries(&self) -> Result<Vec<TrashEntry>, String> {
            match self.unreadable {
                true => Err("permission denied".into()),
                false => Ok(self.items.lock().unwrap().clone()),
            }
        }

        fn restore(&self, entries: &[TrashEntry]) -> Vec<Result<(), String>> {
            self.take(entries, "clash", "restore")
        }

        fn purge(&self, entries: &[TrashEntry]) -> Vec<Result<(), String>> {
            self.take(entries, "stuck", "purge")
        }
    }

    fn sample() -> Arc<Fake> {
        Fake::new(vec![
            entry("3", "newest.txt", 300, EntrySize::Bytes(2048)),
            entry("2", "clash", 200, EntrySize::Bytes(10)),
            entry("1", "folder", 100, EntrySize::Entries(4)),
        ])
    }

    fn open(fake: &Arc<Fake>) -> (tokio::runtime::Runtime, TrashView) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let backend: Arc<dyn TrashBackend> = fake.clone();
        let mut view = TrashView::open(runtime.handle().clone(), backend);
        settle(&mut view);
        (runtime, view)
    }

    fn settle(view: &mut TrashView) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            view.poll();
            if !view.is_loading() && !view.is_working() && view.loading.is_none() {
                return;
            }
            assert!(Instant::now() < deadline, "the trash view never settled");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn press(view: &mut TrashView, code: KeyCode) -> bool {
        let action = match code {
            KeyCode::Char('j') => Some(Action::MoveDown),
            KeyCode::Char('k') => Some(Action::MoveUp),
            KeyCode::Char('c') => Some(Action::Cancel),
            KeyCode::Char('q') => Some(Action::Quit),
            _ => None,
        };
        let close = view.key(code, action);
        settle(view);
        close
    }

    fn ch(view: &mut TrashView, c: char) -> bool {
        press(view, KeyCode::Char(c))
    }

    #[test]
    fn it_lists_what_the_trash_holds_in_the_order_it_gives() {
        let (_rt, view) = open(&sample());
        let names: Vec<&str> = view.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["newest.txt", "clash", "folder"]);
        assert_eq!(view.selected(), 0);
        assert!(view.failure().is_none());
    }

    #[test]
    fn a_trash_that_cannot_be_read_says_why() {
        let fake = Arc::new(Fake {
            items: Mutex::new(Vec::new()),
            unreadable: true,
            calls: Mutex::new(Vec::new()),
        });
        let (_rt, view) = open(&fake);
        assert_eq!(view.failure(), Some("permission denied"));
        assert!(view.entries().is_empty());
    }

    #[test]
    fn the_cursor_moves_and_stops_at_both_ends() {
        let (_rt, mut view) = open(&sample());
        ch(&mut view, 'k');
        assert_eq!(view.selected(), 0);
        ch(&mut view, 'j');
        ch(&mut view, 'j');
        ch(&mut view, 'j');
        assert_eq!(view.selected(), 2);
        press(&mut view, KeyCode::Home);
        assert_eq!(view.selected(), 0);
        press(&mut view, KeyCode::End);
        assert_eq!(view.selected(), 2);
    }

    #[test]
    fn v_marks_and_unmarks_the_entry_under_the_cursor_and_only_v_does() {
        let (_rt, mut view) = open(&sample());
        ch(&mut view, 'v');
        assert_eq!(view.marked_count(), 1);
        assert!(view.is_marked(&view.entries()[0]));
        ch(&mut view, 'v');
        assert_eq!(view.marked_count(), 0);

        // Moving, restoring and deleting never mark anything on their own.
        ch(&mut view, 'j');
        ch(&mut view, 'r');
        ch(&mut view, 'x');
        assert_eq!(view.marked_count(), 0);
    }

    #[test]
    fn nothing_acts_on_the_cursor_row_when_nothing_is_marked() {
        let fake = sample();
        let (_rt, mut view) = open(&fake);
        ch(&mut view, 'r');
        assert_eq!(view.note(), Some(NOTHING_MARKED));
        ch(&mut view, 'x');
        assert_eq!(view.note(), Some(NOTHING_MARKED));
        assert!(view.ask().is_none(), "no confirmation for nothing");
        assert!(fake.calls().is_empty(), "the backend was never asked");
        assert_eq!(fake.names().len(), 3);
    }

    #[test]
    fn restore_puts_back_the_marked_entries_and_leaves_the_rest() {
        let fake = sample();
        let (_rt, mut view) = open(&fake);
        ch(&mut view, 'v'); // newest.txt
        ch(&mut view, 'j');
        ch(&mut view, 'j');
        ch(&mut view, 'v'); // folder
        ch(&mut view, 'r');

        assert_eq!(
            fake.calls(),
            ["restore:newest.txt,folder"],
            "in listed order"
        );
        assert_eq!(fake.names(), ["clash"]);
        let names: Vec<&str> = view.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["clash"], "the list is read again");
        assert_eq!(view.note(), Some("restored 2 items"));
        assert_eq!(
            view.marked_count(),
            0,
            "what was marked is gone, so are its marks"
        );
    }

    #[test]
    fn a_restore_that_cannot_go_back_says_why_and_the_entry_stays() {
        let fake = sample();
        let (_rt, mut view) = open(&fake);
        ch(&mut view, 'j');
        ch(&mut view, 'v'); // clash
        ch(&mut view, 'k');
        ch(&mut view, 'v'); // newest.txt
        ch(&mut view, 'r');

        assert_eq!(view.note(), Some("restored 1 of 2 — clash is in the way"));
        assert_eq!(fake.names(), ["clash", "folder"]);
        assert_eq!(
            view.marked_count(),
            1,
            "the one that failed is still marked"
        );
    }

    #[test]
    fn deleting_for_good_asks_first_and_only_y_goes_ahead() {
        let fake = sample();
        let (_rt, mut view) = open(&fake);
        ch(&mut view, 'v');
        ch(&mut view, 'x');
        assert_eq!(view.ask(), Some(Ask::Purge));
        assert_eq!(
            view.question().as_deref(),
            Some("delete 1 marked item for good? (y/N)")
        );
        assert!(fake.calls().is_empty(), "nothing yet");

        ch(&mut view, 'n');
        assert!(view.ask().is_none());
        assert_eq!(view.note(), Some("cancelled"));
        assert!(fake.calls().is_empty());
        assert_eq!(view.marked_count(), 1, "declining keeps the mark");

        ch(&mut view, 'x');
        ch(&mut view, 'y');
        assert_eq!(fake.calls(), ["purge:newest.txt"]);
        assert_eq!(fake.names(), ["clash", "folder"]);
        assert_eq!(view.note(), Some("deleted for good: 1 item"));
    }

    #[test]
    fn any_key_but_y_declines_including_enter_and_escape() {
        for code in [
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Char('Y'),
            KeyCode::Char(' '),
        ] {
            let fake = sample();
            let (_rt, mut view) = open(&fake);
            ch(&mut view, 'v');
            ch(&mut view, 'x');
            let closed = press(&mut view, code);
            assert!(
                !closed,
                "{code:?} answers the question, it does not close the view"
            );
            assert!(fake.calls().is_empty(), "{code:?}");
        }
    }

    #[test]
    fn emptying_the_trash_asks_and_removes_everything() {
        let fake = sample();
        let (_rt, mut view) = open(&fake);
        ch(&mut view, 'E');
        assert_eq!(view.ask(), Some(Ask::Empty));
        assert_eq!(
            view.question().as_deref(),
            Some("empty the whole trash, 3 items? (y/N)")
        );
        ch(&mut view, 'y');
        assert!(fake.names().is_empty());
        assert!(view.entries().is_empty());
        assert_eq!(view.note(), Some("deleted for good: 3 items"));
        ch(&mut view, 'E');
        assert_eq!(view.note(), Some("the trash is already empty"));
    }

    #[test]
    fn emptying_does_not_depend_on_marks() {
        let fake = sample();
        let (_rt, mut view) = open(&fake);
        assert_eq!(view.marked_count(), 0);
        ch(&mut view, 'E');
        ch(&mut view, 'y');
        assert!(fake.names().is_empty());
    }

    #[test]
    fn a_delete_that_fails_for_one_entry_reports_it_and_deletes_the_rest() {
        let fake = Fake::new(vec![
            entry("1", "gone-1", 3, EntrySize::Bytes(1)),
            entry("2", "stuck", 2, EntrySize::Bytes(1)),
            entry("3", "gone-3", 1, EntrySize::Bytes(1)),
        ]);
        let (_rt, mut view) = open(&fake);
        ch(&mut view, 'E');
        ch(&mut view, 'y');
        assert_eq!(
            view.note(),
            Some("deleted for good: 2 of 3 — stuck is in the way")
        );
        assert_eq!(fake.names(), ["stuck"]);
    }

    #[test]
    fn c_clears_the_marks_and_q_or_escape_close_the_view() {
        let (_rt, mut view) = open(&sample());
        ch(&mut view, 'v');
        ch(&mut view, 'c');
        assert_eq!(view.marked_count(), 0);
        assert!(ch(&mut view, 'q'));
        let (_rt, mut view) = open(&sample());
        assert!(press(&mut view, KeyCode::Esc));
    }

    #[test]
    fn keys_are_ignored_while_a_restore_or_delete_runs() {
        let fake = sample();
        let (_rt, mut view) = open(&fake);
        ch(&mut view, 'v');
        // Start a restore and press keys before it is polled.
        view.key(KeyCode::Char('r'), None);
        assert!(view.is_working());
        assert!(
            !view.key(KeyCode::Esc, None),
            "Esc does not close mid-operation"
        );
        assert!(!view.key(KeyCode::Char('E'), None));
        settle(&mut view);
        assert!(!view.is_working());
    }

    #[test]
    fn the_columns_thin_out_name_last() {
        let wide = plan(120);
        assert!(wide.size && wide.date && wide.folder > 0);
        let medium = plan(60);
        assert!(medium.size && medium.date && medium.folder == 0);
        let narrow = plan(40);
        assert!(narrow.size && !narrow.date);
        let tiny = plan(20);
        assert!(!tiny.size && !tiny.date && tiny.name >= MIN_NAME.min(20 - PREFIX));
        assert_eq!(plan(0).name, 0);
    }

    proptest! {
        /// The columns never add up to more than the width they were planned for.
        #[test]
        fn a_plan_never_overflows_its_width(width in 0usize..300) {
            let c = plan(width);
            let used = PREFIX
                + c.name
                + if c.size { 1 + SIZE_WIDTH } else { 0 }
                + if c.date { 1 + DATE_WIDTH } else { 0 }
                + if c.folder > 0 { 1 + c.folder } else { 0 };
            prop_assert!(used <= width.max(PREFIX));
        }
    }

    #[test]
    fn sizes_dates_and_folders_read_as_text() {
        assert_eq!(size_cell(EntrySize::Bytes(2048)), "2.0K");
        assert_eq!(size_cell(EntrySize::Entries(1)), "1 item");
        assert_eq!(size_cell(EntrySize::Entries(4)), "4 items");
        assert_eq!(size_cell(EntrySize::Unknown), "—");
        assert_eq!(deleted_cell(-1), "—");
        let cell = deleted_cell(1_700_000_000);
        assert_eq!(text_width(&cell), hud::MODIFIED_WIDTH, "{cell}");
        assert_eq!(elide_left("/home/me/docs", 40), "/home/me/docs");
        assert_eq!(elide_left("/home/me/docs/deep/er", 10), "…s/deep/er");
        assert_eq!(elide_left("/a", 0), "");
        for width in 0..20 {
            assert!(text_width(&elide_left("/home/me/docs/deep/er/more", width)) <= width);
        }
    }

    #[test]
    fn the_scroll_keeps_the_cursor_on_screen_and_fills_the_window() {
        assert_eq!(scroll_top(0, 10, 0, 50), 0);
        assert_eq!(scroll_top(12, 10, 0, 50), 3);
        assert_eq!(scroll_top(2, 10, 5, 50), 2);
        assert_eq!(scroll_top(49, 10, 0, 50), 40);
        assert_eq!(
            scroll_top(5, 10, 40, 12),
            2,
            "no blank space under the last row"
        );
        assert_eq!(scroll_top(0, 0, 0, 5), 0);
        assert_eq!(scroll_top(0, 10, 0, 0), 0);
    }

    #[test]
    fn a_click_finds_its_row_inside_the_list_only() {
        let places = layout(Rect::new(0, 0, 80, 20));
        let first = Position::new(places.rows.x + 3, places.rows.y);
        assert_eq!(row_at(&places, 0, 5, first), Some(0));
        assert_eq!(row_at(&places, 7, 50, first), Some(7));
        let past = Position::new(places.rows.x, places.rows.y + 6);
        assert_eq!(row_at(&places, 0, 5, past), None, "below the last row");
        assert_eq!(
            row_at(&places, 0, 5, Position::new(0, 0)),
            None,
            "on the frame"
        );
        assert_eq!(
            row_at(
                &places,
                0,
                5,
                Position::new(places.footer.x, places.footer.y)
            ),
            None
        );
    }

    fn drawn(view: &TrashView, width: u16, height: u16) -> String {
        let config = Config::from_sources(None, None, None);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, view, &config)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_screen_shows_each_entry_with_its_size_folder_and_marks() {
        let (_rt, mut view) = open(&sample());
        ch(&mut view, 'v');
        let screen = drawn(&view, 99, 12);
        assert!(screen.contains("3 items, 1 marked"), "{screen}");
        assert!(
            screen.contains("newest.txt") && screen.contains("2.0K"),
            "{screen}"
        );
        assert!(
            screen.contains("folder") && screen.contains("4 items"),
            "{screen}"
        );
        assert!(screen.contains("/home/me/docs"), "{screen}");
        assert!(
            screen.contains("* newest.txt"),
            "a marked row shows its mark: {screen}"
        );
        assert!(
            screen.contains("v mark") && screen.contains("E empty"),
            "{screen}"
        );
    }

    #[test]
    fn a_wide_screen_gets_a_preview_pane_beside_the_list_and_a_narrow_one_does_not() {
        let narrow = layout(Rect::new(0, 0, PREVIEW_MIN_WIDTH - 1, 30));
        assert_eq!(narrow.preview, None);
        assert_eq!(narrow.frame, Rect::new(0, 0, PREVIEW_MIN_WIDTH - 1, 30));

        let wide = layout(Rect::new(0, 0, 140, 30));
        let preview = wide.preview.expect("room for a pane");
        assert_eq!(
            wide.frame.x + wide.frame.width,
            preview.x,
            "side by side, no gap"
        );
        assert_eq!(
            wide.frame.width + preview.width,
            140,
            "together they fill the screen"
        );
        assert!(
            wide.frame.width > preview.width,
            "the list is the larger part"
        );
        assert!(preview.width >= 40, "wide enough to read: {preview:?}");
        assert!(
            wide.rows.right() <= wide.frame.right(),
            "the rows stay inside the list's frame"
        );
    }

    proptest! {
        /// Whatever the screen, the list and the pane never overlap and never leave it.
        #[test]
        fn the_list_and_the_pane_partition_the_screen(w in 0u16..400, h in 0u16..100) {
            let screen = Rect::new(0, 0, w, h);
            let places = layout(screen);
            prop_assert_eq!(places.frame.intersection(screen), places.frame);
            if let Some(preview) = places.preview {
                prop_assert_eq!(preview.intersection(screen), preview);
                prop_assert_eq!(places.frame.intersection(preview).area(), 0);
                prop_assert_eq!(places.frame.width + preview.width, w);
            }
            prop_assert_eq!(places.rows.intersection(places.frame), places.rows);
        }
    }

    #[test]
    fn the_preview_subject_is_the_stored_file_described_for_the_browsers_pipelines() {
        let (_rt, mut view) = open(&sample());
        let file = view
            .preview_subject()
            .expect("the first entry has a stored path");
        assert_eq!(file.path, PathBuf::from("/trash/files/newest.txt"));
        assert_eq!(file.name, "newest.txt");
        assert!(!file.is_dir);
        assert_eq!(file.size, 2048);

        ch(&mut view, 'j');
        ch(&mut view, 'j');
        let folder = view.preview_subject().unwrap();
        assert_eq!(folder.path, PathBuf::from("/trash/files/folder"));
        assert!(
            folder.is_dir,
            "an entry with a count of entries is a folder"
        );
        assert_eq!(view.selected_name(), Some("folder"));
    }

    #[test]
    fn with_nowhere_to_read_it_from_there_is_no_subject_but_the_name_remains() {
        let mut odd = entry("1", "mystery", 1, EntrySize::Unknown);
        odd.stored_path = None;
        let (_rt, view) = open(&Fake::new(vec![odd]));
        assert!(view.preview_subject().is_none());
        assert_eq!(view.selected_name(), Some("mystery"));

        let (_rt, empty) = open(&Fake::new(Vec::new()));
        assert!(empty.preview_subject().is_none());
        assert_eq!(empty.selected_name(), None);
    }

    #[test]
    fn the_subject_follows_the_cursor() {
        let (_rt, mut view) = open(&sample());
        let first = view.preview_subject().unwrap().path;
        ch(&mut view, 'j');
        assert_ne!(view.preview_subject().unwrap().path, first);
        ch(&mut view, 'k');
        assert_eq!(view.preview_subject().unwrap().path, first);
    }

    #[test]
    fn on_a_wide_screen_the_list_is_framed_on_its_own_and_still_shows_its_rows() {
        let (_rt, mut view) = open(&sample());
        ch(&mut view, 'v');
        let screen = drawn(&view, 140, 12);
        let first_line = screen.lines().next().unwrap();
        assert!(first_line.starts_with("╭ trash "), "{first_line}");
        assert!(
            first_line.contains("╮"),
            "the list has its own right corner: {first_line}"
        );
        assert!(
            screen.contains("3 items, 1 marked") && screen.contains("* newest.txt"),
            "{screen}"
        );
        assert!(
            screen.contains("2.0K") && screen.contains("4 items"),
            "{screen}"
        );
    }

    #[test]
    fn the_screen_asks_its_question_and_shows_what_happened() {
        let (_rt, mut view) = open(&sample());
        ch(&mut view, 'E');
        let screen = drawn(&view, 100, 10);
        assert!(
            screen.contains("empty the whole trash, 3 items? (y/N)"),
            "{screen}"
        );
        ch(&mut view, 'n');
        assert!(drawn(&view, 100, 10).contains("cancelled"));
    }

    #[test]
    fn an_empty_or_unreadable_trash_is_drawn_as_such() {
        let (_rt, view) = open(&Fake::new(Vec::new()));
        assert!(drawn(&view, 80, 8).contains("the trash is empty"));
        let fake = Arc::new(Fake {
            items: Mutex::new(Vec::new()),
            unreadable: true,
            calls: Mutex::new(Vec::new()),
        });
        let (_rt, view) = open(&fake);
        assert!(drawn(&view, 80, 8).contains("cannot read the trash: permission denied"));
    }

    #[test]
    fn a_tiny_terminal_does_not_panic() {
        let (_rt, view) = open(&sample());
        for (w, h) in [(1, 1), (5, 3), (10, 4), (20, 5), (30, 2)] {
            let _ = drawn(&view, w, h);
        }
    }
}
