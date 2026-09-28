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

//! Compressing files into, and extracting them from, `zip`, `tar` and `tar.gz` archives.
//!
//! Like `trash`, this works on the local filesystem directly rather than through `Vfs`: an
//! archive is read and written as a stream, which a remote backend cannot offer.
//!
//! An archive is untrusted input, and extracting one writes to disk, so every entry passes through
//! [`SafePath`] before anything is created, and the rules below hold whatever the archive says:
//!
//! - A name that is absolute, climbs with `..`, or holds a control character is refused with
//!   [`FileOpsError::UnsafeEntry`] (the "zip-slip" attack).
//! - Nothing is written through a symlink: if a directory on the way to an entry is a symlink, the
//!   entry is refused, and a file is created with `create_new`, which never follows a link.
//! - A symlink entry is only created when its target descends (no `..`, not absolute), so no chain
//!   of links can lead out of the destination. Any other link, and hard links, devices and
//!   fifos, are skipped and counted in [`ExtractSummary::unsupported`].
//! - Permissions are clamped: no setuid/setgid/sticky bits, no group or other write, and always
//!   owner read and write, so an entry cannot be made unreadable or dangerous.
//! - [`ExtractLimits`] bounds the entry count and the bytes actually written (not the sizes the
//!   archive declares, which may lie), which stops a zip bomb; a file cut off by the limit is
//!   deleted.
//!
//! Writing an archive goes to a hidden sibling file that replaces the target only once complete,
//! so a cancelled or failed run never leaves a half-written archive under the real name.
//! Symlinks are stored as links, never followed. Zip entries carry no modification time.

use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use shared::VfsError;

use crate::{
    ConflictPolicy, FileOpsError, Outcome, ProgressFn, guard_distinct, guard_not_recursive, report,
};

/// The archive formats that can be read and written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    TarGz,
}

impl Format {
    /// Which format `path` is, going by its name alone (`.zip`, `.jar`, `.tar`, `.tar.gz`,
    /// `.tgz`), the same rule the preview pane uses.
    pub fn of(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?.to_lowercase();
        if name.ends_with(".zip") || name.ends_with(".jar") {
            Some(Self::Zip)
        } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Some(Self::TarGz)
        } else if name.ends_with(".tar") {
            Some(Self::Tar)
        } else {
            None
        }
    }

    /// `path`'s file name without its archive extension (`photos.tar.gz` gives `photos`), or
    /// `None` if it is not an archive or nothing is left of the name.
    pub fn stem_of(path: &Path) -> Option<String> {
        let name = path.file_name()?.to_str()?;
        let lower = name.to_lowercase();
        let extension = [".tar.gz", ".tgz", ".tar", ".zip", ".jar"]
            .into_iter()
            .find(|extension| lower.ends_with(extension))?;
        // Lower-casing can change a name's byte length, so cut the original by characters.
        let kept = name.chars().count() - extension.chars().count();
        let stem: String = name.chars().take(kept).collect();
        (!stem.is_empty()).then_some(stem)
    }

    /// The file-name ending an archive of this format is given.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Tar => "tar",
            Self::TarGz => "tar.gz",
        }
    }
}

/// How hard [`compress_with_level`] works to make an archive small. Ignored for a plain `tar`,
/// which is not compressed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Level {
    /// The quickest, for a big tree or a slow disk.
    Fast,
    #[default]
    Normal,
    /// The smallest, at the cost of time.
    Best,
}

impl Level {
    /// The deflate level (1 to 9) this maps to.
    fn deflate(self) -> u32 {
        match self {
            Self::Fast => 1,
            Self::Normal => 6,
            Self::Best => 9,
        }
    }
}

/// How much an extraction may create before it is stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractLimits {
    /// Entries (files, directories, links) an archive may hold.
    pub max_entries: u64,
    /// Bytes of file content that may be written in all.
    pub max_bytes: u64,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        Self {
            max_entries: 1_000_000,
            max_bytes: 32 << 30,
        }
    }
}

/// What an extraction did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExtractSummary {
    pub files: u64,
    pub dirs: u64,
    pub symlinks: u64,
    /// Entries left alone because their name was taken and the policy was `Skip`.
    pub skipped: u64,
    /// Entries this module does not extract: hard links, devices, fifos, and symlinks that could
    /// point outside the destination.
    pub unsupported: u64,
}

/// A path taken from an archive that is safe to join onto a destination: relative, non-empty,
/// made only of ordinary names, none holding a control character. The only way to get one is
/// [`SafePath::try_new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafePath(PathBuf);

impl SafePath {
    /// `Ok(None)` for a name with nothing in it (`.`, `./`), which is the archive's own root and
    /// simply means "the destination".
    ///
    /// # Errors
    ///
    /// [`FileOpsError::UnsafeEntry`] for an absolute path, one with `..` or a drive prefix, or a
    /// name with a control character (an escape sequence in a file name could repaint the
    /// terminal of whoever lists it).
    pub fn try_new(raw: &Path) -> Result<Option<Self>, FileOpsError> {
        let mut safe = PathBuf::new();
        for component in raw.components() {
            match component {
                Component::CurDir => {}
                Component::Normal(part) => {
                    if part.to_string_lossy().chars().any(char::is_control) {
                        return Err(unsafe_entry(raw));
                    }
                    safe.push(part);
                }
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(unsafe_entry(raw));
                }
            }
        }
        Ok((!safe.as_os_str().is_empty()).then_some(Self(safe)))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

fn unsafe_entry(raw: &Path) -> FileOpsError {
    FileOpsError::UnsafeEntry {
        name: raw.to_path_buf(),
    }
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> FileOpsError {
    let path = path.to_path_buf();
    move |source| FileOpsError::Vfs(VfsError::Io { path, source })
}

fn damaged(reason: impl std::fmt::Display) -> FileOpsError {
    FileOpsError::Archive(reason.to_string())
}

// ---------------------------------------------------------------------------------------------
// Extracting
// ---------------------------------------------------------------------------------------------

/// What one archive entry is, as far as extraction cares.
enum Kind {
    File,
    Dir,
    Symlink(PathBuf),
    Other,
}

struct Incoming {
    name: PathBuf,
    kind: Kind,
    mode: Option<u32>,
}

/// Extracts `archive` into the directory `dest` (created if missing). A file that already exists
/// is handled per `policy`: `Abort` fails, `Skip` leaves it, `Overwrite` replaces it (a directory
/// in the way of a file is never removed, and is an error). Whatever was written before a
/// failure or a cancel stays on disk.
///
/// # Errors
///
/// [`FileOpsError::UnsupportedArchive`] for an unknown extension, [`FileOpsError::UnsafeEntry`]
/// for an entry that would escape `dest`, [`FileOpsError::ArchiveLimit`] past `limits`,
/// [`FileOpsError::Archive`] for a damaged archive, and [`FileOpsError::Cancelled`] when
/// `on_progress` asks to stop.
pub fn extract(
    archive: &Path,
    dest: &Path,
    policy: ConflictPolicy,
    limits: ExtractLimits,
    on_progress: &mut ProgressFn<'_>,
) -> Result<ExtractSummary, FileOpsError> {
    let format = Format::of(archive)
        .ok_or_else(|| FileOpsError::UnsupportedArchive(archive.to_path_buf()))?;
    let file = File::open(archive).map_err(io_error(archive))?;
    fs::create_dir_all(dest).map_err(io_error(dest))?;

    let mut sink = Sink {
        dest,
        policy,
        limits,
        on_progress,
        summary: ExtractSummary::default(),
        entries: 0,
        bytes: 0,
    };
    match format {
        Format::Zip => extract_zip(file, &mut sink)?,
        Format::Tar => extract_tar(BufReader::new(file), &mut sink)?,
        Format::TarGz => {
            // Tar headers and padding come on top of the file data; this is generous for them.
            let allowance = limits
                .max_bytes
                .saturating_add(limits.max_entries.saturating_mul(4096));
            let inflated = Capped::new(GzDecoder::new(BufReader::new(file)), allowance);
            extract_tar(inflated, &mut sink)?;
        }
    }
    Ok(sink.summary)
}

fn extract_zip(file: File, sink: &mut Sink<'_>) -> Result<(), FileOpsError> {
    let mut archive = zip::ZipArchive::new(BufReader::new(file)).map_err(damaged)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(damaged)?;
        // Some Windows tools write `\` between names; a zip's separator is `/`.
        let name = PathBuf::from(entry.name().replace('\\', "/"));
        let mode = entry.unix_mode();
        let kind = if entry.is_dir() {
            Kind::Dir
        } else if entry.is_symlink() {
            let mut target = Vec::new();
            // A link target is a path; nothing longer than PATH_MAX is one.
            (&mut entry)
                .take(4096)
                .read_to_end(&mut target)
                .map_err(damaged)?;
            match String::from_utf8(target) {
                Ok(target) => Kind::Symlink(PathBuf::from(target)),
                Err(_) => Kind::Other,
            }
        } else {
            Kind::File
        };
        sink.accept(Incoming { name, kind, mode }, &mut entry)?;
    }
    Ok(())
}

fn extract_tar(reader: impl Read, sink: &mut Sink<'_>) -> Result<(), FileOpsError> {
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries().map_err(damaged)? {
        let mut entry = entry.map_err(damaged)?;
        let name = entry.path().map_err(damaged)?.into_owned();
        let header = entry.header();
        let mode = header.mode().ok();
        let kind = match header.entry_type() {
            t if t.is_file() => Kind::File,
            t if t.is_dir() => Kind::Dir,
            t if t.is_symlink() => match entry.link_name().map_err(damaged)? {
                Some(target) => Kind::Symlink(target.into_owned()),
                None => Kind::Other,
            },
            _ => Kind::Other,
        };
        sink.accept(Incoming { name, kind, mode }, &mut entry)?;
    }
    Ok(())
}

struct Sink<'a> {
    dest: &'a Path,
    policy: ConflictPolicy,
    limits: ExtractLimits,
    on_progress: &'a mut ProgressFn<'a>,
    summary: ExtractSummary,
    entries: u64,
    bytes: u64,
}

impl Sink<'_> {
    fn accept(&mut self, incoming: Incoming, data: &mut dyn Read) -> Result<(), FileOpsError> {
        self.entries += 1;
        if self.entries > self.limits.max_entries {
            return Err(FileOpsError::ArchiveLimit {
                what: "entry count",
                limit: self.limits.max_entries,
            });
        }
        let Some(safe) = SafePath::try_new(&incoming.name)? else {
            return Ok(());
        };
        refuse_symlinked_ancestors(self.dest, &safe)?;
        let target = self.dest.join(safe.as_path());

        match incoming.kind {
            Kind::Dir => {
                fs::create_dir_all(&target).map_err(io_error(&target))?;
                set_mode(&target, incoming.mode, 0o700).map_err(io_error(&target))?;
                self.summary.dirs += 1;
            }
            Kind::File => {
                if !self.write_file(&target, incoming.mode, data)? {
                    return Ok(());
                }
                self.summary.files += 1;
            }
            Kind::Symlink(link) if descends(&link) => {
                if !self.write_symlink(&target, &link)? {
                    return Ok(());
                }
                self.summary.symlinks += 1;
            }
            Kind::Symlink(_) | Kind::Other => {
                self.summary.unsupported += 1;
                return Ok(());
            }
        }
        report(self.on_progress, &target)
    }

    /// Whether the file was written (`false` when the policy skipped it).
    fn write_file(
        &mut self,
        target: &Path,
        mode: Option<u32>,
        data: &mut dyn Read,
    ) -> Result<bool, FileOpsError> {
        ensure_parent(target)?;
        let Some(mut file) = self.create(target, |path| {
            OpenOptions::new().write(true).create_new(true).open(path)
        })?
        else {
            return Ok(false);
        };

        match copy_within(data, &mut file, self.limits.max_bytes - self.bytes) {
            Ok(written) => self.bytes += written,
            Err(e) => {
                // A file cut short by a limit or a read error is not the file the archive holds.
                drop(file);
                let _ = fs::remove_file(target);
                return Err(match e.kind() {
                    io::ErrorKind::FileTooLarge => FileOpsError::ArchiveLimit {
                        what: "extracted size",
                        limit: self.limits.max_bytes,
                    },
                    _ => damaged(e),
                });
            }
        }
        drop(file);
        set_mode(target, mode, 0o600).map_err(io_error(target))?;
        Ok(true)
    }

    fn write_symlink(&mut self, target: &Path, link: &Path) -> Result<bool, FileOpsError> {
        ensure_parent(target)?;
        Ok(self
            .create(target, |path| make_symlink(link, path))?
            .is_some())
    }

    /// Runs `make` to create `target`, resolving "already exists" with the policy. `None` when the
    /// policy skipped the entry.
    fn create<T>(
        &mut self,
        target: &Path,
        make: impl Fn(&Path) -> io::Result<T>,
    ) -> Result<Option<T>, FileOpsError> {
        match make(target) {
            Ok(made) => Ok(Some(made)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => match self.policy {
                ConflictPolicy::Abort => Err(FileOpsError::Vfs(VfsError::AlreadyExists(
                    target.to_path_buf(),
                ))),
                ConflictPolicy::Skip => {
                    self.summary.skipped += 1;
                    Ok(None)
                }
                ConflictPolicy::Overwrite => {
                    // `remove_file` unlinks a symlink itself and refuses a directory, so an
                    // archive can neither redirect the write nor delete a tree.
                    fs::remove_file(target).map_err(io_error(target))?;
                    make(target).map(Some).map_err(io_error(target))
                }
            },
            Err(e) => Err(io_error(target)(e)),
        }
    }
}

fn ensure_parent(target: &Path) -> Result<(), FileOpsError> {
    match target.parent() {
        Some(parent) => fs::create_dir_all(parent).map_err(io_error(parent)),
        None => Ok(()),
    }
}

/// Refuses an entry when a directory between `dest` and it is a symlink, since writing there
/// would land wherever the link points.
fn refuse_symlinked_ancestors(dest: &Path, safe: &SafePath) -> Result<(), FileOpsError> {
    let components: Vec<_> = safe.as_path().components().collect();
    let mut walked = dest.to_path_buf();
    for component in &components[..components.len() - 1] {
        walked.push(component);
        match fs::symlink_metadata(&walked) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(unsafe_entry(safe.as_path()));
            }
            Ok(_) => {}
            // Nothing exists from here down, so nothing below can be a link.
            Err(e) if e.kind() == io::ErrorKind::NotFound => break,
            Err(e) => return Err(io_error(&walked)(e)),
        }
    }
    Ok(())
}

/// Whether a symlink target only ever goes down from the link's own directory. Such a link cannot
/// point outside the destination, and neither can any chain of them.
fn descends(link: &Path) -> bool {
    let mut named = false;
    for component in link.components() {
        match component {
            Component::Normal(_) => named = true,
            Component::CurDir => {}
            _ => return false,
        }
    }
    named
}

#[cfg(unix)]
fn make_symlink(link: &Path, at: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(link, at)
}

#[cfg(not(unix))]
fn make_symlink(_link: &Path, _at: &Path) -> io::Result<()> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

/// Applies an archive's permission bits, clamped: setuid, setgid, sticky and group/other write
/// are dropped, and `floor` (owner read/write, or full owner access for a directory) is forced on.
#[cfg(unix)]
fn set_mode(path: &Path, mode: Option<u32>, floor: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let Some(mode) = mode else { return Ok(()) };
    let clamped = (mode & 0o777 & !0o022) | floor;
    fs::set_permissions(path, fs::Permissions::from_mode(clamped))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: Option<u32>, _floor: u32) -> io::Result<()> {
    Ok(())
}

/// Copies `from` into `to`, failing with `FileTooLarge` if more than `room` bytes come. Reading
/// one byte past `room` is what tells "exactly full" from "over".
fn copy_within(from: &mut dyn Read, to: &mut impl Write, room: u64) -> io::Result<u64> {
    let copied = io::copy(&mut from.take(room.saturating_add(1)), to)?;
    if copied > room {
        return Err(io::Error::from(io::ErrorKind::FileTooLarge));
    }
    Ok(copied)
}

/// A reader that fails, rather than reporting end of file, once `left` bytes have been read and
/// more are still coming. `Read::take` would end quietly, and a tar reader takes that for the
/// archive's real end, so a cut-off extraction would pass for a complete one.
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
            return match self.inner.read(&mut [0u8; 1])? {
                0 => Ok(0),
                _ => Err(io::Error::from(io::ErrorKind::FileTooLarge)),
            };
        }
        let room = usize::try_from(self.left).map_or(buf.len(), |left| buf.len().min(left));
        let read = self.inner.read(&mut buf[..room])?;
        self.left -= read as u64;
        Ok(read)
    }
}

// ---------------------------------------------------------------------------------------------
// Compressing
// ---------------------------------------------------------------------------------------------

/// Packs `sources` into a new archive `out` of `format`. Each source becomes a top-level entry
/// named after its file name, directories with everything under them. If `out` already exists,
/// `policy` decides: `Abort` fails, `Skip` returns [`Outcome::Skipped`], `Overwrite` replaces it
/// once the new archive is complete.
///
/// # Errors
///
/// [`FileOpsError::NothingToArchive`] for no sources, [`FileOpsError::SameLocation`] or
/// [`FileOpsError::RecursiveDestination`] when `out` is a source or inside one,
/// [`FileOpsError::UnarchivableName`] for a name a zip cannot hold (not UTF-8), and
/// [`FileOpsError::Cancelled`] when `on_progress` asks to stop; on any error `out` is untouched.
pub fn compress(
    sources: &[PathBuf],
    out: &Path,
    format: Format,
    policy: ConflictPolicy,
    on_progress: &mut ProgressFn<'_>,
) -> Result<Outcome, FileOpsError> {
    compress_with_level(sources, out, format, Level::default(), policy, on_progress)
}

/// [`compress`] with a chosen [`Level`].
pub fn compress_with_level(
    sources: &[PathBuf],
    out: &Path,
    format: Format,
    level: Level,
    policy: ConflictPolicy,
    on_progress: &mut ProgressFn<'_>,
) -> Result<Outcome, FileOpsError> {
    if sources.is_empty() {
        return Err(FileOpsError::NothingToArchive);
    }
    for source in sources {
        guard_distinct(source, out)?;
        guard_not_recursive(source, out)?;
        source
            .file_name()
            .ok_or_else(|| FileOpsError::NoParent(source.clone()))?;
    }
    if fs::symlink_metadata(out).is_ok() {
        match policy {
            ConflictPolicy::Abort => {
                return Err(FileOpsError::Vfs(VfsError::AlreadyExists(
                    out.to_path_buf(),
                )));
            }
            ConflictPolicy::Skip => return Ok(Outcome::Skipped),
            ConflictPolicy::Overwrite => {}
        }
    }

    let partial = partial_path(out)?;
    let written = write_archive(sources, &partial, format, level, on_progress)
        .and_then(|()| fs::rename(&partial, out).map_err(io_error(out)));
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
    written.map(|()| Outcome::Completed)
}

/// A hidden sibling of `out` to build the archive in.
fn partial_path(out: &Path) -> Result<PathBuf, FileOpsError> {
    let name = out
        .file_name()
        .ok_or_else(|| FileOpsError::NoParent(out.to_path_buf()))?;
    let mut partial = std::ffi::OsString::from(".");
    partial.push(name);
    partial.push(format!(".{}.partial", std::process::id()));
    Ok(out.with_file_name(partial))
}

fn write_archive(
    sources: &[PathBuf],
    partial: &Path,
    format: Format,
    level: Level,
    on_progress: &mut ProgressFn<'_>,
) -> Result<(), FileOpsError> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(partial)
        .map_err(io_error(partial))?;
    let sink = BufWriter::new(file);
    match format {
        Format::Zip => {
            let mut packer = ZipPacker(zip::ZipWriter::new(sink), level);
            walk(sources, &mut packer, on_progress)?;
            let mut sink = packer.0.finish().map_err(damaged)?;
            sink.flush().map_err(io_error(partial))
        }
        Format::Tar => {
            let mut packer = TarPacker::new(sink);
            walk(sources, &mut packer, on_progress)?;
            let mut sink = packer.0.into_inner().map_err(io_error(partial))?;
            sink.flush().map_err(io_error(partial))
        }
        Format::TarGz => {
            let mut packer =
                TarPacker::new(GzEncoder::new(sink, Compression::new(level.deflate())));
            walk(sources, &mut packer, on_progress)?;
            let encoder = packer.0.into_inner().map_err(io_error(partial))?;
            let mut sink = encoder.finish().map_err(io_error(partial))?;
            sink.flush().map_err(io_error(partial))
        }
    }
}

/// One archive being written. `real` is the file on disk, `name` what it is called inside.
trait Packer {
    fn add(&mut self, real: &Path, name: &Path, meta: &Metadata) -> Result<(), FileOpsError>;
}

/// Adds every source, and everything under each directory, in name order so the same tree always
/// makes the same archive. Anything that is not a file, directory or symlink is left out.
fn walk(
    sources: &[PathBuf],
    packer: &mut dyn Packer,
    on_progress: &mut ProgressFn<'_>,
) -> Result<(), FileOpsError> {
    let mut pending: Vec<(PathBuf, PathBuf)> = sources
        .iter()
        .rev()
        .filter_map(|s| Some((s.clone(), PathBuf::from(s.file_name()?))))
        .collect();
    while let Some((real, name)) = pending.pop() {
        let meta = fs::symlink_metadata(&real).map_err(io_error(&real))?;
        let kind = meta.file_type();
        if !(kind.is_file() || kind.is_dir() || kind.is_symlink()) {
            continue;
        }
        packer.add(&real, &name, &meta)?;
        if kind.is_dir() {
            let mut children: Vec<_> = fs::read_dir(&real)
                .and_then(|entries| entries.map(|e| e.map(|e| e.file_name())).collect())
                .map_err(io_error(&real))?;
            children.sort();
            for child in children.into_iter().rev() {
                pending.push((real.join(&child), name.join(&child)));
            }
        }
        report(on_progress, &real)?;
    }
    Ok(())
}

#[cfg(unix)]
fn mode_of(meta: &Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn mode_of(meta: &Metadata) -> u32 {
    if meta.is_dir() { 0o755 } else { 0o644 }
}

struct TarPacker<W: Write>(tar::Builder<W>);

impl<W: Write> TarPacker<W> {
    fn new(writer: W) -> Self {
        let mut builder = tar::Builder::new(writer);
        builder.follow_symlinks(false);
        Self(builder)
    }
}

impl<W: Write> Packer for TarPacker<W> {
    fn add(&mut self, real: &Path, name: &Path, _meta: &Metadata) -> Result<(), FileOpsError> {
        // With links not followed this adds a file, a symlink, or just a directory's own entry.
        self.0
            .append_path_with_name(real, name)
            .map_err(io_error(real))
    }
}

struct ZipPacker<W: Write + io::Seek>(zip::ZipWriter<W>, Level);

impl<W: Write + io::Seek> Packer for ZipPacker<W> {
    fn add(&mut self, real: &Path, name: &Path, meta: &Metadata) -> Result<(), FileOpsError> {
        let stored = zip_name(name)?;
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(i64::from(self.1.deflate())))
            .unix_permissions(mode_of(meta))
            .large_file(meta.len() > u64::from(u32::MAX));
        let kind = meta.file_type();
        if kind.is_dir() {
            self.0.add_directory(stored, options).map_err(damaged)
        } else if kind.is_symlink() {
            let target = fs::read_link(real).map_err(io_error(real))?;
            let target = target
                .to_str()
                .ok_or_else(|| FileOpsError::UnarchivableName(target.clone()))?;
            self.0.add_symlink(stored, target, options).map_err(damaged)
        } else {
            self.0.start_file(stored, options).map_err(damaged)?;
            let mut source = File::open(real).map_err(io_error(real))?;
            io::copy(&mut source, &mut self.0)
                .map(drop)
                .map_err(io_error(real))
        }
    }
}

/// `name` as a zip stores it: UTF-8, `/`-separated.
fn zip_name(name: &Path) -> Result<String, FileOpsError> {
    let parts: Option<Vec<&str>> = name
        .components()
        .map(|c| match c {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect();
    parts
        .map(|parts| parts.join("/"))
        .ok_or_else(|| FileOpsError::UnarchivableName(name.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::ops::ControlFlow;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch(label: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "minuteman-archive-test-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn go(_: &Path) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn unpack(
        archive: &Path,
        dest: &Path,
        policy: ConflictPolicy,
    ) -> Result<ExtractSummary, FileOpsError> {
        extract(archive, dest, policy, ExtractLimits::default(), &mut go)
    }

    /// One 512-byte ustar header (checksum filled in) followed by the data, padded to a block.
    /// Written by hand because `tar::Builder` refuses the hostile names these tests need.
    fn tar_entry(name: &str, typeflag: u8, link: &str, data: &[u8]) -> Vec<u8> {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..107].copy_from_slice(b"0000644");
        header[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
        header[156] = typeflag;
        header[157..157 + link.len()].copy_from_slice(link.as_bytes());
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        header[148..156].fill(b' ');
        let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        let mut out = header.to_vec();
        out.extend_from_slice(data);
        out.resize(512 + data.len().div_ceil(512) * 512, 0);
        out
    }

    fn write_tar(path: &Path, entries: &[Vec<u8>]) {
        let mut bytes: Vec<u8> = entries.concat();
        bytes.extend_from_slice(&[0u8; 1024]);
        fs::write(path, bytes).unwrap();
    }

    fn write_tar_gz(path: &Path, entries: &[Vec<u8>]) {
        let mut bytes: Vec<u8> = entries.concat();
        bytes.extend_from_slice(&[0u8; 1024]);
        let mut encoder = GzEncoder::new(File::create(path).unwrap(), Compression::default());
        encoder.write_all(&bytes).unwrap();
        encoder.finish().unwrap();
    }

    fn sample_tree(root: &Path) {
        fs::create_dir_all(root.join("top/sub/deeper")).unwrap();
        fs::create_dir_all(root.join("top/empty")).unwrap();
        fs::write(root.join("top/a.txt"), b"alpha").unwrap();
        fs::write(root.join("top/sub/b.bin"), [0u8, 1, 2, 255]).unwrap();
        fs::write(root.join("top/sub/deeper/c.txt"), b"").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("a.txt", root.join("top/link")).unwrap();
    }

    fn assert_sample_tree(root: &Path) {
        assert_eq!(fs::read(root.join("top/a.txt")).unwrap(), b"alpha");
        assert_eq!(
            fs::read(root.join("top/sub/b.bin")).unwrap(),
            [0u8, 1, 2, 255]
        );
        assert_eq!(fs::read(root.join("top/sub/deeper/c.txt")).unwrap(), b"");
        assert!(root.join("top/empty").is_dir());
        #[cfg(unix)]
        assert_eq!(
            fs::read_link(root.join("top/link")).unwrap(),
            Path::new("a.txt")
        );
    }

    #[test]
    fn every_format_round_trips_a_tree() {
        for format in [Format::Zip, Format::Tar, Format::TarGz] {
            let dir = scratch("round-trip");
            sample_tree(&dir.join("src"));
            let out = dir.join(format!("packed.{}", format.extension()));

            let made = compress(
                &[dir.join("src/top")],
                &out,
                format,
                ConflictPolicy::Abort,
                &mut go,
            )
            .unwrap();
            assert_eq!(made, Outcome::Completed);
            assert_eq!(Format::of(&out), Some(format));

            let summary = unpack(&out, &dir.join("out"), ConflictPolicy::Abort).unwrap();
            assert_sample_tree(&dir.join("out"));
            assert_eq!(summary.unsupported, 0, "{format:?}");
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn a_climbing_or_absolute_name_is_refused_and_writes_nothing() {
        for name in [
            "../evil",
            "a/../../evil",
            "/tmp/minuteman-evil",
            "ok/../../evil",
        ] {
            let dir = scratch("slip");
            let archive = dir.join("bad.tar");
            write_tar(&archive, &[tar_entry(name, b'0', "", b"pwned")]);

            let result = unpack(&archive, &dir.join("dest"), ConflictPolicy::Abort);
            assert!(
                matches!(result, Err(FileOpsError::UnsafeEntry { .. })),
                "{name}: {result:?}"
            );
            assert!(!dir.join("evil").exists(), "{name}");
            assert!(!Path::new("/tmp/minuteman-evil").exists(), "{name}");
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn a_climbing_zip_entry_is_refused() {
        let dir = scratch("zip-slip");
        let archive = dir.join("bad.zip");
        let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("../evil", options).unwrap();
        writer.write_all(b"pwned").unwrap();
        writer.start_file("..\\evil2", options).unwrap();
        writer.write_all(b"pwned").unwrap();
        writer.finish().unwrap();

        let result = unpack(&archive, &dir.join("dest"), ConflictPolicy::Abort);
        assert!(
            matches!(result, Err(FileOpsError::UnsafeEntry { .. })),
            "{result:?}"
        );
        assert!(!dir.join("evil").exists());
        assert!(!dir.join("evil2").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_control_character_in_a_name_is_refused() {
        let dir = scratch("control");
        let archive = dir.join("bad.tar");
        write_tar(&archive, &[tar_entry("esc\u{1b}[2J", b'0', "", b"x")]);
        let result = unpack(&archive, &dir.join("dest"), ConflictPolicy::Abort);
        assert!(
            matches!(result, Err(FileOpsError::UnsafeEntry { .. })),
            "{result:?}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_that_could_leave_the_destination_is_skipped_not_followed() {
        let dir = scratch("link-escape");
        let archive = dir.join("bad.tar");
        write_tar(
            &archive,
            &[
                tar_entry("abs", b'2', "/etc", b""),
                tar_entry("up", b'2', "..", b""),
                tar_entry("d", b'5', "", b""),
                tar_entry("d/hop", b'2', "..", b""),
                tar_entry("sneaky", b'2', "d/hop/..", b""),
                tar_entry("hard", b'1', "abs", b""),
                tar_entry("fine", b'2', "sub/x", b""),
            ],
        );

        let summary = unpack(&archive, &dir.join("dest"), ConflictPolicy::Abort).unwrap();
        assert_eq!(summary.unsupported, 5);
        assert_eq!(summary.symlinks, 1);
        assert!(fs::symlink_metadata(dir.join("dest/abs")).is_err());
        assert!(fs::symlink_metadata(dir.join("dest/sneaky")).is_err());
        assert!(fs::symlink_metadata(dir.join("dest/fine")).is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_entry_is_never_written_through_a_symlinked_directory() {
        let dir = scratch("link-dir");
        let outside = dir.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::create_dir(dir.join("dest")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("dest/link")).unwrap();
        let archive = dir.join("bad.tar");
        write_tar(&archive, &[tar_entry("link/planted", b'0', "", b"x")]);

        let result = unpack(&archive, &dir.join("dest"), ConflictPolicy::Overwrite);
        assert!(
            matches!(result, Err(FileOpsError::UnsafeEntry { .. })),
            "{result:?}"
        );
        assert!(!outside.join("planted").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn overwrite_replaces_a_symlink_instead_of_writing_through_it() {
        let dir = scratch("link-file");
        let victim = dir.join("victim");
        fs::write(&victim, b"keep").unwrap();
        fs::create_dir(dir.join("dest")).unwrap();
        std::os::unix::fs::symlink(&victim, dir.join("dest/f")).unwrap();
        let archive = dir.join("a.tar");
        write_tar(&archive, &[tar_entry("f", b'0', "", b"new")]);

        unpack(&archive, &dir.join("dest"), ConflictPolicy::Overwrite).unwrap();
        assert_eq!(fs::read(&victim).unwrap(), b"keep");
        assert_eq!(fs::read(dir.join("dest/f")).unwrap(), b"new");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn permissions_are_clamped() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("modes");
        let archive = dir.join("m.tar");
        let mut hostile = tar_entry("suid", b'0', "", b"x");
        hostile[100..107].copy_from_slice(b"0004777");
        let mut locked = tar_entry("locked", b'0', "", b"x");
        locked[100..107].copy_from_slice(b"0000000");
        for header in [&mut hostile, &mut locked] {
            header[148..156].fill(b' ');
            let sum: u32 = header[..512].iter().map(|&b| u32::from(b)).sum();
            header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        }
        write_tar(&archive, &[hostile, locked]);

        unpack(&archive, &dir.join("dest"), ConflictPolicy::Abort).unwrap();
        let mode = |name: &str| {
            fs::metadata(dir.join("dest").join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777
        };
        assert_eq!(mode("suid"), 0o755);
        assert_eq!(mode("locked"), 0o600);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_byte_limit_stops_a_bomb_and_removes_the_partial_file() {
        let dir = scratch("bomb");
        let archive = dir.join("bomb.tar.gz");
        write_tar_gz(
            &archive,
            &[tar_entry("zeros", b'0', "", &vec![0u8; 1 << 20])],
        );

        let limits = ExtractLimits {
            max_entries: 10,
            max_bytes: 4096,
        };
        let result = extract(
            &archive,
            &dir.join("dest"),
            ConflictPolicy::Abort,
            limits,
            &mut go,
        );
        assert!(
            matches!(result, Err(FileOpsError::ArchiveLimit { .. })),
            "{result:?}"
        );
        assert!(!dir.join("dest/zeros").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_of_exactly_the_limit_is_accepted() {
        let dir = scratch("exact");
        let archive = dir.join("a.tar");
        write_tar(&archive, &[tar_entry("f", b'0', "", &[7u8; 100])]);
        let limits = ExtractLimits {
            max_entries: 1,
            max_bytes: 100,
        };
        extract(
            &archive,
            &dir.join("dest"),
            ConflictPolicy::Abort,
            limits,
            &mut go,
        )
        .unwrap();
        assert_eq!(fs::read(dir.join("dest/f")).unwrap().len(), 100);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_entry_limit_stops_an_archive_of_too_many_entries() {
        let dir = scratch("entries");
        let archive = dir.join("many.tar");
        let entries: Vec<_> = (0..5)
            .map(|i| tar_entry(&format!("f{i}"), b'0', "", b"x"))
            .collect();
        write_tar(&archive, &entries);
        let limits = ExtractLimits {
            max_entries: 3,
            max_bytes: 1 << 20,
        };
        let result = extract(
            &archive,
            &dir.join("dest"),
            ConflictPolicy::Abort,
            limits,
            &mut go,
        );
        assert!(
            matches!(result, Err(FileOpsError::ArchiveLimit { .. })),
            "{result:?}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn conflict_policies_apply_to_existing_files() {
        let dir = scratch("conflict");
        let archive = dir.join("a.tar");
        write_tar(&archive, &[tar_entry("f", b'0', "", b"new")]);
        let dest = dir.join("dest");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("f"), b"old").unwrap();

        let abort = unpack(&archive, &dest, ConflictPolicy::Abort);
        assert!(matches!(
            abort,
            Err(FileOpsError::Vfs(VfsError::AlreadyExists(_)))
        ));
        assert_eq!(fs::read(dest.join("f")).unwrap(), b"old");

        let skip = unpack(&archive, &dest, ConflictPolicy::Skip).unwrap();
        assert_eq!(skip.skipped, 1);
        assert_eq!(fs::read(dest.join("f")).unwrap(), b"old");

        unpack(&archive, &dest, ConflictPolicy::Overwrite).unwrap();
        assert_eq!(fs::read(dest.join("f")).unwrap(), b"new");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn overwrite_never_deletes_a_directory_in_the_way_of_a_file() {
        let dir = scratch("dir-in-way");
        let archive = dir.join("a.tar");
        write_tar(&archive, &[tar_entry("f", b'0', "", b"new")]);
        let dest = dir.join("dest");
        fs::create_dir_all(dest.join("f")).unwrap();
        fs::write(dest.join("f/precious"), b"keep").unwrap();

        assert!(unpack(&archive, &dest, ConflictPolicy::Overwrite).is_err());
        assert_eq!(fs::read(dest.join("f/precious")).unwrap(), b"keep");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damaged_archive_is_an_error_not_a_panic() {
        let dir = scratch("damaged");
        for name in ["x.zip", "x.tar", "x.tar.gz"] {
            fs::write(dir.join(name), b"this is not an archive at all").unwrap();
            let result = unpack(&dir.join(name), &dir.join("dest"), ConflictPolicy::Abort);
            assert!(result.is_err(), "{name}");
        }
        fs::write(dir.join("x.rar"), b"Rar!").unwrap();
        assert!(matches!(
            unpack(&dir.join("x.rar"), &dir.join("dest"), ConflictPolicy::Abort),
            Err(FileOpsError::UnsupportedArchive(_))
        ));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cancelling_an_extraction_stops_with_cancelled() {
        let dir = scratch("cancel-extract");
        let archive = dir.join("a.tar");
        write_tar(
            &archive,
            &[
                tar_entry("a", b'0', "", b"1"),
                tar_entry("b", b'0', "", b"2"),
            ],
        );
        let result = extract(
            &archive,
            &dir.join("dest"),
            ConflictPolicy::Abort,
            ExtractLimits::default(),
            &mut |_| ControlFlow::Break(()),
        );
        assert!(matches!(result, Err(FileOpsError::Cancelled)));
        assert!(!dir.join("dest/b").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn compress_refuses_an_output_inside_a_source() {
        let dir = scratch("recursive");
        sample_tree(&dir);
        let result = compress(
            &[dir.join("top")],
            &dir.join("top/inside.zip"),
            Format::Zip,
            ConflictPolicy::Abort,
            &mut go,
        );
        assert!(matches!(
            result,
            Err(FileOpsError::RecursiveDestination { .. })
        ));
        assert!(matches!(
            compress(
                &[],
                &dir.join("x.zip"),
                Format::Zip,
                ConflictPolicy::Abort,
                &mut go
            ),
            Err(FileOpsError::NothingToArchive)
        ));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn compress_honours_the_policy_for_an_existing_output() {
        let dir = scratch("compress-conflict");
        fs::write(dir.join("f"), b"data").unwrap();
        let out = dir.join("out.tar");
        fs::write(&out, b"precious").unwrap();
        let sources = [dir.join("f")];

        assert!(matches!(
            compress(&sources, &out, Format::Tar, ConflictPolicy::Abort, &mut go),
            Err(FileOpsError::Vfs(VfsError::AlreadyExists(_)))
        ));
        assert_eq!(
            compress(&sources, &out, Format::Tar, ConflictPolicy::Skip, &mut go).unwrap(),
            Outcome::Skipped
        );
        assert_eq!(fs::read(&out).unwrap(), b"precious");

        compress(
            &sources,
            &out,
            Format::Tar,
            ConflictPolicy::Overwrite,
            &mut go,
        )
        .unwrap();
        assert!(fs::metadata(&out).unwrap().len() >= 1024);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_cancelled_compression_leaves_no_archive_and_no_partial_file() {
        let dir = scratch("cancel-compress");
        sample_tree(&dir);
        let out = dir.join("out.zip");
        let result = compress(
            &[dir.join("top")],
            &out,
            Format::Zip,
            ConflictPolicy::Abort,
            &mut |_| ControlFlow::Break(()),
        );
        assert!(matches!(result, Err(FileOpsError::Cancelled)));
        assert!(!out.exists());
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".partial"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_higher_level_makes_a_smaller_archive_that_still_round_trips() {
        let dir = scratch("levels");
        let text: String = (0..20_000).map(|i| format!("line {}\n", i % 97)).collect();
        fs::write(dir.join("big.txt"), &text).unwrap();
        for format in [Format::Zip, Format::TarGz] {
            let size = |level: Level| {
                let out = dir.join(format!("{level:?}.{}", format.extension()));
                compress_with_level(
                    &[dir.join("big.txt")],
                    &out,
                    format,
                    level,
                    ConflictPolicy::Abort,
                    &mut go,
                )
                .unwrap();
                let dest = dir.join(format!("out-{level:?}-{format:?}"));
                unpack(&out, &dest, ConflictPolicy::Abort).unwrap();
                assert_eq!(fs::read_to_string(dest.join("big.txt")).unwrap(), text);
                fs::metadata(&out).unwrap().len()
            };
            let (fast, best) = (size(Level::Fast), size(Level::Best));
            assert!(best <= fast, "{format:?}: best {best} > fast {fast}");
            assert!(best < text.len() as u64 / 4, "{format:?}: {best}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_stem_drops_the_archive_extension() {
        let stem = |name: &str| Format::stem_of(Path::new(name));
        assert_eq!(stem("photos.tar.gz").as_deref(), Some("photos"));
        assert_eq!(stem("/x/y/Photos.TGZ").as_deref(), Some("Photos"));
        assert_eq!(stem("a.b.zip").as_deref(), Some("a.b"));
        assert_eq!(stem("notes.txt"), None);
        assert_eq!(stem(".zip"), None);
    }

    #[test]
    fn a_dot_entry_is_the_destination_itself() {
        assert_eq!(SafePath::try_new(Path::new("./")).unwrap(), None);
        assert_eq!(SafePath::try_new(Path::new(".")).unwrap(), None);
        assert_eq!(
            SafePath::try_new(Path::new("./a/./b"))
                .unwrap()
                .unwrap()
                .as_path(),
            Path::new("a/b")
        );
    }

    proptest! {
        /// Whatever an archive names an entry, a `SafePath` built from it stays under the
        /// destination: only ordinary names, and joining it never climbs out.
        #[test]
        fn a_safe_path_never_escapes(
            parts in prop::collection::vec(
                prop_oneof![
                    Just("..".to_string()),
                    Just(".".to_string()),
                    Just(String::new()),
                    "[a-z]{1,4}",
                    "[a-z]{1,3}\\\\[a-z]{1,3}",
                ],
                0..8,
            ),
            absolute in any::<bool>(),
        ) {
            let raw = format!("{}{}", if absolute { "/" } else { "" }, parts.join("/"));
            if let Ok(Some(safe)) = SafePath::try_new(Path::new(&raw)) {
                prop_assert!(safe.as_path().is_relative());
                prop_assert!(safe.as_path().components().all(|c| matches!(c, Component::Normal(_))));
                let dest = Path::new("/dest");
                prop_assert!(dest.join(safe.as_path()).starts_with(dest));
            }
        }

        #[test]
        fn tar_gz_round_trips_arbitrary_files(
            files in prop::collection::btree_map("[a-z]{1,8}", prop::collection::vec(any::<u8>(), 0..200), 1..6),
        ) {
            let dir = scratch("prop-round-trip");
            let root = dir.join("root");
            fs::create_dir_all(&root).unwrap();
            for (name, bytes) in &files {
                fs::write(root.join(name), bytes).unwrap();
            }
            let out = dir.join("p.tar.gz");
            compress(&[root], &out, Format::TarGz, ConflictPolicy::Abort, &mut go).unwrap();
            unpack(&out, &dir.join("out"), ConflictPolicy::Abort).unwrap();
            for (name, bytes) in &files {
                prop_assert_eq!(&fs::read(dir.join("out/root").join(name)).unwrap(), bytes);
            }
            fs::remove_dir_all(&dir).unwrap();
        }
    }
}
