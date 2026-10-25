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

//! The undo history as text, so it can be kept between sessions.
//!
//! One line per change, fields separated by a tab (`<tab>` below), with `\`, tab, newline and carriage return escaped inside a
//! field; an entry starts with a `#` line naming it (see `EntryId`), ends with a `-` line, and
//! `@ undo` / `@ redo` say which stack the entries after it belong to (oldest first). The first line
//! names the format, so a later one can be told apart. Format 1, which has no `#` lines, is still
//! read; an entry without one is named by its place in the file.
//!
//! ```text
//! mman-history 2
//! @ undo
//! # 19c2f3a1b2c-1f4d-0
//! M<tab>/home/a/old.txt<tab>/home/a/new.txt
//! T<tab>/home/a/gone.txt<tab>/home/a/.local/share/Trash/info/gone.txt.trashinfo<tab>1760000000
//! -
//! @ redo
//! # 19c2f3a1b2d-1f4d-1
//! P<tab>/home/a/out.zip<tab>a
//! -
//! ```
//!
//! Only what can still be trusted is brought back. A trash handle is the trash's own name for an
//! item, so it is kept only when the item is still in the trash under that name and time; an entry
//! that names one that is gone is dropped whole, as is one with a line that does not parse. A path
//! that is not absolute (another machine's) or not UTF-8 is not written at all, so a restored
//! change never runs against a folder the next session happens to start in.

use std::collections::HashMap;
use std::path::PathBuf;

use trash::{TrashEntry, Trashed};

use super::{Change, Direction, Entry, EntryId, History, Product};

const HEADER_NAME: &str = "mman-history";
pub(super) const VERSION: &str = "2";
/// Formats this version reads: the current one and the one before entries had ids.
const READABLE: [&str; 2] = ["1", VERSION];

/// Why a saved history could not be read at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("it is not a saved undo history")]
    NotHistory,
    #[error("it was saved in format {0}, which this version does not read")]
    Version(String),
}

/// What `History::decode` made of a saved history.
#[derive(Debug)]
pub struct Restored {
    pub history: History,
    /// Entries left out because they could not be trusted any more (see the module docs).
    pub dropped: usize,
    /// The entries the file holds that `history` does not: the dropped ones and the oldest ones
    /// past the capacity. A `Merger` given these does not write them back.
    pub forgotten: Vec<EntryId>,
}

/// One entry of a saved history as it stands in the text: named, and with its lines unread.
pub(super) struct Block<'a> {
    pub id: EntryId,
    pub lines: Vec<&'a str>,
}

/// The entries of a saved history, sorted into the stack they were listed under.
pub(super) struct Blocks<'a> {
    pub undo: Vec<Block<'a>>,
    pub redo: Vec<Block<'a>>,
    /// Entries that could not be sorted into either: no `-`, or under no known `@` line.
    pub stray: usize,
}

/// Splits a saved history into entries without reading what is in them, so the same pass serves
/// `decode` and the merge of two files.
///
/// # Errors
///
/// The text does not start with a first line of a format this version reads.
pub(super) fn parse_blocks(text: &str) -> Result<Blocks<'_>, DecodeError> {
    let mut lines = text.lines();
    match lines.next().and_then(|l| l.strip_prefix(HEADER_NAME)) {
        Some(rest)
            if rest
                .strip_prefix(' ')
                .is_some_and(|v| READABLE.contains(&v)) => {}
        Some(rest) if rest.starts_with(' ') => {
            return Err(DecodeError::Version(rest.trim().to_owned()));
        }
        _ => return Err(DecodeError::NotHistory),
    }

    let mut blocks = Blocks {
        undo: Vec::new(),
        redo: Vec::new(),
        stray: 0,
    };
    let mut side = None;
    let mut open: Option<(Option<EntryId>, Vec<&str>)> = None;
    // Counts every entry that ends, so an unnamed one gets the same name in every session.
    let mut seen = 0;
    for line in lines.filter(|line| !line.is_empty()) {
        if let Some(name) = line.strip_prefix("@ ") {
            blocks.stray += usize::from(open.take().is_some());
            side = match name {
                "undo" => Some(Direction::Undo),
                "redo" => Some(Direction::Redo),
                _ => None,
            };
        } else if line == "-" {
            let (id, lines) = open.take().unwrap_or_default();
            let id = id.unwrap_or_else(|| EntryId::legacy(seen));
            seen += 1;
            match side {
                Some(Direction::Undo) => blocks.undo.push(Block { id, lines }),
                Some(Direction::Redo) => blocks.redo.push(Block { id, lines }),
                None => blocks.stray += 1,
            }
        } else {
            let (id, lines) = open.get_or_insert_with(|| (None, Vec::new()));
            match line.strip_prefix("# ").map(str::parse) {
                Some(Ok(named)) if id.is_none() && lines.is_empty() => *id = Some(named),
                _ => lines.push(line),
            }
        }
    }
    blocks.stray += usize::from(open.is_some());
    Ok(blocks)
}

impl History {
    /// The history as text for `decode`. A batch still being recorded is written as the entry it
    /// will become, and then no redo is written, since committing it forgets them. An entry that
    /// cannot be written (see the module docs) is left out.
    pub fn encode(&self) -> String {
        let mut out = format!("{HEADER_NAME} {VERSION}\n");
        let pending = self
            .pending_id
            .filter(|_| !self.pending.is_empty())
            .map(|id| Entry {
                id,
                changes: self.pending.clone(),
            });
        out.push_str("@ undo\n");
        for entry in self.undo.iter().chain(pending.as_ref()) {
            write_entry(&mut out, entry, Direction::Undo);
        }
        if pending.is_none() {
            out.push_str("@ redo\n");
            for entry in &self.redo {
                write_entry(&mut out, entry, Direction::Redo);
            }
        }
        out
    }

    /// Reads what `encode` wrote. `live` is what the trash holds now, listed once by the caller
    /// (`trash::entries`); `capacity` keeps the newest entries when the file holds more.
    ///
    /// # Errors
    ///
    /// The text does not start with a first line of a format this version reads.
    pub fn decode(
        text: &str,
        capacity: usize,
        live: &[TrashEntry],
    ) -> Result<Restored, DecodeError> {
        let blocks = parse_blocks(text)?;
        let held: HashMap<(&str, i64), &TrashEntry> = live
            .iter()
            .map(|entry| ((entry.id.as_str(), entry.deleted), entry))
            .collect();

        let mut history = History::with_capacity(capacity);
        let mut dropped = blocks.stray;
        let mut forgotten = Vec::new();
        for (stack, side, from) in [
            (&mut history.undo, Direction::Undo, blocks.undo),
            (&mut history.redo, Direction::Redo, blocks.redo),
        ] {
            let mut entries = Vec::new();
            for block in from {
                match block_changes(&block, side, &held) {
                    Some(changes) => entries.push(Entry {
                        id: block.id,
                        changes,
                    }),
                    None => {
                        dropped += 1;
                        forgotten.push(block.id);
                    }
                }
            }
            let excess = entries.len().saturating_sub(history.capacity);
            forgotten.extend(entries.iter().take(excess).map(|entry| entry.id));
            stack.extend(entries.into_iter().skip(excess));
        }
        Ok(Restored {
            history,
            dropped,
            forgotten,
        })
    }
}

/// Every line of `block` as a change, or `None` when one does not read or there are none.
fn block_changes(
    block: &Block<'_>,
    side: Direction,
    held: &HashMap<(&str, i64), &TrashEntry>,
) -> Option<Vec<Change>> {
    let changes: Vec<Change> = block
        .lines
        .iter()
        .map(|line| parse_change(line, side, held))
        .collect::<Option<_>>()?;
    (!changes.is_empty()).then_some(changes)
}

fn write_entry(out: &mut String, entry: &Entry, side: Direction) {
    let lines: Option<Vec<String>> = entry
        .changes
        .iter()
        .map(|change| change_line(change, side))
        .collect();
    if let Some(lines) = lines {
        out.push_str(&format!("# {}\n", entry.id));
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        out.push_str("-\n");
    }
}

/// An absolute UTF-8 path as an escaped field, or `None` for any other.
fn path_field(path: &std::path::Path) -> Option<String> {
    path.is_absolute().then_some(())?;
    Some(escape(path.to_str()?))
}

fn change_line(change: &Change, side: Direction) -> Option<String> {
    let fields: Vec<String> = match change {
        Change::Created { path, is_dir } => {
            vec![
                "C".into(),
                path_field(path)?,
                if *is_dir { "d" } else { "f" }.into(),
            ]
        }
        Change::Moved { from, to } => vec!["M".into(), path_field(from)?, path_field(to)?],
        Change::Copied { src, dst } => vec!["Y".into(), path_field(src)?, path_field(dst)?],
        Change::Trashed { path, item } => {
            let mut fields = vec!["T".into(), path_field(path)?];
            // After an undo the handle is spent: redoing trashes the path again and takes a new one.
            if side == Direction::Undo {
                fields.extend(handle_fields(item)?);
            }
            fields
        }
        Change::Produced {
            path,
            product,
            held,
        } => {
            let mut fields = vec![
                "P".into(),
                path_field(path)?,
                match product {
                    Product::Archive => "a",
                    Product::Folder => "f",
                }
                .into(),
            ];
            if let Some(item) = held {
                fields.extend(handle_fields(item)?);
            }
            fields
        }
    };
    Some(fields.join("\t"))
}

fn handle_fields(item: &Trashed) -> Option<[String; 2]> {
    Some([escape(item.id()?), item.deleted().to_string()])
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(field: &str) -> Option<String> {
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next()? {
            '\\' => '\\',
            't' => '\t',
            'n' => '\n',
            'r' => '\r',
            _ => return None,
        });
    }
    Some(out)
}

fn parse_change(
    line: &str,
    side: Direction,
    held: &HashMap<(&str, i64), &TrashEntry>,
) -> Option<Change> {
    let fields: Vec<String> = line.split('\t').map(unescape).collect::<Option<_>>()?;
    let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
    let path = |text: &str| Some(PathBuf::from(text)).filter(|p| p.is_absolute());
    let handle = |id: &str, deleted: &str| {
        held.get(&(id, deleted.parse().ok()?))
            .map(|entry| Trashed::from(*entry))
    };
    Some(match (fields.as_slice(), side) {
        (["C", at, kind], _) => Change::Created {
            path: path(at)?,
            is_dir: flag(kind, "d", "f")?,
        },
        (["M", from, to], _) => Change::Moved {
            from: path(from)?,
            to: path(to)?,
        },
        (["Y", src, dst], _) => Change::Copied {
            src: path(src)?,
            dst: path(dst)?,
        },
        (["T", at], Direction::Redo) => {
            let at = path(at)?;
            Change::Trashed {
                item: Trashed::spent(&at)?,
                path: at,
            }
        }
        (["T", at, id, deleted], Direction::Undo) => Change::Trashed {
            path: path(at)?,
            item: handle(id, deleted)?,
        },
        (["P", at, kind], _) => Change::Produced {
            path: path(at)?,
            product: product(kind)?,
            held: None,
        },
        (["P", at, kind, id, deleted], _) => Change::Produced {
            path: path(at)?,
            product: product(kind)?,
            held: Some(handle(id, deleted)?),
        },
        _ => return None,
    })
}

fn flag(text: &str, yes: &str, no: &str) -> Option<bool> {
    match text {
        t if t == yes => Some(true),
        t if t == no => Some(false),
        _ => None,
    }
}

fn product(text: &str) -> Option<Product> {
    flag(text, "a", "f").map(|archive| {
        if archive {
            Product::Archive
        } else {
            Product::Folder
        }
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use proptest::prelude::*;

    use super::*;

    fn item(id: &str, at: &str, deleted: i64) -> TrashEntry {
        let at = Path::new(at);
        TrashEntry {
            id: id.into(),
            name: at.file_name().unwrap().to_string_lossy().into_owned(),
            original_dir: at.parent().unwrap().to_path_buf(),
            deleted,
            size: trash::EntrySize::Unknown,
            stored_path: None,
        }
    }

    fn entry(changes: Vec<Change>) -> Entry {
        Entry {
            id: EntryId::fresh(),
            changes,
        }
    }

    fn changes_of(entries: &[Entry]) -> Vec<Vec<Change>> {
        entries.iter().map(|e| e.changes.clone()).collect()
    }

    fn moved(from: &str, to: &str) -> Change {
        Change::Moved {
            from: from.into(),
            to: to.into(),
        }
    }

    fn history(undo: Vec<Entry>, redo: Vec<Entry>) -> History {
        History {
            undo,
            redo,
            ..History::default()
        }
    }

    fn restore(history: &History, live: &[TrashEntry]) -> Restored {
        History::decode(&history.encode(), 100, live).unwrap()
    }

    #[test]
    fn an_entry_keeps_its_name_through_a_save_and_a_load() {
        let original = history(vec![entry(vec![moved("/h/a", "/h/b")])], vec![]);
        let restored = restore(&original, &[]);
        assert_eq!(restored.history.undo[0].id, original.undo[0].id);
    }

    #[test]
    fn a_batch_keeps_one_name_while_recorded_and_once_committed() {
        let mut history = History::default();
        history.record(moved("/h/a", "/h/b"));
        let while_recorded = parse_blocks(&history.encode()).unwrap().undo[0].id;
        history.commit();
        assert_eq!(history.undo[0].id, while_recorded);
    }

    #[test]
    fn what_decode_leaves_out_is_listed_as_forgotten() {
        let live = [item("/t/info/a.trashinfo", "/h/a", 7)];
        let gone = entry(vec![Change::Trashed {
            path: "/h/gone".into(),
            item: Trashed::from(&item("/t/info/x.trashinfo", "/h/gone", 3)),
        }]);
        let old = entry(vec![moved("/h/o", "/h/p")]);
        let kept = entry(vec![moved("/h/a", "/h/b")]);
        let original = history(vec![old.clone(), gone.clone(), kept], vec![]);

        let restored = History::decode(&original.encode(), 1, &live).unwrap();

        assert_eq!(restored.forgotten, [gone.id, old.id]);
    }

    #[test]
    fn an_empty_history_comes_back_empty() {
        let restored = restore(&History::default(), &[]);
        assert!(!restored.history.can_undo() && !restored.history.can_redo());
        assert_eq!(restored.dropped, 0);
    }

    #[test]
    fn every_kind_of_change_survives_a_round_trip() {
        let live = [item("/t/info/a.trashinfo", "/h/a", 7)];
        let held = Trashed::from(&live[0]);
        let undo = vec![
            entry(vec![
                Change::Created {
                    path: "/h/new".into(),
                    is_dir: true,
                },
                moved("/h/x", "/h/y"),
                Change::Copied {
                    src: "/h/s".into(),
                    dst: "/h/d".into(),
                },
                Change::Trashed {
                    path: "/h/a".into(),
                    item: held.clone(),
                },
            ]),
            entry(vec![Change::Produced {
                path: "/h/o.zip".into(),
                product: Product::Archive,
                held: None,
            }]),
        ];
        let redo = vec![entry(vec![
            Change::Trashed {
                path: "/h/b".into(),
                item: Trashed::spent(Path::new("/h/b")).unwrap(),
            },
            Change::Produced {
                path: "/h/out".into(),
                product: Product::Folder,
                held: Some(held),
            },
        ])];
        let original = history(undo, redo);

        let restored = restore(&original, &live);

        assert_eq!(restored.dropped, 0);
        assert_eq!(restored.history.undo, original.undo);
        assert_eq!(restored.history.redo, original.redo);
    }

    #[test]
    fn an_entry_whose_trash_item_is_gone_is_dropped_and_the_rest_kept() {
        let live = [item("/t/info/a.trashinfo", "/h/a", 7)];
        let kept = entry(vec![moved("/h/x", "/h/y")]);
        let gone = entry(vec![Change::Trashed {
            path: "/h/z".into(),
            item: Trashed::from(&item("/t/info/z.trashinfo", "/h/z", 9)),
        }]);
        let reused_name = entry(vec![Change::Trashed {
            path: "/h/a".into(),
            item: Trashed::from(&item("/t/info/a.trashinfo", "/h/a", 8)),
        }]);
        let original = history(vec![kept.clone(), gone, reused_name], vec![]);

        let restored = restore(&original, &live);

        assert_eq!(restored.history.undo, vec![kept]);
        assert_eq!(restored.dropped, 2);
    }

    #[test]
    fn a_batch_in_progress_is_written_as_an_entry_and_the_redo_is_not() {
        let mut original = history(vec![], vec![entry(vec![moved("/h/a", "/h/b")])]);
        original.record(moved("/h/c", "/h/d"));

        let restored = restore(&original, &[]);

        assert_eq!(
            changes_of(&restored.history.undo),
            [vec![moved("/h/c", "/h/d")]]
        );
        assert!(!restored.history.can_redo());
    }

    #[test]
    fn a_path_that_is_not_absolute_or_not_text_is_never_written() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let bytes = PathBuf::from(OsString::from_vec(b"/h/\xff".to_vec()));
        let kept = entry(vec![moved("/h/a", "/h/b")]);
        let original = history(
            vec![
                entry(vec![moved("ssh://box/a", "ssh://box/b")]),
                entry(vec![
                    moved("/h/a", "/h/b"),
                    Change::Created {
                        path: bytes,
                        is_dir: false,
                    },
                ]),
                kept.clone(),
            ],
            vec![],
        );

        let restored = restore(&original, &[]);

        assert_eq!(restored.history.undo, vec![kept]);
        assert_eq!(restored.dropped, 0);
    }

    #[test]
    fn a_line_that_does_not_parse_drops_only_its_entry() {
        let text = "mman-history 1\n@ undo\nM\t/h/a\t/h/b\n-\nM\trelative\t/h/b\n-\nM\t/h/c\n-\n\
                    Q\twhat\n-\nM\t/h/e\t/h/f\n-\nM\t/h/g\t/h/h\n";

        let restored = History::decode(text, 100, &[]).unwrap();

        assert_eq!(
            changes_of(&restored.history.undo),
            [vec![moved("/h/a", "/h/b")], vec![moved("/h/e", "/h/f")],]
        );
        // Unnamed entries get a name from their place in the file, dropped ones counted.
        assert_eq!(restored.history.undo[1].id, EntryId::legacy(4));
        assert_eq!(restored.dropped, 4);
    }

    #[test]
    fn a_handle_in_the_wrong_place_is_refused() {
        let live = [item("/t/info/a.trashinfo", "/h/a", 7)];
        let text =
            "mman-history 1\n@ undo\nT\t/h/a\n-\n@ redo\nT\t/h/a\t/t/info/a.trashinfo\t7\n-\n";

        let restored = History::decode(text, 100, &live).unwrap();

        assert!(!restored.history.can_undo() && !restored.history.can_redo());
        assert_eq!(restored.dropped, 2);
    }

    #[test]
    fn only_the_newest_entries_fit_in_the_capacity() {
        let entries: Vec<Entry> = (0..5)
            .map(|n| entry(vec![moved(&format!("/h/{n}"), "/h/to")]))
            .collect();
        let text = history(entries.clone(), vec![]).encode();

        let restored = History::decode(&text, 2, &[]).unwrap();

        assert_eq!(restored.history.undo, entries[3..]);
    }

    #[test]
    fn other_text_is_refused_by_its_first_line() {
        assert_eq!(
            History::decode("", 100, &[]).unwrap_err(),
            DecodeError::NotHistory
        );
        assert_eq!(
            History::decode("hello\n", 100, &[]).unwrap_err(),
            DecodeError::NotHistory
        );
        assert_eq!(
            History::decode("mman-history 3\n@ undo\n", 100, &[]).unwrap_err(),
            DecodeError::Version("3".into())
        );
    }

    /// Any absolute path a file name can hold: separators, tabs, newlines and backslashes
    /// included, which is what the escaping is for.
    fn any_path() -> impl Strategy<Value = PathBuf> {
        "/[^\\x00]{0,16}".prop_map(PathBuf::from)
    }

    fn any_change() -> impl Strategy<Value = Change> {
        prop_oneof![
            (any_path(), any::<bool>()).prop_map(|(path, is_dir)| Change::Created { path, is_dir }),
            (any_path(), any_path()).prop_map(|(from, to)| Change::Moved { from, to }),
            (any_path(), any_path()).prop_map(|(src, dst)| Change::Copied { src, dst }),
            (any_path(), any::<bool>()).prop_map(|(path, archive)| Change::Produced {
                path,
                product: if archive {
                    Product::Archive
                } else {
                    Product::Folder
                },
                held: None,
            }),
        ]
    }

    proptest! {
        #[test]
        fn encode_then_decode_gives_back_the_same_history(
            undo in prop::collection::vec(prop::collection::vec(any_change(), 1..4), 0..6),
            redo in prop::collection::vec(prop::collection::vec(any_change(), 1..4), 0..6),
        ) {
            let original = history(
                undo.into_iter().map(entry).collect(),
                redo.into_iter().map(entry).collect(),
            );

            let restored = restore(&original, &[]);

            prop_assert_eq!(restored.dropped, 0);
            prop_assert_eq!(restored.history.undo, original.undo);
            prop_assert_eq!(restored.history.redo, original.redo);
        }

        #[test]
        fn decode_never_panics_on_any_text(text in "\\PC{0,200}") {
            let _ = History::decode(&text, 100, &[]);
        }

        #[test]
        fn decode_never_panics_on_text_that_looks_like_a_history(
            body in prop::collection::vec("[MCYTP@ a-z/\\t\\\\0-9-]{0,24}", 0..12),
        ) {
            let text = format!("mman-history 1\n{}", body.join("\n"));
            let _ = History::decode(&text, 100, &[]);
        }
    }
}
