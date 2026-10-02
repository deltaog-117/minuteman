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

//! An archive read into a tree in memory: what is in it, how big, and where its bytes are, so a
//! folder can be listed without touching the data and a file opened on its own.
//!
//! An archive is untrusted input that gets opened just by moving the cursor onto it, so every read
//! is bounded: a limit on entries, on how much of a `.tar.gz` is inflated while indexing, on what
//! a zip's central directory may claim, and on how large one entry may be when it is opened.
//! Going over a limit is an error, never a quietly shortened listing, because a folder that looks
//! complete and is not would mislead.

use std::collections::HashMap;
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use flate2::read::GzDecoder;
use shared::{ReadSeek, Vfs};

use crate::address::Format;

/// Entries an archive may hold and still be browsed.
pub const MAX_ENTRIES: usize = 200_000;
/// Bytes a `.tar.gz` may inflate to while it is indexed, or while one entry is looked for in it.
pub const MAX_INFLATED: u64 = 512 << 20;
/// The largest entry that is opened (it is held in memory, since a compressed stream cannot seek).
pub const MAX_ENTRY_BYTES: u64 = 256 << 20;
/// A zip whose central directory claims more than this is refused before the `zip` crate reads it
/// whole into memory. Twin of the guard in the `preview` crate, kept separate because feature
/// crates do not depend on each other.
const MAX_ZIP_DIRECTORY: u32 = 8 << 20;
const MAX_NAME_CHARS: usize = 1024;

const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;
const S_IFLNK: u32 = 0o120_000;

/// What the archive file looked like when it was indexed. An index is reused only while the file
/// still looks the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub len: u64,
    pub modified: Option<SystemTime>,
}

/// Where an entry's bytes are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Position in a zip's central directory.
    Zip(usize),
    /// Offset of the data in a plain tar, which can be read in place.
    TarAt { start: u64 },
    /// Position among a `.tar.gz`'s entries; reaching it means inflating everything before it.
    TarGz { ordinal: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Dir,
    File(Source),
    /// Holds the link's text, which names another entry of the same archive or nothing.
    Symlink(String),
    /// A hard link, device or pipe: shown, never opened.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Cleaned of anything unsafe to draw; the same text is the key in lookups.
    pub name: String,
    pub content: Content,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Permission bits with the type bits of `st_mode`, when the archive recorded them.
    pub mode: Option<u32>,
    /// For a folder: its children, folders first then by name ignoring case.
    pub children: Vec<usize>,
}

impl Node {
    pub fn is_folder(&self) -> bool {
        matches!(self.content, Content::Dir)
    }
}

#[derive(Debug)]
pub struct Index {
    pub stamp: Stamp,
    nodes: Vec<Node>,
    by_path: HashMap<String, usize>,
}

/// The index of the root folder, which every archive has.
pub const ROOT: usize = 0;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// A name made safe to put on a terminal and in a lookup: control characters and the invisible
/// characters that reorder text become `?`, and an absurdly long one is cut with `…`.
fn clean(raw: &str) -> String {
    let mut chars = raw.chars();
    let mut name: String = chars
        .by_ref()
        .take(MAX_NAME_CHARS)
        .map(|c| {
            let unsafe_char = c.is_control()
                || matches!(c,
                    '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}');
            if unsafe_char { '?' } else { c }
        })
        .collect();
    if chars.next().is_some() {
        name.push('…');
    }
    name
}

/// The parts of an entry's name, or `None` for one that climbs out of the archive (`../x`), which
/// is left out rather than put somewhere it was not meant to be.
fn parts_of(raw: &str) -> Option<Vec<String>> {
    let mut parts: Vec<String> = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            name => parts.push(clean(name)),
        }
    }
    Some(parts)
}

struct Entry {
    parts: Vec<String>,
    content: Content,
    size: u64,
    modified: Option<SystemTime>,
    mode: Option<u32>,
}

struct Builder {
    nodes: Vec<Node>,
    by_path: HashMap<String, usize>,
}

impl Builder {
    fn new() -> Self {
        let root = Node {
            name: String::new(),
            content: Content::Dir,
            size: 0,
            modified: None,
            mode: None,
            children: Vec::new(),
        };
        Self {
            nodes: vec![root],
            by_path: HashMap::from([(String::new(), ROOT)]),
        }
    }

    fn push(&mut self, parent: usize, key: String, node: Node) -> io::Result<usize> {
        if self.nodes.len() >= MAX_ENTRIES {
            return Err(invalid(format!(
                "more than {MAX_ENTRIES} entries: too many to browse, extract it instead"
            )));
        }
        let at = self.nodes.len();
        self.nodes.push(node);
        self.nodes[parent].children.push(at);
        self.by_path.insert(key, at);
        Ok(at)
    }

    /// Adds an entry, making the folders above it that the archive does not list itself. A later
    /// file of the same name replaces an earlier one, as extracting would; a name that is a
    /// folder in one place and a file in another keeps the first.
    fn add(&mut self, entry: Entry) -> io::Result<()> {
        let Some((last, above)) = entry.parts.split_last() else {
            // An entry for the root itself (`./`) only says when it was changed.
            self.nodes[ROOT].modified = entry.modified.or(self.nodes[ROOT].modified);
            return Ok(());
        };
        let mut parent = ROOT;
        let mut key = String::new();
        for name in above {
            if !key.is_empty() {
                key.push('/');
            }
            key.push_str(name);
            parent = match self.by_path.get(&key) {
                Some(&at) if self.nodes[at].is_folder() => at,
                Some(_) => return Ok(()),
                None => self.push(parent, key.clone(), folder(name))?,
            };
        }
        if !key.is_empty() {
            key.push('/');
        }
        key.push_str(last);
        let node = Node {
            name: last.clone(),
            content: entry.content,
            size: entry.size,
            modified: entry.modified,
            mode: entry.mode,
            children: Vec::new(),
        };
        match self.by_path.get(&key).copied() {
            None => {
                self.push(parent, key, node)?;
            }
            Some(at) => {
                let old = &mut self.nodes[at];
                match (old.is_folder(), node.is_folder()) {
                    (true, true) => {
                        old.modified = node.modified.or(old.modified);
                        old.mode = node.mode.or(old.mode);
                    }
                    (false, false) => {
                        let children = std::mem::take(&mut old.children);
                        *old = Node { children, ..node };
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn finish(mut self, stamp: Stamp) -> Index {
        // Sorted once here rather than on every listing; the order matches the local backend's.
        let keys: Vec<(bool, String)> = self
            .nodes
            .iter()
            .map(|node| (!node.is_folder(), node.name.to_lowercase()))
            .collect();
        for node in &mut self.nodes {
            node.children.sort_by(|&a, &b| keys[a].cmp(&keys[b]));
        }
        Index {
            stamp,
            nodes: self.nodes,
            by_path: self.by_path,
        }
    }
}

fn folder(name: &str) -> Node {
    Node {
        name: name.to_owned(),
        content: Content::Dir,
        size: 0,
        modified: None,
        mode: None,
        children: Vec::new(),
    }
}

/// Days from 1970-01-01 to a civil date (proleptic Gregorian), after Howard Hinnant's algorithm.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// A zip's recorded time as a `SystemTime`. The format has no time zone, so the fields are taken
/// as UTC, which is also how this application writes them.
fn civil_time(
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
) -> Option<SystemTime> {
    let days = days_from_civil(i64::from(year), i64::from(month), i64::from(day));
    let seconds =
        days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second);
    u64::try_from(seconds)
        .ok()
        .map(|seconds| UNIX_EPOCH + Duration::from_secs(seconds))
}

fn unix_time(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

impl Index {
    /// Reads the table of contents of `file`, which is `format`.
    ///
    /// # Errors
    ///
    /// `InvalidData` for a damaged archive or one over a limit, and whatever reading the file
    /// reports.
    pub fn build(file: Box<dyn ReadSeek>, format: Format, stamp: Stamp) -> io::Result<Self> {
        let mut builder = Builder::new();
        match format {
            Format::Zip => index_zip(file, &mut builder)?,
            Format::Tar => {
                let mut archive = tar::Archive::new(BufReader::new(file));
                // Seeking past each entry's data makes this cost the headers, not the file.
                index_tar(archive.entries_with_seek()?, &mut builder, true)?;
            }
            Format::TarGz => {
                let inflated = Capped::new(GzDecoder::new(BufReader::new(file)), MAX_INFLATED);
                let mut archive = tar::Archive::new(inflated);
                index_tar(archive.entries()?, &mut builder, false)?;
            }
        }
        Ok(builder.finish(stamp))
    }

    pub fn node(&self, at: usize) -> &Node {
        &self.nodes[at]
    }

    /// Where `key` leads, following links in the archive (at most [`MAX_LINK_HOPS`] of them, so a
    /// loop ends). A link that points outside the archive leads nowhere. The last name is followed
    /// only when `follow_last` is set, so a link itself can be asked about.
    pub fn resolve(&self, key: &str, follow_last: bool) -> Option<usize> {
        // Names still to walk, the next one last.
        let mut pending: Vec<String> = split_rev(key);
        let mut at: Vec<String> = Vec::new();
        let mut hops = 0;
        while let Some(part) = pending.pop() {
            match part.as_str() {
                "." => continue,
                ".." => {
                    at.pop()?;
                    continue;
                }
                _ => at.push(part),
            }
            let found = *self.by_path.get(&at.join("/"))?;
            if let Content::Symlink(target) = &self.nodes[found].content
                && (follow_last || !pending.is_empty())
            {
                hops += 1;
                if hops > MAX_LINK_HOPS || target.starts_with('/') {
                    return None;
                }
                // The target is relative to the folder the link is in.
                at.pop();
                pending.extend(split_rev(target));
            }
        }
        self.by_path.get(&at.join("/")).copied()
    }
}

/// How many links one lookup follows before giving up.
pub const MAX_LINK_HOPS: usize = 16;

fn split_rev(text: &str) -> Vec<String> {
    text.split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .rev()
        .collect()
}

/// A reader that fails, rather than reporting end of file, once `left` bytes have been read from
/// `inner` and more are still coming. A plain `Read::take` would end quietly, and a tar reader
/// that finds the stream ended takes it for the archive's real end, so a cut-off index would pass
/// for a complete one.
struct Capped<R> {
    inner: R,
    left: u64,
}

impl<R> Capped<R> {
    fn new(inner: R, left: u64) -> Self {
        Self { inner, left }
    }
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            // Exactly at the cap is fine if the stream ends there; only more data is over it.
            return match self.inner.read(&mut [0u8; 1])? {
                0 => Ok(0),
                _ => Err(invalid("archive inflates to more than can be browsed")),
            };
        }
        let room = usize::try_from(self.left).map_or(buf.len(), |left| buf.len().min(left));
        let read = self.inner.read(&mut buf[..room])?;
        self.left -= read as u64;
        Ok(read)
    }
}

/// Whether the zip whose last bytes are `tail` claims a central directory small enough to open.
/// Zip64 (all-ones fields, used past 65,535 entries or 4 GiB) is turned away as well. Every place
/// that looks like an end-of-central-directory record is checked, since a reader may skip a bogus
/// one and use an earlier, and at least one must end exactly at the end of the file.
fn directory_is_small(tail: &[u8]) -> bool {
    const SIGNATURE: &[u8] = b"PK\x05\x06";
    const RECORD: usize = 22;
    let mut ends_at_the_end = false;
    for at in 0..tail.len().saturating_sub(RECORD - 1) {
        if !tail[at..].starts_with(SIGNATURE) {
            continue;
        }
        let entries = u16::from_le_bytes([tail[at + 10], tail[at + 11]]);
        let size = u32::from_le_bytes([tail[at + 12], tail[at + 13], tail[at + 14], tail[at + 15]]);
        if entries == u16::MAX || size == u32::MAX || size > MAX_ZIP_DIRECTORY {
            return false;
        }
        let comment = u16::from_le_bytes([tail[at + 20], tail[at + 21]]) as usize;
        ends_at_the_end |= at + RECORD + comment == tail.len();
    }
    ends_at_the_end
}

fn index_zip(mut file: Box<dyn ReadSeek>, builder: &mut Builder) -> io::Result<()> {
    let len = file.seek(SeekFrom::End(0))?;
    // 22 bytes of record plus a comment of at most 65,535.
    let tail_len = len.min(22 + 65_535);
    file.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = Vec::new();
    (&mut file).take(tail_len).read_to_end(&mut tail)?;
    if !directory_is_small(&tail) {
        return Err(invalid(
            "not a zip this build will open (damaged, zip64 or oversized)",
        ));
    }
    let mut zip = zip::ZipArchive::new(BufReader::new(file)).map_err(io::Error::other)?;
    if zip.len() > MAX_ENTRIES {
        return Err(invalid(format!(
            "{} entries: too many to browse, extract it instead",
            zip.len()
        )));
    }
    for position in 0..zip.len() {
        // The raw entry: only its header is read, so nothing is decompressed.
        let entry = zip.by_index_raw(position).map_err(io::Error::other)?;
        let Some(parts) = parts_of(entry.name()) else {
            continue;
        };
        let is_folder = entry.is_dir(); // vfs-gate: archive entry
        let unix_mode = entry.unix_mode();
        // A link inside a zip is a file whose data is the link's text; it is offered as that file
        // rather than followed, which keeps it from pointing anywhere.
        let permissions = unix_mode.map(|mode| mode & 0o7777);
        let (content, size, mode) = if is_folder {
            (Content::Dir, 0, permissions.map(|p| S_IFDIR | p))
        } else {
            (
                Content::File(Source::Zip(position)),
                entry.size(),
                permissions.map(|p| S_IFREG | p),
            )
        };
        let modified = entry.last_modified().and_then(|t| {
            civil_time(
                t.year(),
                t.month(),
                t.day(),
                t.hour(),
                t.minute(),
                t.second(),
            )
        });
        builder.add(Entry {
            parts,
            content,
            size,
            modified,
            mode,
        })?;
    }
    Ok(())
}

fn index_tar<'a, R: Read + 'a>(
    entries: impl Iterator<Item = io::Result<tar::Entry<'a, R>>>,
    builder: &mut Builder,
    positioned: bool,
) -> io::Result<()> {
    for (ordinal, item) in entries.enumerate() {
        let entry = item?;
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() {
            continue;
        }
        let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let Some(parts) = parts_of(&name) else {
            continue;
        };
        let permissions = entry.header().mode().ok().map(|mode| mode & 0o7777);
        let modified = entry.header().mtime().ok().map(unix_time);
        let is_folder = kind.is_dir() || name.ends_with('/'); // vfs-gate: archive entry
        let is_regular = kind.is_file() || kind.is_contiguous(); // vfs-gate: archive entry
        let (content, size, type_bits) = if is_folder {
            (Content::Dir, 0, S_IFDIR)
        } else if kind.is_symlink() {
            let target = entry
                .link_name_bytes()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_default();
            (Content::Symlink(target), entry.size(), S_IFLNK)
        } else if is_regular {
            let source = if positioned {
                Source::TarAt {
                    start: entry.raw_file_position(),
                }
            } else {
                Source::TarGz { ordinal }
            };
            (Content::File(source), entry.size(), S_IFREG)
        } else {
            (Content::Other, entry.size(), 0)
        };
        builder.add(Entry {
            parts,
            content,
            size,
            modified,
            mode: permissions.map(|p| type_bits | p),
        })?;
    }
    Ok(())
}

/// A window onto part of a file: reads and seeks as if the part were the whole file. Lets a plain
/// tar's entry be read where it lies, with no copy.
pub(crate) struct Section {
    inner: Box<dyn ReadSeek>,
    start: u64,
    len: u64,
    position: u64,
}

impl Read for Section {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.len.saturating_sub(self.position);
        let room = usize::try_from(left).map_or(buf.len(), |left| buf.len().min(left));
        if room == 0 {
            return Ok(0);
        }
        self.inner
            .seek(SeekFrom::Start(self.start + self.position))?;
        let read = self.inner.read(&mut buf[..room])?;
        self.position += read as u64;
        Ok(read)
    }
}

impl Seek for Section {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(delta) => self.len.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
        };
        self.position = target
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before the start"))?;
        Ok(self.position)
    }
}

/// Reads all of `reader` into memory, failing past [`MAX_ENTRY_BYTES`].
fn read_capped(reader: impl Read, declared: u64) -> io::Result<Vec<u8>> {
    if declared > MAX_ENTRY_BYTES {
        return Err(too_large());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(declared).unwrap_or(0).min(1 << 20));
    // One byte past the cap tells "exactly the cap" from "more than it".
    reader.take(MAX_ENTRY_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_ENTRY_BYTES {
        return Err(too_large());
    }
    Ok(bytes)
}

fn too_large() -> io::Error {
    io::Error::new(
        io::ErrorKind::FileTooLarge,
        format!(
            "larger than {} MiB, extract it instead",
            MAX_ENTRY_BYTES >> 20
        ),
    )
}

/// Opens one file of an archive. A plain tar's is read in place; every other kind is unpacked into
/// memory, because a deflate or gzip stream cannot seek and the caller needs to.
///
/// # Errors
///
/// `PermissionDenied` for an encrypted zip entry, `FileTooLarge` over [`MAX_ENTRY_BYTES`], and
/// `InvalidData` or the file's own error otherwise.
pub fn open_entry(
    vfs: &dyn Vfs,
    archive: &std::path::Path,
    source: Source,
    size: u64,
) -> io::Result<Box<dyn ReadSeek>> {
    let file = vfs.open_read(archive).map_err(|error| match error {
        shared::VfsError::Io { source, .. } => source,
        other => io::Error::other(other.to_string()),
    })?;
    match source {
        Source::TarAt { start } => Ok(Box::new(Section {
            inner: file,
            start,
            len: size,
            position: 0,
        })),
        Source::Zip(position) => {
            let mut zip = zip::ZipArchive::new(BufReader::new(file)).map_err(io::Error::other)?;
            let entry = zip.by_index(position).map_err(|error| {
                use zip::result::ZipError;
                match &error {
                    ZipError::UnsupportedArchive(reason)
                        if *reason == ZipError::PASSWORD_REQUIRED =>
                    {
                        io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "encrypted: extract it to give the password",
                        )
                    }
                    _ => io::Error::other(error),
                }
            })?;
            let declared = entry.size();
            Ok(Box::new(Cursor::new(read_capped(entry, declared)?)))
        }
        Source::TarGz { ordinal } => {
            let inflated = Capped::new(GzDecoder::new(BufReader::new(file)), MAX_INFLATED);
            let mut archive = tar::Archive::new(inflated);
            let mut entries = archive.entries()?;
            let entry = entries
                .nth(ordinal)
                .ok_or_else(|| invalid("the archive changed while it was open"))??;
            let declared = entry.size();
            Ok(Box::new(Cursor::new(read_capped(entry, declared)?)))
        }
    }
}

#[cfg(test)]
impl Section {
    pub(crate) fn for_test(inner: Box<dyn ReadSeek>, start: u64, len: u64) -> Self {
        Self {
            inner,
            start,
            len,
            position: 0,
        }
    }
}

#[cfg(test)]
pub(crate) fn civil_seconds_for_test(year: u16, month: u8, day: u8) -> i64 {
    civil_time(year, month, day, 0, 0, 0)
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(-1, |d| i64::try_from(d.as_secs()).unwrap())
}
