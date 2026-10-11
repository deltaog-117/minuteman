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

//! The panel's rows: the entry's facts worded as a label and a value each, in display order.
//!
//! Owner, group, link count, permissions and disk usage are whatever the backend reported through
//! `stat`; a backend that has no such field leaves the row out rather than the panel guessing.
//! Owner and group arrive as numbers and are turned into names with this machine's `passwd` and
//! `group` only for a path on this machine's own disk, since another machine's ids mean nothing in
//! them.

use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use plugin_api::{InspectRow, StatKind, StatReply, TallyDone, TallyEnding};
use shared::format::{format_size, format_time, group_thousands, mode_string};

/// Where a folder's count stands, for the "Contents" row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contents {
    /// Not a folder, so there is nothing to count.
    NotApplicable,
    Counting,
    Counted(TallyDone),
    /// The count could not be started or the host went away before it answered.
    Unavailable,
}

impl Contents {
    fn label(&self) -> String {
        match self {
            Self::NotApplicable => "—".into(),
            Self::Counting => "counting…".into(),
            Self::Unavailable => "unavailable".into(),
            Self::Counted(done) => {
                let mut text = format!(
                    "{} files, {} folders, {}",
                    group_thousands(done.files),
                    group_thousands(done.dirs),
                    format_size(done.bytes)
                );
                match done.ending {
                    TallyEnding::Complete => {}
                    TallyEnding::Truncated => text.insert_str(0, "at least "),
                    TallyEnding::Cancelled => text.push_str(" (stopped)"),
                }
                if done.unreadable > 0 {
                    text.push_str(&format!(", {} unreadable", done.unreadable));
                }
                text
            }
        }
    }
}

/// The rows for the entry at `path`, whose facts are `stat`.
pub fn rows(path: &Path, stat: &StatReply, contents: &Contents) -> Vec<InspectRow> {
    let row = |label: &str, value: String| (label.to_string(), value);
    let mut rows = vec![
        row("Name", name_of(path)),
        row(
            "Location",
            path.parent()
                .map_or_else(|| "—".into(), |p| p.display().to_string()),
        ),
        row("Type", type_label(path, stat)),
    ];
    match stat.kind {
        StatKind::Dir => rows.push(row("Contents", contents.label())),
        StatKind::File | StatKind::Symlink | StatKind::Other => {
            rows.push(row("Size", size_label(stat.size)));
        }
    }
    if let Some(allocated) = stat.allocated {
        rows.push(row("On disk", size_label(allocated)));
    }
    if let Some(mode) = stat.mode {
        rows.push(row(
            "Permissions",
            format!("{} ({:04o})", mode_string(mode), mode & 0o7777),
        ));
    }
    if let (Some(uid), Some(gid)) = (stat.uid, stat.gid) {
        let local = stat.local_path.is_some();
        rows.push(row(
            "Owner",
            format!(
                "{}:{}",
                owner_name(local, "/etc/passwd", uid),
                owner_name(local, "/etc/group", gid)
            ),
        ));
    }
    if let Some(links) = stat.nlink {
        rows.push(row("Links", links.to_string()));
    }
    rows.push(row("Modified", time(stat.modified)));
    rows.push(row("Accessed", time(stat.accessed)));
    rows.push(row("Created", time(stat.created)));
    rows
}

/// The one row shown when the entry could not be described.
pub fn failure(path: &Path, reason: &str) -> Vec<InspectRow> {
    vec![
        ("Name".to_string(), name_of(path)),
        ("Error".to_string(), reason.to_string()),
    ]
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn time(seconds: Option<u64>) -> String {
    format_time(seconds.map(|s| UNIX_EPOCH + Duration::from_secs(s)))
}

/// "text", "image" or "binary or unknown" for a regular file.
fn content_label(path: &Path) -> &'static str {
    if shared::is_image(path) {
        "image"
    } else if shared::is_text(path) {
        "text"
    } else {
        "binary or unknown"
    }
}

fn type_label(path: &Path, stat: &StatReply) -> String {
    match (stat.kind, &stat.link_target) {
        (StatKind::Dir, _) => "Directory".into(),
        (StatKind::File, _) => format!("File ({})", content_label(path)),
        (StatKind::Symlink, Some(target)) => format!(
            "Symbolic link → {}{}",
            target.display(),
            if stat.link_broken { " (broken)" } else { "" }
        ),
        (StatKind::Symlink, None) => "Symbolic link".into(),
        (StatKind::Other, _) => "Special file".into(),
    }
}

/// `1.2K (1,234 bytes)`: the short form the lists use, then the exact count.
fn size_label(bytes: u64) -> String {
    format!("{} ({} bytes)", format_size(bytes), group_thousands(bytes))
}

/// The name for `id`: looked up in this machine's `passwd`- or `group`-format file when the path
/// is on this machine's disk, and the number itself when it is not, the file is unreadable, or it
/// has no such line (a container, a deleted user).
fn owner_name(local: bool, table: &str, id: u32) -> String {
    local
        .then(|| std::fs::read_to_string(table).ok())
        .flatten()
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn stat(kind: StatKind) -> StatReply {
        StatReply {
            kind,
            size: 5000,
            link_target: None,
            link_broken: false,
            allocated: None,
            mode: None,
            uid: None,
            gid: None,
            nlink: None,
            modified: None,
            accessed: None,
            created: None,
            local_path: None,
        }
    }

    fn value<'a>(rows: &'a [InspectRow], label: &str) -> Option<&'a str> {
        rows.iter()
            .find(|(l, _)| l == label)
            .map(|(_, v)| v.as_str())
    }

    fn done(files: u64, ending: TallyEnding, unreadable: u64) -> TallyDone {
        TallyDone {
            tally: 1,
            files,
            dirs: 2,
            bytes: 1536,
            unreadable,
            ending,
        }
    }

    #[test]
    fn a_file_shows_its_size_type_and_permissions() {
        let mut file = stat(StatKind::File);
        file.mode = Some(0o100_644);
        file.allocated = Some(8192);
        file.nlink = Some(1);
        let rows = rows(Path::new("/d/note.txt"), &file, &Contents::NotApplicable);
        assert_eq!(value(&rows, "Name"), Some("note.txt"));
        assert_eq!(value(&rows, "Location"), Some("/d"));
        assert_eq!(value(&rows, "Type"), Some("File (text)"));
        assert_eq!(value(&rows, "Size"), Some("4.9K (5,000 bytes)"));
        assert_eq!(value(&rows, "On disk"), Some("8.0K (8,192 bytes)"));
        assert_eq!(value(&rows, "Permissions"), Some("-rw-r--r-- (0644)"));
        assert_eq!(value(&rows, "Links"), Some("1"));
        assert_eq!(value(&rows, "Contents"), None, "only a folder has contents");
    }

    #[test]
    fn a_backend_without_owner_or_disk_fields_has_those_rows_left_out() {
        let rows = rows(
            Path::new("/d/f"),
            &stat(StatKind::File),
            &Contents::NotApplicable,
        );
        for label in ["On disk", "Permissions", "Owner", "Links"] {
            assert_eq!(value(&rows, label), None, "{label}");
        }
        assert_eq!(value(&rows, "Modified"), Some("—"));
    }

    #[test]
    fn another_machines_ids_stay_numbers_and_a_local_unknown_id_does_too() {
        let mut remote = stat(StatKind::File);
        remote.uid = Some(1000);
        remote.gid = Some(1000);
        let shown = rows(Path::new("/d/f"), &remote, &Contents::NotApplicable);
        assert_eq!(value(&shown, "Owner"), Some("1000:1000"));
        assert_eq!(owner_name(true, "/no/such/table", 42), "42");
    }

    #[test]
    fn a_name_is_found_by_its_numeric_id_and_a_missing_id_stays_a_number() {
        let table = "root:x:0:0:root:/root:/bin/sh\nda vi:x:1000:1000::/home/d:/bin/sh\n# odd\n";
        assert_eq!(name_in(table, 0).as_deref(), Some("root"));
        assert_eq!(name_in(table, 1000).as_deref(), Some("da vi"));
        assert_eq!(name_in(table, 7), None);
    }

    #[test]
    fn a_link_shows_where_it_points_and_whether_that_is_missing() {
        let mut link = stat(StatKind::Symlink);
        link.link_target = Some(PathBuf::from("/d/real"));
        let ok = rows(Path::new("/d/ok"), &link, &Contents::NotApplicable);
        assert_eq!(value(&ok, "Type"), Some("Symbolic link → /d/real"));
        link.link_broken = true;
        let broken = rows(Path::new("/d/ok"), &link, &Contents::NotApplicable);
        assert_eq!(
            value(&broken, "Type"),
            Some("Symbolic link → /d/real (broken)")
        );
    }

    #[test]
    fn a_folder_shows_its_count_as_it_stands() {
        let dir = stat(StatKind::Dir);
        let at = |contents: Contents| {
            value(&rows(Path::new("/d"), &dir, &contents), "Contents").map(str::to_string)
        };
        assert_eq!(at(Contents::Counting).as_deref(), Some("counting…"));
        assert_eq!(at(Contents::Unavailable).as_deref(), Some("unavailable"));
        assert_eq!(
            at(Contents::Counted(done(1000, TallyEnding::Complete, 0))).as_deref(),
            Some("1,000 files, 2 folders, 1.5K")
        );
        assert_eq!(
            at(Contents::Counted(done(3, TallyEnding::Truncated, 0))).as_deref(),
            Some("at least 3 files, 2 folders, 1.5K")
        );
        assert_eq!(
            at(Contents::Counted(done(3, TallyEnding::Cancelled, 2))).as_deref(),
            Some("3 files, 2 folders, 1.5K (stopped), 2 unreadable")
        );
        assert_eq!(
            value(&rows(Path::new("/d"), &dir, &Contents::Counting), "Size"),
            None
        );
    }

    #[test]
    fn a_failure_names_the_entry_and_the_reason() {
        let shown = failure(Path::new("/d/gone"), "No such file");
        assert_eq!(value(&shown, "Name"), Some("gone"));
        assert_eq!(value(&shown, "Error"), Some("No such file"));
    }
}
