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

//! The state of a disk usage view, shared by the interface's `u` view and the atlas plugin: a
//! stack of levels (the folders on the way down), each with its rows sorted biggest first and a
//! cursor, fed by a background scan of one folder at a time (`usage::scan_level`).
//!
//! The scan runs on a thread of its own and sends each finished row back over a channel; dropping
//! its job, or replacing it, sets its cancel flag and it stops. Only the folder being looked at is
//! scanned: opening a subfolder scans that one, and going back restores the level above from
//! memory, so memory holds the rows of the folders on the way down, never a whole tree.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use crate::Vfs;
use crate::usage::{End, Kind, MAX_ENTRIES, Measure, Row, scan_level};

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
    rx: Receiver<Message>,
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
    vfs: Arc<dyn Vfs>,
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
    pub fn open(vfs: Arc<dyn Vfs>, path: PathBuf) -> Self {
        let mut view = Self {
            vfs,
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
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_bg = Arc::clone(&cancel);
        let level = self.level_mut();
        level.end = None;
        level.rows.clear();
        level.selected = 0;
        level.moved = false;
        let (path, seen) = (level.path.clone(), Arc::clone(&level.seen));
        seen.store(0, Ordering::Relaxed);
        let vfs = Arc::clone(&self.vfs);
        // Replacing the job drops the old one, which cancels its scan.
        self.job = Some(Job { cancel, rx });
        std::thread::spawn(move || {
            let end = scan_level(
                vfs.as_ref(),
                &path,
                MAX_ENTRIES,
                &cancel_bg,
                &seen,
                &mut |rows| {
                    let _ = tx.send(Message::Rows(rows));
                },
            );
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
    use crate::LocalVfs;
    use crate::usage::Sizes;

    use super::*;
    use proptest::prelude::*;
    use std::path::Path;
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
        let root = scratch("view");
        write(&root.join("small"), 10);
        write(&root.join("sub/big"), 500_000);
        write(&root.join("sub/other"), 100);
        let mut view = DiskUsageView::open(Arc::new(LocalVfs), root.clone());
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
        let root = scratch("parent");
        write(&root.join("aaa/x"), 10);
        write(&root.join("zzz/y"), 900_000);
        write(&root.join("mmm/z"), 40_000);
        let mut view = DiskUsageView::open(Arc::new(LocalVfs), root.join("mmm"));
        wait(&mut view);
        assert!(view.leave());
        wait(&mut view);
        assert_eq!(view.path(), root);
        assert_eq!(view.selected_row().map(|r| r.name.as_str()), Some("mmm"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn switching_the_measure_re_sorts_and_keeps_the_cursor_on_the_same_row() {
        let root = scratch("measure");
        write(&root.join("aa"), 1);
        write(&root.join("bb"), 20_000);
        let mut view = DiskUsageView::open(Arc::new(LocalVfs), root.clone());
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
        let root = scratch("move");
        for i in 0..30 {
            write(&root.join(format!("f{i:02}")), 10 + i);
        }
        let mut view = DiskUsageView::open(Arc::new(LocalVfs), root.clone());
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
        let root = scratch("drop");
        write(&root.join("a"), 1);
        let view = DiskUsageView::open(Arc::new(LocalVfs), root.clone());
        let cancel = Arc::clone(&view.job.as_ref().unwrap().cancel);
        drop(view);
        assert!(cancel.load(Ordering::Relaxed));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_view_runs_on_another_backend() {
        let mem = crate::MemVfs::new();
        mem.add_file("/v/small", vec![0u8; 10]);
        mem.add_file("/v/sub/big", vec![0u8; 500_000]);
        let mut view = DiskUsageView::open(Arc::new(mem), PathBuf::from("/v"));
        wait(&mut view);

        assert_eq!(view.rows()[0].name, "sub", "the biggest is first");
        assert!(view.enter());
        wait(&mut view);
        assert_eq!(view.rows()[0].name, "big");
    }
}
