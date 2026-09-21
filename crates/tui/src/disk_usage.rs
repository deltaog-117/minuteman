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

//! The disk usage view's data: what each thing in a folder takes up, biggest first, and where the
//! user is in it as they open folders to see what fills them.
//!
//! Sizes come from a background scan (`scan_level`) of one folder at a time, run on the blocking
//! pool the way `inspect` and `marked_size` run theirs: it sends each finished row back over a
//! channel and stops the moment its job is dropped. Only the folder being looked at is scanned;
//! opening a subfolder scans that one, and going back restores the level above from memory, so
//! memory holds the rows of the folders on the way down, never a whole tree.
//!
//! Three rules decide what a size means. It is the space on disk (allocated blocks, as `du`
//! shows) by default, with the file's own length available as `Measure::Apparent`, since a sparse
//! or compressed file differs a lot; both are recorded in one scan, so switching costs nothing. A
//! file with several hard links is counted once, at the first place the scan meets it. And the scan
//! stays on the filesystem it started on and never follows a symlink (a link counts as the link),
//! so scanning `/` does not wander into `/proc`, a network mount, or a symlink loop.
//!
//! A folder with a million files must not cost a million rows of memory, so each level keeps at
//! most `MAX_ROWS` of its biggest files and folds the rest into one `Kind::Rest` row.

use std::cell::Cell;
use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// Entries one scan looks at before it stops and calls its answer a lower bound.
pub const MAX_ENTRIES: u64 = 10_000_000;

/// Rows kept per folder; the rest are folded into one.
pub const MAX_ROWS: usize = 20_000;

/// The two sizes recorded for everything, so the view can switch between them without scanning
/// again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sizes {
    /// Allocated on disk, in whole blocks.
    pub disk: u64,
    /// The length of the file (for a folder, of everything in it).
    pub apparent: u64,
}

impl Sizes {
    fn plus(self, other: Sizes) -> Sizes {
        Sizes {
            disk: self.disk.saturating_add(other.disk),
            apparent: self.apparent.saturating_add(other.apparent),
        }
    }

    pub fn of(self, measure: Measure) -> u64 {
        match measure {
            Measure::OnDisk => self.disk,
            Measure::Apparent => self.apparent,
        }
    }

    /// The larger of the two: what decides whether a row is big enough to keep, whichever
    /// measure is later chosen.
    fn weight(self) -> u64 {
        self.disk.max(self.apparent)
    }
}

/// Which of the two sizes the view shows and sorts by.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Measure {
    #[default]
    OnDisk,
    Apparent,
}

impl Measure {
    pub fn toggled(self) -> Self {
        match self {
            Measure::OnDisk => Measure::Apparent,
            Measure::Apparent => Measure::OnDisk,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Measure::OnDisk => "on disk",
            Measure::Apparent => "apparent size",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A folder; its sizes are everything below it.
    Dir,
    File,
    /// A symlink, counted as the link and not followed.
    Link,
    /// Sockets, pipes, devices.
    Other,
    /// A folder on another filesystem, which the scan does not enter.
    OtherFilesystem,
    /// Everything beyond `MAX_ROWS`, folded into one row.
    Rest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Cleaned of control characters, so safe to draw.
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub sizes: Sizes,
    /// What is inside: entries below a folder, entries folded into `Rest`, 1 otherwise.
    pub items: u64,
    /// A folder below this one could not be read, so the size is a lower bound.
    pub partial: bool,
}

/// How a scan of one level ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Complete,
    /// Hit `MAX_ENTRIES`; the sizes are a lower bound.
    Truncated,
    Cancelled,
    /// The folder itself could not be read.
    Unreadable,
}

/// Everything the scan of one folder needs to keep track of.
struct Walk<'a> {
    root_dev: u64,
    cancel: &'a AtomicBool,
    seen: &'a AtomicU64,
    /// Entries looked at by this scan; compared with `limit`.
    count: u64,
    limit: u64,
    /// Files with more than one link that have been counted, by `(device, inode)`.
    links: HashSet<(u64, u64)>,
}

impl Walk<'_> {
    /// Notes one more entry looked at, and says why the scan must stop if it must.
    fn tick(&mut self) -> Result<(), End> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(End::Cancelled);
        }
        if self.count >= self.limit {
            return Err(End::Truncated);
        }
        self.count += 1;
        self.seen.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// The sizes `meta` adds: nothing for a second link to a file already counted.
    fn sizes_of(&mut self, meta: &std::fs::Metadata) -> Sizes {
        if !meta.is_dir() && meta.nlink() > 1 && !self.links.insert((meta.dev(), meta.ino())) {
            return Sizes::default();
        }
        Sizes {
            // `st_blocks` counts 512-byte units whatever the filesystem's block size is.
            disk: meta.blocks().saturating_mul(512),
            apparent: meta.len(),
        }
    }
}

/// What a walk of one folder found below it.
struct Below {
    sizes: Sizes,
    items: u64,
    partial: bool,
}

/// Adds up everything below `dir` (and `dir`'s own node), staying on the root's filesystem and not
/// following links.
fn walk_dir(dir: &Path, own: &std::fs::Metadata, walk: &mut Walk<'_>) -> Result<Below, End> {
    let mut below = Below {
        sizes: walk.sizes_of(own),
        items: 0,
        partial: false,
    };
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            below.partial = true;
            continue;
        };
        for entry in entries.flatten() {
            walk.tick()?;
            let Ok(meta) = entry.metadata() else {
                below.partial = true;
                continue;
            };
            // `DirEntry::metadata` does not follow a symlink, so a link is a link here.
            if meta.is_dir() {
                if meta.dev() != walk.root_dev {
                    continue;
                }
                below.sizes = below.sizes.plus(walk.sizes_of(&meta));
                below.items += 1;
                pending.push(entry.path());
            } else {
                below.sizes = below.sizes.plus(walk.sizes_of(&meta));
                below.items += 1;
            }
        }
    }
    Ok(below)
}

fn kind_of(meta: &std::fs::Metadata) -> Kind {
    let file_type = meta.file_type();
    if file_type.is_symlink() {
        Kind::Link
    } else if file_type.is_file() {
        Kind::File
    } else {
        Kind::Other
    }
}

fn row_name(path: &Path) -> String {
    preview::archive::clean_name(&path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    ))
}

/// The files of a folder, keeping only the biggest `MAX_ROWS` however many there are.
#[derive(Default)]
struct Files {
    rows: Vec<Row>,
    rest: Sizes,
    rest_items: u64,
}

impl Files {
    fn push(&mut self, row: Row) {
        self.rows.push(row);
        // Folding only when the list has doubled keeps the sorting cost per file constant.
        if self.rows.len() >= 2 * MAX_ROWS {
            self.fold();
        }
    }

    fn fold(&mut self) {
        self.rows
            .sort_unstable_by_key(|row| std::cmp::Reverse(row.sizes.weight()));
        for row in self.rows.drain(MAX_ROWS..) {
            self.rest = self.rest.plus(row.sizes);
            self.rest_items += 1;
        }
    }

    fn finish(mut self) -> (Vec<Row>, Sizes, u64) {
        if self.rows.len() > MAX_ROWS {
            self.fold();
        }
        (self.rows, self.rest, self.rest_items)
    }
}

/// Scans the folder `root`, passing each batch of finished rows to `emit`: first every file
/// and link in the folder at once, then each subfolder as soon as its total is known. Stops early
/// when `cancel` is set or after `limit` entries. `seen` counts entries looked at, for a progress
/// display.
pub fn scan_level(
    root: &Path,
    limit: u64,
    cancel: &AtomicBool,
    seen: &AtomicU64,
    emit: &mut dyn FnMut(Vec<Row>),
) -> End {
    let Ok(meta) = std::fs::symlink_metadata(root) else {
        return End::Unreadable;
    };
    scan_level_on(root, meta.dev(), limit, cancel, seen, emit)
}

/// `scan_level` with the filesystem to stay on given, so tests can pretend everything below is on
/// another one.
fn scan_level_on(
    root: &Path,
    root_dev: u64,
    limit: u64,
    cancel: &AtomicBool,
    seen: &AtomicU64,
    emit: &mut dyn FnMut(Vec<Row>),
) -> End {
    let Ok(entries) = std::fs::read_dir(root) else {
        return End::Unreadable;
    };
    let mut walk = Walk {
        root_dev,
        cancel,
        seen,
        count: 0,
        limit,
        links: HashSet::new(),
    };
    let mut files = Files::default();
    let mut dirs: Vec<(PathBuf, std::fs::Metadata)> = Vec::new();
    let mut other_filesystems = Vec::new();

    let mut ending = End::Complete;
    for entry in entries.flatten() {
        match walk.tick() {
            Ok(()) => {}
            Err(End::Cancelled) => return End::Cancelled,
            Err(end) => {
                // Out of budget: list what was found rather than nothing.
                ending = end;
                break;
            }
        }
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            if meta.dev() == root_dev {
                dirs.push((path, meta));
            } else {
                other_filesystems.push(Row {
                    name: row_name(&path),
                    path,
                    kind: Kind::OtherFilesystem,
                    sizes: Sizes::default(),
                    items: 0,
                    partial: false,
                });
            }
        } else {
            let sizes = walk.sizes_of(&meta);
            files.push(Row {
                name: row_name(&path),
                path,
                kind: kind_of(&meta),
                sizes,
                items: 1,
                partial: false,
            });
        }
    }

    let (mut rows, mut rest, mut rest_items) = files.finish();
    rows.extend(other_filesystems);
    emit(rows);

    let mut folders_sent = 0usize;
    for (path, meta) in dirs {
        match walk_dir(&path, &meta, &mut walk) {
            Ok(below) if folders_sent < MAX_ROWS => {
                folders_sent += 1;
                emit(vec![Row {
                    name: row_name(&path),
                    path,
                    kind: Kind::Dir,
                    sizes: below.sizes,
                    items: below.items,
                    partial: below.partial,
                }]);
            }
            Ok(below) => {
                rest = rest.plus(below.sizes);
                rest_items += 1 + below.items;
            }
            Err(End::Truncated) => {
                ending = End::Truncated;
                break;
            }
            Err(end) => return end,
        }
    }
    if rest_items > 0 {
        emit(vec![Row {
            name: format!("{rest_items} smaller entries"),
            path: PathBuf::new(),
            kind: Kind::Rest,
            sizes: rest,
            items: rest_items,
            partial: false,
        }]);
    }
    ending
}

/// The order rows are listed in: biggest first by `measure`, then by name so equal sizes do not
/// shuffle between frames.
fn compare(a: &Row, b: &Row, measure: Measure) -> std::cmp::Ordering {
    b.sizes
        .of(measure)
        .cmp(&a.sizes.of(measure))
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.path.cmp(&b.path))
}

enum Message {
    Rows(Vec<Row>),
    Done(End),
}

struct Job {
    cancel: Arc<AtomicBool>,
    rx: UnboundedReceiver<Message>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// One folder's rows, and where the cursor is in them.
struct Level {
    path: PathBuf,
    /// Sorted by `compare`.
    rows: Vec<Row>,
    selected: usize,
    /// `None` while the scan is running.
    end: Option<End>,
    seen: Arc<AtomicU64>,
    /// A path to put the cursor on once its row arrives (the folder just came up out of).
    reselect: Option<PathBuf>,
    /// Whether the user has moved the cursor. Until then it stays on the first row, which is
    /// always the biggest so far, instead of following whichever row happened to arrive first
    /// while bigger ones sort in above it.
    moved: bool,
}

impl Level {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            rows: Vec::new(),
            selected: 0,
            end: None,
            seen: Arc::new(AtomicU64::new(0)),
            reselect: None,
            moved: false,
        }
    }

    fn selected_path(&self) -> Option<&PathBuf> {
        self.rows.get(self.selected).map(|row| &row.path)
    }

    /// Puts the rows in order and the cursor back on the row it was on (or on the row asked for
    /// with `reselect`, once that row has arrived).
    fn sort(&mut self, measure: Measure) {
        let waiting = self.reselect.take();
        let want = if self.moved || waiting.is_some() {
            waiting.clone().or_else(|| self.selected_path().cloned())
        } else {
            None
        };
        self.rows.sort_by(|a, b| compare(a, b, measure));
        let found = want
            .as_ref()
            .and_then(|path| self.rows.iter().position(|row| &row.path == path));
        self.selected = found.unwrap_or(0);
        // The row to land on may not have been scanned yet.
        if found.is_none() && self.end.is_none() {
            self.reselect = waiting;
        }
    }
}

/// The disk usage view: a stack of levels, the top one being looked at.
pub struct DiskUsageView {
    handle: tokio::runtime::Handle,
    levels: Vec<Level>,
    measure: Measure,
    job: Option<Job>,
    /// First row shown and rows that fit, kept from the last draw: a key moves by a page and a
    /// click finds its row from them, and drawing only has `&self`.
    top: Cell<usize>,
    viewport: Cell<usize>,
}

impl DiskUsageView {
    /// Opens the view on `path` and starts scanning it.
    pub fn open(handle: tokio::runtime::Handle, path: PathBuf) -> Self {
        let mut view = Self {
            handle,
            levels: vec![Level::new(path)],
            measure: Measure::default(),
            job: None,
            top: Cell::new(0),
            viewport: Cell::new(0),
        };
        view.start_scan();
        view
    }

    fn level(&self) -> &Level {
        self.levels.last().expect("there is always a level")
    }

    fn level_mut(&mut self) -> &mut Level {
        self.levels.last_mut().expect("there is always a level")
    }

    fn start_scan(&mut self) {
        let (tx, rx) = unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_bg = Arc::clone(&cancel);
        let level = self.level_mut();
        level.end = None;
        level.rows.clear();
        level.selected = 0;
        level.moved = false;
        let (path, seen) = (level.path.clone(), Arc::clone(&level.seen));
        seen.store(0, Ordering::Relaxed);
        // Replacing the job drops the old one, which cancels its scan.
        self.job = Some(Job { cancel, rx });
        self.handle.spawn_blocking(move || {
            let end = scan_level(&path, MAX_ENTRIES, &cancel_bg, &seen, &mut |rows| {
                let _ = tx.send(Message::Rows(rows));
            });
            let _ = tx.send(Message::Done(end));
        });
    }

    /// Picks up rows the scan has finished. Call once per render tick.
    pub fn poll(&mut self) {
        let Some(job) = self.job.as_mut() else { return };
        let (mut added, mut done) = (Vec::new(), None);
        loop {
            match job.rx.try_recv() {
                Ok(Message::Rows(rows)) => added.extend(rows),
                Ok(Message::Done(end)) => {
                    done = Some(end);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    done = Some(End::Cancelled);
                    break;
                }
            }
        }
        let measure = self.measure;
        let level = self.level_mut();
        if let Some(end) = done {
            level.end = Some(end);
        }
        if !added.is_empty() || done.is_some() {
            level.rows.extend(added);
            level.sort(measure);
        }
        if done.is_some() {
            self.job = None;
        }
    }

    pub fn path(&self) -> &Path {
        &self.level().path
    }

    pub fn rows(&self) -> &[Row] {
        &self.level().rows
    }

    pub fn selected(&self) -> usize {
        self.level().selected
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.level().rows.get(self.level().selected)
    }

    pub fn measure(&self) -> Measure {
        self.measure
    }

    /// `None` while the scan is running.
    pub fn end(&self) -> Option<End> {
        self.level().end
    }

    /// Entries the scan has looked at so far.
    pub fn seen(&self) -> u64 {
        self.level().seen.load(Ordering::Relaxed)
    }

    /// What the listed rows add up to, by the current measure.
    pub fn total(&self) -> u64 {
        self.rows().iter().fold(0u64, |sum, row| {
            sum.saturating_add(row.sizes.of(self.measure))
        })
    }

    pub fn top(&self) -> &Cell<usize> {
        &self.top
    }

    pub fn viewport(&self) -> &Cell<usize> {
        &self.viewport
    }

    pub fn move_by(&mut self, rows: isize) {
        let level = self.level_mut();
        let last = level.rows.len().saturating_sub(1);
        level.selected = level.selected.saturating_add_signed(rows).min(last);
        level.moved = true;
    }

    pub fn move_to(&mut self, index: usize) {
        let level = self.level_mut();
        level.selected = index.min(level.rows.len().saturating_sub(1));
        level.moved = true;
    }

    /// Moves by a screenful (at least a row), `direction` being `1` for down and `-1` for up.
    pub fn page(&mut self, direction: isize) {
        let page = self.viewport.get().saturating_sub(1).max(1) as isize;
        self.move_by(direction * page);
    }

    /// Switches between the size on disk and the apparent size, keeping the cursor on the same
    /// row.
    pub fn toggle_measure(&mut self) {
        self.measure = self.measure.toggled();
        let measure = self.measure;
        self.level_mut().sort(measure);
    }

    /// Scans the current folder again, for when things changed on disk.
    pub fn rescan(&mut self) {
        let keep = self.level().selected_path().cloned();
        self.start_scan();
        self.level_mut().reselect = keep;
    }

    /// Opens the selected folder, if it is one and can be looked into. Returns whether it did.
    pub fn enter(&mut self) -> bool {
        let Some(row) = self.selected_row() else {
            return false;
        };
        if row.kind != Kind::Dir {
            return false;
        }
        let path = row.path.clone();
        self.levels.push(Level::new(path));
        self.start_scan();
        true
    }

    /// Goes back up: to the level above if there is one on the stack, otherwise to the parent
    /// folder, with the cursor on the folder just left. Returns whether anything changed.
    pub fn leave(&mut self) -> bool {
        if self.levels.len() > 1 {
            self.levels.pop();
            // A level left before its scan finished has only part of its rows.
            if self.level().end.is_none() || self.level().end == Some(End::Cancelled) {
                self.rescan();
            } else {
                self.job = None;
            }
            return true;
        }
        let here = self.level().path.clone();
        let Some(parent) = here.parent() else {
            return false;
        };
        let mut level = Level::new(parent.to_path_buf());
        level.reselect = Some(here);
        self.levels = vec![level];
        self.start_scan();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::time::{Duration, Instant};

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-disk-usage-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, len: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; len]).unwrap();
    }

    fn scan(root: &Path) -> (Vec<Row>, End) {
        scan_with_limit(root, MAX_ENTRIES)
    }

    fn scan_with_limit(root: &Path, limit: u64) -> (Vec<Row>, End) {
        let mut rows = Vec::new();
        let end = scan_level(
            root,
            limit,
            &AtomicBool::new(false),
            &AtomicU64::new(0),
            &mut |batch| rows.extend(batch),
        );
        (rows, end)
    }

    fn by_name<'a>(rows: &'a [Row], name: &str) -> &'a Row {
        rows.iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("no row {name}"))
    }

    #[test]
    fn a_folders_size_is_everything_below_it_and_files_are_listed_on_their_own() {
        let root = scratch("basic");
        write(&root.join("top.txt"), 100);
        write(&root.join("dir/a"), 1000);
        write(&root.join("dir/deep/b"), 5000);
        write(&root.join("dir/deep/deeper/c"), 20);
        let (rows, end) = scan(&root);
        assert_eq!(end, End::Complete);
        assert_eq!(by_name(&rows, "top.txt").sizes.apparent, 100);
        assert_eq!(by_name(&rows, "top.txt").kind, Kind::File);
        let dir = by_name(&rows, "dir");
        assert_eq!(dir.kind, Kind::Dir);
        // The three files, plus the folders' own nodes (a few bytes to a few KiB each).
        assert!(dir.sizes.apparent >= 6020, "{dir:?}");
        assert_eq!(dir.items, 5, "a, deep, b, deeper, c");
        assert!(!dir.partial);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn disk_use_is_whole_blocks_and_apparent_size_is_the_length() {
        let root = scratch("blocks");
        write(&root.join("tiny"), 1);
        let (rows, _) = scan(&root);
        let tiny = by_name(&rows, "tiny");
        assert_eq!(tiny.sizes.apparent, 1);
        assert!(
            tiny.sizes.disk >= 512 && tiny.sizes.disk.is_multiple_of(512),
            "{tiny:?}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_with_two_links_is_counted_once_wherever_it_is_met() {
        let root = scratch("hardlinks");
        write(&root.join("a/original"), 3000);
        std::fs::create_dir_all(root.join("b")).unwrap();
        std::fs::hard_link(root.join("a/original"), root.join("b/second")).unwrap();
        let (rows, _) = scan(&root);
        let total: u64 = rows.iter().map(|row| row.sizes.apparent).sum();
        let one_copy = 3000;
        assert!(
            (one_copy..one_copy + 2 * 8192).contains(&total),
            "the file was counted twice: {total}"
        );
        // And within one folder, the second name adds nothing.
        std::fs::hard_link(root.join("a/original"), root.join("a/third")).unwrap();
        let (inner, _) = scan(&root.join("a"));
        let sizes: Vec<u64> = inner.iter().map(|row| row.sizes.apparent).collect();
        assert_eq!(sizes.iter().filter(|&&s| s == 3000).count(), 1, "{sizes:?}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_symlink_counts_as_the_link_and_a_loop_cannot_hang_the_scan() {
        let root = scratch("links");
        write(&root.join("big"), 9000);
        std::os::unix::fs::symlink(root.join("big"), root.join("to_file")).unwrap();
        std::os::unix::fs::symlink(&root, root.join("to_root")).unwrap();
        let (rows, end) = scan(&root);
        assert_eq!(end, End::Complete);
        let link = by_name(&rows, "to_file");
        assert_eq!(link.kind, Kind::Link);
        assert!(link.sizes.apparent < 9000, "counted the target: {link:?}");
        assert_eq!(by_name(&rows, "to_root").kind, Kind::Link);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_folder_on_another_filesystem_is_listed_but_never_entered() {
        let root = scratch("otherfs");
        write(&root.join("mount/data"), 4000);
        write(&root.join("plain"), 10);
        let seen = AtomicU64::new(0);
        let mut rows = Vec::new();
        // Pretending the root is on a device no folder below it is on.
        let end = scan_level_on(
            &root,
            u64::MAX,
            MAX_ENTRIES,
            &AtomicBool::new(false),
            &seen,
            &mut |b| rows.extend(b),
        );
        assert_eq!(end, End::Complete);
        let mount = by_name(&rows, "mount");
        assert_eq!(mount.kind, Kind::OtherFilesystem);
        assert_eq!(mount.sizes, Sizes::default());
        assert_eq!(
            seen.load(Ordering::Relaxed),
            2,
            "only the two entries of the root were looked at"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_folder_that_cannot_be_read_is_marked_partial_and_the_scan_goes_on() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("unreadable");
        write(&root.join("open/x"), 100);
        write(&root.join("closed/y"), 100);
        std::fs::set_permissions(root.join("closed"), std::fs::Permissions::from_mode(0o000))
            .unwrap();
        let (rows, end) = scan(&root);
        // Running as root can read anything, in which case there is nothing to mark.
        let readable = std::fs::read_dir(root.join("closed")).is_ok();
        std::fs::set_permissions(root.join("closed"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        assert_eq!(end, End::Complete);
        assert_eq!(by_name(&rows, "closed").partial, !readable);
        assert!(!by_name(&rows, "open").partial);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_or_unreadable_root_ends_as_unreadable() {
        let (rows, end) = scan(Path::new("/definitely/not/here"));
        assert!(rows.is_empty());
        assert_eq!(end, End::Unreadable);
    }

    #[test]
    fn the_entry_limit_stops_the_scan_and_says_the_answer_is_a_lower_bound() {
        let root = scratch("limit");
        for i in 0..20 {
            write(&root.join(format!("dir/f{i}")), 10);
        }
        let (_, end) = scan_with_limit(&root, 5);
        assert_eq!(end, End::Truncated);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_cancelled_scan_reports_it_and_stops_looking() {
        let root = scratch("cancel");
        write(&root.join("a"), 1);
        let seen = AtomicU64::new(0);
        let end = scan_level(
            &root,
            MAX_ENTRIES,
            &AtomicBool::new(true),
            &seen,
            &mut |_| {},
        );
        assert_eq!(end, End::Cancelled);
        assert_eq!(seen.load(Ordering::Relaxed), 0);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn more_files_than_the_row_limit_fold_into_one_row_keeping_the_total() {
        let mut files = Files::default();
        let count = 2 * MAX_ROWS + 500;
        let mut expected = 0u64;
        for i in 0..count {
            let size = (i % 977) as u64 + 1;
            expected += size;
            files.push(Row {
                name: format!("f{i}"),
                path: PathBuf::from(format!("/x/f{i}")),
                kind: Kind::File,
                sizes: Sizes {
                    disk: size,
                    apparent: size,
                },
                items: 1,
                partial: false,
            });
        }
        let (rows, rest, rest_items) = files.finish();
        assert_eq!(rows.len(), MAX_ROWS);
        assert_eq!(rows.len() as u64 + rest_items, count as u64);
        let kept: u64 = rows.iter().map(|r| r.sizes.apparent).sum();
        assert_eq!(
            kept + rest.apparent,
            expected,
            "folding must not lose or invent bytes"
        );
        // What was kept is the biggest: every one of the largest size is still there.
        let biggest = |size: u64| (0..count).filter(|i| (*i % 977) as u64 + 1 == size).count();
        assert_eq!(
            rows.iter().filter(|r| r.sizes.apparent == 977).count(),
            biggest(977)
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(20))]

        /// For any tree of files, what the scan reports for each folder is exactly the sum of
        /// the files' lengths in it (folders' own nodes are the only extra), nothing is counted
        /// twice, and the top level lists every entry once.
        #[test]
        fn folder_totals_are_the_sums_of_the_files_below(
            tree in proptest::collection::vec(
                (0usize..3, proptest::collection::vec(0usize..2, 0..3), 0usize..3000),
                0..14,
            ),
        ) {
            let root = scratch("model");
            let mut expected: std::collections::BTreeMap<String, u64> = Default::default();
            let mut top_names = std::collections::BTreeSet::new();
            for (n, (top, sub, size)) in tree.iter().enumerate() {
                let mut path = root.join(format!("d{top}"));
                for s in sub {
                    path = path.join(format!("s{s}"));
                }
                write(&path.join(format!("file{n}")), *size);
                *expected.entry(format!("d{top}")).or_default() += *size as u64;
                top_names.insert(format!("d{top}"));
            }
            let (rows, end) = scan(&root);
            prop_assert_eq!(end, End::Complete);
            prop_assert_eq!(rows.len(), top_names.len());
            for row in &rows {
                let files = expected[&row.name];
                // At most the folder nodes on top of the files: never fewer bytes, never a copy.
                prop_assert!(row.sizes.apparent >= files, "{:?} < {}", row, files);
                let folders = row.items - tree.iter().filter(|(t, _, _)| format!("d{t}") == row.name).count() as u64;
                prop_assert!(row.sizes.apparent <= files + (folders + 1) * 65_536, "{:?}", row);
                prop_assert!(row.sizes.disk.is_multiple_of(512));
            }
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    // ---- the view -------------------------------------------------------------------------

    fn row(name: &str, kind: Kind, disk: u64, apparent: u64) -> Row {
        Row {
            name: name.into(),
            path: PathBuf::from("/v").join(name),
            kind,
            sizes: Sizes { disk, apparent },
            items: 1,
            partial: false,
        }
    }

    fn wait(view: &mut DiskUsageView) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            view.poll();
            if view.end().is_some() {
                return;
            }
            assert!(Instant::now() < deadline, "the scan never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn rows_come_out_biggest_first_by_the_chosen_measure_and_ties_by_name() {
        let mut level = Level::new(PathBuf::from("/v"));
        level.rows = vec![
            row("b", Kind::File, 10, 900),
            row("a", Kind::File, 10, 5),
            row("big", Kind::File, 4096, 100),
            row("c", Kind::File, 10, 5),
        ];
        level.sort(Measure::OnDisk);
        let names: Vec<_> = level.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["big", "a", "b", "c"]);
        level.sort(Measure::Apparent);
        let names: Vec<_> = level.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["b", "big", "a", "c"]);
    }

    proptest! {
        /// Rows arriving and being re-sorted must not move the cursor off the row it is on.
        #[test]
        fn the_cursor_stays_on_its_row_as_rows_arrive(
            first in proptest::collection::vec(1u64..1000, 1..12),
            later in proptest::collection::vec(1u64..1000, 0..12),
            pick in 0usize..12,
        ) {
            let mut level = Level::new(PathBuf::from("/v"));
            level.rows = first.iter().enumerate().map(|(i, s)| row(&format!("a{i}"), Kind::File, *s, *s)).collect();
            level.sort(Measure::OnDisk);
            level.selected = pick.min(level.rows.len() - 1);
            level.moved = true;
            let chosen = level.selected_path().cloned();
            level.rows.extend(later.iter().enumerate().map(|(i, s)| row(&format!("z{i}"), Kind::File, *s, *s)));
            level.sort(Measure::OnDisk);
            prop_assert_eq!(level.selected_path().cloned(), chosen);
            for pair in level.rows.windows(2) {
                prop_assert!(pair[0].sizes.disk >= pair[1].sizes.disk);
            }
        }
    }

    #[test]
    fn until_the_user_moves_the_cursor_stays_on_the_biggest_row_as_bigger_ones_arrive() {
        let mut level = Level::new(PathBuf::from("/v"));
        level.rows = vec![row("first", Kind::File, 10, 10)];
        level.sort(Measure::OnDisk);
        level.rows.push(row("bigger", Kind::File, 999, 999));
        level.sort(Measure::OnDisk);
        assert_eq!(level.rows[level.selected].name, "bigger");
        level.rows.push(row("biggest", Kind::File, 5000, 5000));
        level.sort(Measure::OnDisk);
        assert_eq!(level.rows[level.selected].name, "biggest");
    }

    #[test]
    fn the_view_scans_lists_drills_down_and_comes_back_to_the_same_row() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("view");
        write(&root.join("small"), 10);
        write(&root.join("sub/big"), 500_000);
        write(&root.join("sub/other"), 100);
        let mut view = DiskUsageView::open(rt.handle().clone(), root.clone());
        wait(&mut view);
        assert_eq!(view.path(), root);
        assert_eq!(view.rows()[0].name, "sub", "the biggest is first");
        assert_eq!(view.end(), Some(End::Complete));
        assert!(view.total() >= 500_110);

        assert!(view.enter());
        wait(&mut view);
        assert_eq!(view.path(), root.join("sub"));
        assert_eq!(view.rows()[0].name, "big");

        assert!(!view.enter(), "a file cannot be opened");
        assert!(view.leave());
        assert_eq!(view.path(), root);
        assert_eq!(view.selected_row().map(|r| r.name.as_str()), Some("sub"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn leaving_the_top_level_goes_to_the_parent_with_the_cursor_on_where_it_came_from() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("parent");
        write(&root.join("aaa/x"), 10);
        write(&root.join("zzz/y"), 900_000);
        write(&root.join("mmm/z"), 40_000);
        let mut view = DiskUsageView::open(rt.handle().clone(), root.join("mmm"));
        wait(&mut view);
        assert!(view.leave());
        wait(&mut view);
        assert_eq!(view.path(), root);
        assert_eq!(view.selected_row().map(|r| r.name.as_str()), Some("mmm"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn switching_the_measure_re_sorts_and_keeps_the_cursor_on_the_same_row() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("measure");
        write(&root.join("aa"), 1);
        write(&root.join("bb"), 20_000);
        let mut view = DiskUsageView::open(rt.handle().clone(), root.clone());
        wait(&mut view);
        view.move_to(1);
        let chosen = view.selected_row().unwrap().name.clone();
        view.toggle_measure();
        assert_eq!(view.measure(), Measure::Apparent);
        assert_eq!(view.selected_row().unwrap().name, chosen);
        view.toggle_measure();
        assert_eq!(view.measure(), Measure::OnDisk);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn moving_is_clamped_and_a_page_is_a_screenful_less_one() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("move");
        for i in 0..30 {
            write(&root.join(format!("f{i:02}")), 10 + i);
        }
        let mut view = DiskUsageView::open(rt.handle().clone(), root.clone());
        wait(&mut view);
        view.move_by(-5);
        assert_eq!(view.selected(), 0);
        view.move_by(1000);
        assert_eq!(view.selected(), 29);
        view.move_to(0);
        view.viewport().set(11);
        view.page(1);
        assert_eq!(view.selected(), 10);
        view.page(-1);
        assert_eq!(view.selected(), 0);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn dropping_the_view_cancels_its_scan() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("drop");
        write(&root.join("a"), 1);
        let view = DiskUsageView::open(rt.handle().clone(), root.clone());
        let cancel = Arc::clone(&view.job.as_ref().unwrap().cancel);
        drop(view);
        assert!(cancel.load(Ordering::Relaxed));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Benchmark stub for the scan, the one path that touches every entry under a folder. It
    /// reports how fast a real tree (`MM_SCAN_ROOT`, default `/usr`) is walked; run it with
    /// `cargo test -p tui --release -- --ignored scan_throughput --nocapture`. It is ignored
    /// because the figure depends on the disk and the cache, and asserts only that the scan does
    /// not fall below a floor a laptop disk clears by a wide margin.
    #[test]
    #[ignore = "a timing check against a real tree; run in release mode on purpose"]
    fn scan_throughput_on_a_real_tree_clears_a_floor() {
        let root = std::env::var("MM_SCAN_ROOT").unwrap_or_else(|_| "/usr".into());
        let seen = AtomicU64::new(0);
        let started = Instant::now();
        let end = scan_level(
            Path::new(&root),
            MAX_ENTRIES,
            &AtomicBool::new(false),
            &seen,
            &mut |_| {},
        );
        let (entries, elapsed) = (seen.load(Ordering::Relaxed), started.elapsed());
        let per_second = entries as f64 / elapsed.as_secs_f64().max(1e-9);
        eprintln!(
            "scanned {entries} entries of {root} in {elapsed:?}: {per_second:.0} entries/s ({end:?})"
        );
        assert!(
            entries < 1000 || per_second > 20_000.0,
            "{per_second:.0} entries/s"
        );
    }
}
