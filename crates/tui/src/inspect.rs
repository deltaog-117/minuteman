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

//! The "Inspect" panel's data: everything a file manager's properties dialog shows for one file,
//! folder or link. `Inspection` is a snapshot of one `lstat` (cheap, taken at once);
//! a folder's contents — how many files and how much they add up to — need a walk that can take
//! seconds, so `InspectView` runs it on the blocking pool the way `search_job` does and shows
//! "counting…" until it lands. Dropping the view cancels the walk.
//!
//! Owner, group, link count and disk usage come straight from `std::fs`, so they exist only for
//! local paths; `Vfs` has no such fields and the panel says so rather than guessing.
//!
//! Everything that turns numbers into text lives here as plain functions (`mode_string`,
//! `format_utc`, `group_thousands`) so it can be tested without a terminal.

use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::hud::format_size;

/// A folder with more entries than this is reported as "at least" rather than walked to the end,
/// so inspecting `/` cannot keep the blocking pool busy for minutes.
const TALLY_LIMIT: u64 = 500_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
    Symlink,
    Other,
}

/// One entry's metadata as read at the moment it was inspected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub path: PathBuf,
    pub name: String,
    pub kind: Kind,
    /// Where a symlink points, as written in the link.
    pub link_target: Option<PathBuf>,
    /// Whether that target is missing.
    pub link_broken: bool,
    /// The entry's own size in bytes (for a folder, the directory node — not its contents).
    pub size: u64,
    pub disk_bytes: u64,
    pub mode: u32,
    pub owner: String,
    pub group: String,
    pub hard_links: u64,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub created: Option<SystemTime>,
    /// "text", "image" or "binary or unknown" for a regular file.
    pub content: Option<&'static str>,
}

impl Inspection {
    /// Reads `path`'s metadata without following a final symlink, so a link is inspected as a
    /// link.
    ///
    /// # Errors
    ///
    /// Returns the I/O error from `lstat` (the path is gone, or not readable).
    pub fn of(path: &Path) -> io::Result<Self> {
        let meta = std::fs::symlink_metadata(path)?;
        let file_type = meta.file_type();
        let kind = if file_type.is_symlink() {
            Kind::Symlink
        } else if file_type.is_dir() {
            Kind::Directory
        } else if file_type.is_file() {
            Kind::File
        } else {
            Kind::Other
        };
        let link_target = (kind == Kind::Symlink)
            .then(|| std::fs::read_link(path).ok())
            .flatten();
        let content = (kind == Kind::File).then(|| {
            if preview::is_image(path) {
                "image"
            } else if preview::is_text(path) {
                "text"
            } else {
                "binary or unknown"
            }
        });
        Ok(Self {
            path: path.to_path_buf(),
            name: path.file_name().map_or_else(
                || path.to_string_lossy().into_owned(),
                |name| name.to_string_lossy().into_owned(),
            ),
            kind,
            link_broken: kind == Kind::Symlink && !path.exists(),
            link_target,
            size: meta.len(),
            // `st_blocks` counts 512-byte units whatever the filesystem's block size is.
            disk_bytes: meta.blocks() * 512,
            mode: meta.mode(),
            owner: name_for_id("/etc/passwd", meta.uid()),
            group: name_for_id("/etc/group", meta.gid()),
            hard_links: meta.nlink(),
            modified: meta.modified().ok(),
            accessed: meta.accessed().ok(),
            created: meta.created().ok(),
            content,
        })
    }

    /// The panel's rows, label then value, in display order. `contents` is the folder walk's
    /// state and only matters for a folder.
    pub fn rows(&self, contents: &Contents) -> Vec<(&'static str, String)> {
        let mut rows = vec![
            ("Name", self.name.clone()),
            (
                "Location",
                self.path
                    .parent()
                    .map_or_else(|| "—".into(), |p| p.display().to_string()),
            ),
            ("Type", self.type_label()),
        ];
        match self.kind {
            Kind::Directory => rows.push(("Contents", contents.label())),
            Kind::File | Kind::Symlink | Kind::Other => rows.push(("Size", size_label(self.size))),
        }
        rows.push(("On disk", size_label(self.disk_bytes)));
        rows.push((
            "Permissions",
            format!("{} ({:04o})", mode_string(self.mode), self.mode & 0o7777),
        ));
        rows.push(("Owner", format!("{}:{}", self.owner, self.group)));
        rows.push(("Links", self.hard_links.to_string()));
        rows.push(("Modified", format_utc(self.modified)));
        rows.push(("Accessed", format_utc(self.accessed)));
        rows.push(("Created", format_utc(self.created)));
        rows
    }

    fn type_label(&self) -> String {
        match (self.kind, &self.link_target) {
            (Kind::Directory, _) => "Directory".into(),
            (Kind::File, _) => format!("File ({})", self.content.unwrap_or("unknown")),
            (Kind::Symlink, Some(target)) => format!(
                "Symbolic link → {}{}",
                target.display(),
                if self.link_broken { " (broken)" } else { "" }
            ),
            (Kind::Symlink, None) => "Symbolic link".into(),
            (Kind::Other, _) => "Special file".into(),
        }
    }
}

/// `1.2K (1,234 bytes)`: the short form the lists use, then the exact count.
fn size_label(bytes: u64) -> String {
    format!("{} ({} bytes)", format_size(bytes), group_thousands(bytes))
}

/// `1234567` as `1,234,567`.
pub fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// `ls -l`'s ten-character mode: type letter, then rwx for user, group and other, with the
/// setuid, setgid and sticky bits shown in the execute slots (lower case when that execute bit
/// is also set, upper case when it is not).
pub fn mode_string(mode: u32) -> String {
    let kind = match mode & 0o170_000 {
        0o040_000 => 'd',
        0o120_000 => 'l',
        0o100_000 => '-',
        0o060_000 => 'b',
        0o020_000 => 'c',
        0o010_000 => 'p',
        0o140_000 => 's',
        _ => '?',
    };
    let mut out = String::with_capacity(10);
    out.push(kind);
    // (read, write, execute, the special bit that replaces execute, its two letters)
    let classes = [
        (0o400, 0o200, 0o100, 0o4000, ('s', 'S')),
        (0o040, 0o020, 0o010, 0o2000, ('s', 'S')),
        (0o004, 0o002, 0o001, 0o1000, ('t', 'T')),
    ];
    for (read, write, exec, special, (with_exec, without_exec)) in classes {
        out.push(if mode & read != 0 { 'r' } else { '-' });
        out.push(if mode & write != 0 { 'w' } else { '-' });
        out.push(match (mode & special != 0, mode & exec != 0) {
            (true, true) => with_exec,
            (true, false) => without_exec,
            (false, true) => 'x',
            (false, false) => '-',
        });
    }
    out
}

/// `2023-11-14 22:13:20 UTC`, or `—` for a missing or pre-1970 time. UTC because the standard
/// library has no time zone database and a wrong local time would be worse than a labelled one.
pub fn format_utc(time: Option<SystemTime>) -> String {
    let Some(secs) = time
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
    else {
        return "—".into();
    };
    let (year, month, day) = civil_from_days((secs / 86_400) as i64);
    let rest = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian `(year, month, day)` — Howard Hinnant's
/// `civil_from_days`, which needs no tables and is exact across leap years and centuries.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

/// The name for `id` in a `passwd`- or `group`-format file, or the number itself when the file
/// is unreadable or has no such line (a container, a deleted user).
fn name_for_id(table: &str, id: u32) -> String {
    std::fs::read_to_string(table)
        .ok()
        .and_then(|text| name_in(&text, id))
        .unwrap_or_else(|| id.to_string())
}

/// The first field of the line whose third field is `id`.
fn name_in(table: &str, id: u32) -> Option<String> {
    table.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let line_id = fields.nth(1)?;
        (line_id.parse::<u32>().ok()? == id).then(|| name.to_owned())
    })
}

/// What a walk of a folder found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub files: u64,
    pub dirs: u64,
    /// The files' own sizes added up (a symlink counts as the link, not its target).
    pub bytes: u64,
    /// Folders inside that could not be read.
    pub unreadable: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    Complete,
    /// Stopped at the entry limit; the numbers are a lower bound.
    Truncated,
    Cancelled,
}

/// Walks `root` counting what is below it, without following symlinks (so a link loop cannot
/// hang it) and stopping early once `limit` entries were seen or `cancel` is set.
pub fn tally_dir(root: &Path, limit: u64, cancel: &AtomicBool) -> (Tally, Ending) {
    let mut tally = Tally::default();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            tally.unreadable += 1;
            continue;
        };
        for entry in entries.flatten() {
            if cancel.load(Ordering::Relaxed) {
                return (tally, Ending::Cancelled);
            }
            if tally.files + tally.dirs >= limit {
                return (tally, Ending::Truncated);
            }
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => {
                    tally.dirs += 1;
                    pending.push(entry.path());
                }
                Ok(_) => {
                    tally.files += 1;
                    tally.bytes += entry.metadata().map_or(0, |m| m.len());
                }
                Err(_) => tally.unreadable += 1,
            }
        }
    }
    (tally, Ending::Complete)
}

/// Where a folder's walk stands, for the "Contents" row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contents {
    /// Not a folder, so there is nothing to count.
    NotApplicable,
    Counting,
    Counted(Tally, Ending),
}

impl Contents {
    fn label(&self) -> String {
        match self {
            Contents::NotApplicable => "—".into(),
            Contents::Counting => "counting…".into(),
            Contents::Counted(tally, ending) => {
                let mut text = format!(
                    "{} files, {} folders, {}",
                    group_thousands(tally.files),
                    group_thousands(tally.dirs),
                    format_size(tally.bytes)
                );
                match ending {
                    Ending::Complete => {}
                    Ending::Truncated => text.insert_str(0, "at least "),
                    Ending::Cancelled => text.push_str(" (stopped)"),
                }
                if tally.unreadable > 0 {
                    text.push_str(&format!(", {} unreadable", tally.unreadable));
                }
                text
            }
        }
    }
}

struct TallyJob {
    cancel: Arc<AtomicBool>,
    rx: UnboundedReceiver<(Tally, Ending)>,
}

impl Drop for TallyJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// An open Inspect panel: the snapshot plus the folder walk feeding its "Contents" row.
pub struct InspectView {
    info: Inspection,
    contents: Contents,
    job: Option<TallyJob>,
}

impl InspectView {
    /// Inspects `path` and, for a folder, starts counting its contents on `handle`'s blocking
    /// pool.
    ///
    /// # Errors
    ///
    /// Returns the `lstat` failure, worded for the status line.
    pub fn open(handle: &tokio::runtime::Handle, path: &Path) -> Result<Self, String> {
        let info =
            Inspection::of(path).map_err(|e| format!("cannot inspect {}: {e}", path.display()))?;
        if info.kind != Kind::Directory {
            return Ok(Self {
                info,
                contents: Contents::NotApplicable,
                job: None,
            });
        }
        let (tx, rx) = unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (root, cancel_bg) = (path.to_path_buf(), Arc::clone(&cancel));
        handle.spawn_blocking(move || {
            let _ = tx.send(tally_dir(&root, TALLY_LIMIT, &cancel_bg));
        });
        Ok(Self {
            info,
            contents: Contents::Counting,
            job: Some(TallyJob { cancel, rx }),
        })
    }

    /// Picks up the walk's answer if it has arrived. A walk whose thread died without answering
    /// reads as stopped, so the row stops claiming to count.
    pub fn poll(&mut self) {
        let Some(job) = self.job.as_mut() else { return };
        let answer = match job.rx.try_recv() {
            Ok((tally, ending)) => Contents::Counted(tally, ending),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Contents::Counted(Tally::default(), Ending::Cancelled)
            }
        };
        self.contents = answer;
        self.job = None;
    }

    pub fn title(&self) -> &str {
        &self.info.name
    }

    pub fn rows(&self) -> Vec<(&'static str, String)> {
        self.info.rows(&self.contents)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("minuteman-inspect-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn at(secs: u64) -> Option<SystemTime> {
        Some(UNIX_EPOCH + std::time::Duration::from_secs(secs))
    }

    #[test]
    fn modes_read_like_ls() {
        assert_eq!(mode_string(0o040_755), "drwxr-xr-x");
        assert_eq!(mode_string(0o100_644), "-rw-r--r--");
        assert_eq!(mode_string(0o120_777), "lrwxrwxrwx");
        assert_eq!(mode_string(0o104_755), "-rwsr-xr-x");
        assert_eq!(mode_string(0o104_644), "-rwSr--r--");
        assert_eq!(mode_string(0o041_777), "drwxrwxrwt");
        assert_eq!(mode_string(0o041_776), "drwxrwxrwT");
        assert_eq!(mode_string(0o102_755), "-rwxr-sr-x");
    }

    #[test]
    fn times_are_exact_across_leap_days_and_the_epoch() {
        assert_eq!(format_utc(at(0)), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_utc(at(951_782_400)), "2000-02-29 00:00:00 UTC");
        assert_eq!(format_utc(at(1_700_000_000)), "2023-11-14 22:13:20 UTC");
        assert_eq!(format_utc(at(4_102_444_799)), "2099-12-31 23:59:59 UTC");
        assert_eq!(format_utc(None), "—");
    }

    #[test]
    fn digits_are_grouped_in_threes() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1,000");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn a_name_is_found_by_its_numeric_id_and_a_missing_id_stays_a_number() {
        let table = "root:x:0:0:root:/root:/bin/sh\nda vi:x:1000:1000::/home/d:/bin/sh\n# odd\n";
        assert_eq!(name_in(table, 0).as_deref(), Some("root"));
        assert_eq!(name_in(table, 1000).as_deref(), Some("da vi"));
        assert_eq!(name_in(table, 7), None);
        assert_eq!(name_for_id("/no/such/table", 42), "42");
    }

    #[test]
    fn a_folder_is_counted_with_its_files_folders_and_bytes() {
        let root = scratch("tally");
        std::fs::create_dir_all(root.join("a").join("b")).unwrap();
        std::fs::write(root.join("one"), [0u8; 10]).unwrap();
        std::fs::write(root.join("a").join("two"), [0u8; 20]).unwrap();
        std::fs::write(root.join("a").join("b").join("three"), [0u8; 5]).unwrap();

        let (tally, ending) = tally_dir(&root, 100, &AtomicBool::new(false));
        assert_eq!(ending, Ending::Complete);
        assert_eq!(
            tally,
            Tally {
                files: 3,
                dirs: 2,
                bytes: 35,
                unreadable: 0
            }
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_walk_stops_at_its_limit_and_when_cancelled() {
        let root = scratch("limits");
        for i in 0..6 {
            std::fs::write(root.join(format!("f{i}")), b"").unwrap();
        }
        let (tally, ending) = tally_dir(&root, 3, &AtomicBool::new(false));
        assert_eq!((tally.files, ending), (3, Ending::Truncated));
        let (tally, ending) = tally_dir(&root, 100, &AtomicBool::new(true));
        assert_eq!((tally.files, ending), (0, Ending::Cancelled));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_symlink_loop_is_counted_once_not_followed() {
        let root = scratch("loop");
        std::os::unix::fs::symlink(&root, root.join("again")).unwrap();
        let (tally, ending) = tally_dir(&root, 100, &AtomicBool::new(false));
        assert_eq!((tally.files, tally.dirs, ending), (1, 0, Ending::Complete));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_reports_its_size_type_and_permissions() {
        let root = scratch("file");
        let file = root.join("note.txt");
        std::fs::write(&file, b"hello").unwrap();
        let info = Inspection::of(&file).unwrap();
        assert_eq!(
            (info.kind, info.size, info.name.as_str()),
            (Kind::File, 5, "note.txt")
        );
        let rows = info.rows(&Contents::NotApplicable);
        let value = |label: &str| rows.iter().find(|(l, _)| *l == label).unwrap().1.clone();
        assert_eq!(value("Type"), "File (text)");
        assert_eq!(value("Size"), "5B (5 bytes)");
        assert!(value("Permissions").starts_with("-rw"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_broken_link_is_reported_as_a_link_and_as_broken() {
        let root = scratch("broken");
        let link = root.join("dangling");
        std::os::unix::fs::symlink(root.join("missing"), &link).unwrap();
        let info = Inspection::of(&link).unwrap();
        assert_eq!(info.kind, Kind::Symlink);
        assert!(info.link_broken);
        assert!(info.type_label().ends_with("(broken)"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_path_that_is_gone_is_an_error_not_a_panic() {
        assert!(Inspection::of(Path::new("/no/such/minuteman/path")).is_err());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        assert!(
            InspectView::open(runtime.handle(), Path::new("/no/such/minuteman/path"))
                .is_err_and(|message| message.starts_with("cannot inspect"))
        );
    }

    #[test]
    fn a_folder_view_counts_in_the_background_and_then_shows_the_totals() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("view");
        std::fs::write(root.join("x"), [0u8; 3]).unwrap();
        let mut view = InspectView::open(runtime.handle(), &root).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while view.contents == Contents::Counting {
            assert!(
                std::time::Instant::now() < deadline,
                "the count never finished"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
            view.poll();
        }
        let rows = view.rows();
        let contents = &rows.iter().find(|(l, _)| *l == "Contents").unwrap().1;
        assert_eq!(contents, "1 files, 0 folders, 3B");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn dropping_a_folder_view_cancels_its_walk() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = scratch("drop");
        let view = InspectView::open(runtime.handle(), &root).unwrap();
        let flag = Arc::clone(&view.job.as_ref().unwrap().cancel);
        assert!(!flag.load(Ordering::Relaxed));
        drop(view);
        assert!(flag.load(Ordering::Relaxed));
        std::fs::remove_dir_all(&root).unwrap();
    }

    proptest! {
        #[test]
        fn a_mode_is_always_ten_characters(mode in any::<u32>()) {
            prop_assert_eq!(mode_string(mode).chars().count(), 10);
        }

        #[test]
        fn later_times_never_sort_before_earlier_ones(a in 0u64..4_000_000_000, b in 0u64..4_000_000_000) {
            let (early, late) = (a.min(b), a.max(b));
            prop_assert!(format_utc(at(early)) <= format_utc(at(late)));
        }

        #[test]
        fn grouping_only_adds_commas(n in any::<u64>()) {
            prop_assert_eq!(group_thousands(n).replace(',', ""), n.to_string());
        }
    }
}
