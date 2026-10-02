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

//! Browses an archive as a folder: a [`Vfs`] that wraps another and serves `/x/a.zip!/dir/f` out
//! of `/x/a.zip`, passing every other path to the one underneath untouched.
//!
//! Because it is a `Vfs`, everything that already goes through one works inside an archive with no
//! code of its own: browsing, previews, search, the disk usage view, and copying a file or a
//! whole folder out. The archive can sit on the local disk or on a remote machine, since it is
//! read through whatever backend it wraps.
//!
//! Archives are read-only: every way of changing one reports [`VfsError::Unsupported`]. Zip, tar
//! and tar.gz are served; an archive inside an archive is just a file of the outer one.

mod address;
mod index;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use shared::{DirEntryInfo, FileKind, Metadata, ReadSeek, ScanEntry, Vfs, VfsError, WriteSeek};

pub use address::{Format, Mount, Route, is_archive, is_inside, route};
use index::{Content, Index, Node, Stamp};

const READ_ONLY: &str = "an archive is read-only";
/// Archives whose index is kept, so moving around inside one does not read it again.
const CACHED_ARCHIVES: usize = 8;

pub struct ArchiveVfs {
    inner: Arc<dyn Vfs>,
    // Most recently used first.
    cache: Mutex<Vec<(PathBuf, Arc<Index>)>>,
}

fn read_only<T>() -> Result<T, VfsError> {
    Err(VfsError::Unsupported(READ_ONLY))
}

impl ArchiveVfs {
    pub fn new(inner: Arc<dyn Vfs>) -> Self {
        Self {
            inner,
            cache: Mutex::new(Vec::new()),
        }
    }

    /// The table of contents of the archive `mount` names, read now or reused if the file has not
    /// changed since it was read.
    fn index(&self, mount: &Mount) -> Result<Arc<Index>, VfsError> {
        // Asked every time, not only when building: it is what notices the archive changing.
        let file = self.inner.metadata(&mount.archive)?;
        if file.kind != FileKind::File {
            return Err(VfsError::NotFound(mount.root.clone()));
        }
        let stamp = Stamp {
            len: file.len,
            modified: file.modified,
        };
        {
            let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((_, index)) = cache
                .iter()
                .find(|(path, index)| *path == mount.archive && index.stamp == stamp)
            {
                return Ok(Arc::clone(index));
            }
        }
        let reader = self.inner.open_read(&mount.archive)?;
        let built = Index::build(reader, mount.format, stamp).map_err(|source| VfsError::Io {
            path: mount.root.clone(),
            source,
        })?;
        let index = Arc::new(built);
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.retain(|(path, _)| *path != mount.archive);
        cache.insert(0, (mount.archive.clone(), Arc::clone(&index)));
        cache.truncate(CACHED_ARCHIVES);
        Ok(index)
    }

    /// Splits `path` and finds its node. `Ok(None)` is a path that is not in an archive.
    fn locate(
        &self,
        path: &Path,
        follow_last: bool,
    ) -> Result<Option<(Mount, Arc<Index>, usize)>, VfsError> {
        let mount = match route(path) {
            Route::Outside => return Ok(None),
            Route::Escapes => return Err(VfsError::NotFound(path.to_path_buf())),
            Route::Inside(mount) => mount,
        };
        let index = self.index(&mount)?;
        let at = index
            .resolve(&mount.entry, follow_last)
            .ok_or_else(|| VfsError::NotFound(path.to_path_buf()))?;
        Ok(Some((mount, index, at)))
    }

    /// The key of `name` in the folder whose key is `folder`.
    fn key_of(folder: &str, name: &str) -> String {
        if folder.is_empty() {
            name.to_owned()
        } else {
            format!("{folder}/{name}")
        }
    }
}

fn kind_of(node: &Node) -> FileKind {
    match node.content {
        Content::Dir => FileKind::Dir,
        Content::File(_) => FileKind::File,
        Content::Symlink(_) => FileKind::Symlink,
        Content::Other => FileKind::Other,
    }
}

fn metadata_of(node: &Node) -> Metadata {
    Metadata {
        kind: kind_of(node),
        len: node.size,
        allocated: None,
        mode: node.mode,
        uid: None,
        gid: None,
        nlink: None,
        modified: node.modified,
        accessed: None,
        created: None,
        device: None,
        inode: None,
    }
}

impl Vfs for ArchiveVfs {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, VfsError> {
        let Some((mount, index, at)) = self.locate(path, true)? else {
            return self.inner.list_dir(path);
        };
        if !index.node(at).is_folder() {
            return Err(VfsError::NotADirectory(path.to_path_buf()));
        }
        Ok(index
            .node(at)
            .children
            .iter()
            .map(|&child| {
                let node = index.node(child);
                let key = Self::key_of(&mount.entry, &node.name);
                // A link is listed as what it leads to, so one to a folder opens like a folder;
                // a broken one is an empty file.
                let (is_dir, size, modified, mode) = match node.content {
                    Content::Symlink(_) => match index.resolve(&key, true) {
                        Some(target) => {
                            let target = index.node(target);
                            (
                                target.is_folder(),
                                target.size,
                                target.modified,
                                target.mode,
                            )
                        }
                        None => (false, 0, node.modified, node.mode),
                    },
                    _ => (node.is_folder(), node.size, node.modified, node.mode),
                };
                DirEntryInfo {
                    name: node.name.clone(),
                    path: mount.root.join(&key),
                    is_dir,
                    size: if is_dir { 0 } else { size },
                    modified,
                    mode,
                }
            })
            .collect())
    }

    fn is_dir(&self, path: &Path) -> bool {
        match self.locate(path, true) {
            Ok(Some((_, index, at))) => index.node(at).is_folder(),
            Ok(None) => self.inner.is_dir(path),
            Err(_) => false,
        }
    }

    fn exists(&self, path: &Path) -> bool {
        match self.locate(path, true) {
            Ok(Some(_)) => true,
            Ok(None) => self.inner.exists(path),
            Err(_) => false,
        }
    }

    fn metadata(&self, path: &Path) -> Result<Metadata, VfsError> {
        match self.locate(path, true)? {
            Some((_, index, at)) => Ok(metadata_of(index.node(at))),
            None => self.inner.metadata(path),
        }
    }

    fn symlink_metadata(&self, path: &Path) -> Result<Metadata, VfsError> {
        match self.locate(path, false)? {
            Some((_, index, at)) => Ok(metadata_of(index.node(at))),
            None => self.inner.symlink_metadata(path),
        }
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, VfsError> {
        match self.locate(path, false)? {
            Some((_, index, at)) => match &index.node(at).content {
                Content::Symlink(target) => Ok(PathBuf::from(target)),
                _ => Err(VfsError::Io {
                    path: path.to_path_buf(),
                    source: io::Error::new(io::ErrorKind::InvalidInput, "not a symbolic link"),
                }),
            },
            None => self.inner.read_link(path),
        }
    }

    fn open_read(&self, path: &Path) -> Result<Box<dyn ReadSeek>, VfsError> {
        let Some((mount, index, at)) = self.locate(path, true)? else {
            return self.inner.open_read(path);
        };
        let node = index.node(at);
        let Content::File(source) = node.content else {
            return Err(VfsError::Io {
                path: path.to_path_buf(),
                source: io::Error::new(
                    if node.is_folder() {
                        io::ErrorKind::IsADirectory
                    } else {
                        io::ErrorKind::InvalidInput
                    },
                    "not a file that can be read",
                ),
            });
        };
        index::open_entry(self.inner.as_ref(), &mount.archive, source, node.size).map_err(
            |source| VfsError::Io {
                path: path.to_path_buf(),
                source,
            },
        )
    }

    fn scan_dir(&self, path: &Path) -> Result<Vec<ScanEntry>, VfsError> {
        let Some((mount, index, at)) = self.locate(path, true)? else {
            return self.inner.scan_dir(path);
        };
        if !index.node(at).is_folder() {
            return Err(VfsError::NotADirectory(path.to_path_buf()));
        }
        Ok(index
            .node(at)
            .children
            .iter()
            .map(|&child| {
                let node = index.node(child);
                ScanEntry {
                    path: mount.root.join(Self::key_of(&mount.entry, &node.name)),
                    name: node.name.clone(),
                    metadata: Some(metadata_of(node)),
                }
            })
            .collect())
    }

    fn local_path(&self, path: &Path) -> Option<PathBuf> {
        if is_inside(path) {
            // What is in an archive is not a file on any disk.
            return None;
        }
        self.inner.local_path(path)
    }

    fn resolve_typed(&self, typed: &Path, current: &Path) -> PathBuf {
        self.inner.resolve_typed(typed, current)
    }

    fn create_write(&self, path: &Path) -> Result<Box<dyn WriteSeek>, VfsError> {
        if is_inside(path) {
            return read_only();
        }
        self.inner.create_write(path)
    }

    fn create_symlink(&self, target: &Path, link: &Path) -> Result<(), VfsError> {
        if is_inside(link) {
            return read_only();
        }
        self.inner.create_symlink(target, link)
    }

    fn set_mode(&self, path: &Path, mode: u32) -> Result<(), VfsError> {
        if is_inside(path) {
            return read_only();
        }
        self.inner.set_mode(path, mode)
    }

    fn replace(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        if is_inside(src) || is_inside(dst) {
            return read_only();
        }
        self.inner.replace(src, dst)
    }

    fn create_dir(&self, path: &Path) -> Result<(), VfsError> {
        if is_inside(path) {
            return read_only();
        }
        self.inner.create_dir(path)
    }

    fn create_file(&self, path: &Path) -> Result<(), VfsError> {
        if is_inside(path) {
            return read_only();
        }
        self.inner.create_file(path)
    }

    fn touch(&self, path: &Path) -> Result<(), VfsError> {
        if is_inside(path) {
            return read_only();
        }
        self.inner.touch(path)
    }

    fn copy_file(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        if is_inside(dst) {
            return read_only();
        }
        if !is_inside(src) {
            return self.inner.copy_file(src, dst);
        }
        // Out of an archive into a folder: read the entry and write it as a new file.
        if self.inner.symlink_metadata(dst).is_ok() {
            return Err(VfsError::AlreadyExists(dst.to_path_buf()));
        }
        let meta = self.metadata(src)?;
        let mut from = self.open_read(src)?;
        let mut to = self.inner.create_write(dst)?;
        let copied = io::copy(&mut from, &mut to).and_then(|_| to.flush());
        drop(to);
        if let Err(source) = copied {
            // A half-written copy would otherwise look like the real file.
            let _ = self.inner.remove_file(dst);
            return Err(VfsError::Io {
                path: dst.to_path_buf(),
                source,
            });
        }
        if let Some(mode) = meta.mode {
            // Best effort: the destination may have no permissions to set.
            let _ = self.inner.set_mode(dst, mode & 0o7777);
        }
        Ok(())
    }

    fn rename(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        if is_inside(src) || is_inside(dst) {
            return read_only();
        }
        self.inner.rename(src, dst)
    }

    fn remove_file(&self, path: &Path) -> Result<(), VfsError> {
        if is_inside(path) {
            return read_only();
        }
        self.inner.remove_file(path)
    }

    fn remove_dir_all(&self, path: &Path) -> Result<(), VfsError> {
        if is_inside(path) {
            return read_only();
        }
        self.inner.remove_dir_all(path)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::time::{Duration, UNIX_EPOCH};

    use proptest::prelude::*;
    use shared::MemVfs;

    use super::*;

    /// One thing to put in a test archive.
    #[derive(Clone, Debug)]
    enum Item {
        Dir(String),
        File(String, Vec<u8>),
        Link(String, String),
    }

    fn tar_bytes(items: &[Item]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for item in items {
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o644);
            header.set_mtime(1_700_000_000);
            match item {
                Item::Dir(name) => {
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_size(0);
                    header.set_mode(0o755);
                    builder.append_data(&mut header, name, io::empty()).unwrap();
                }
                Item::File(name, bytes) => {
                    header.set_size(bytes.len() as u64);
                    // The tar writer refuses a `..` in a name, which is exactly what a hostile
                    // archive would carry, so the name is put in the header's bytes directly.
                    let raw = header.as_old_mut();
                    raw.name[..name.len()].copy_from_slice(name.as_bytes());
                    header.set_cksum();
                    builder.append(&header, bytes.as_slice()).unwrap();
                }
                Item::Link(name, target) => {
                    header.set_entry_type(tar::EntryType::Symlink);
                    header.set_size(0);
                    builder.append_link(&mut header, name, target).unwrap();
                }
            }
        }
        builder.into_inner().unwrap()
    }

    fn zip_bytes(items: &[Item]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o640)
            .last_modified_time(
                zip::DateTime::from_date_and_time(2024, 2, 29, 12, 30, 10).unwrap(),
            );
        for item in items {
            match item {
                Item::Dir(name) => writer.add_directory(name.as_str(), options).unwrap(),
                Item::File(name, bytes) => {
                    writer.start_file(name.as_str(), options).unwrap();
                    writer.write_all(bytes).unwrap();
                }
                // A zip has no link entries here; see `index_zip`.
                Item::Link(..) => {}
            }
        }
        writer.finish().unwrap().into_inner()
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn file(name: &str, bytes: &[u8]) -> Item {
        Item::File(name.to_owned(), bytes.to_vec())
    }

    /// The same archive in each format, as `(file name, bytes)`.
    fn every_format(items: &[Item]) -> Vec<(&'static str, Vec<u8>)> {
        let tar = tar_bytes(items);
        vec![
            ("a.zip", zip_bytes(items)),
            ("a.tar", tar.clone()),
            ("a.tar.gz", gzip(&tar)),
        ]
    }

    fn vfs_with(name: &str, bytes: Vec<u8>) -> (ArchiveVfs, Arc<MemVfs>, PathBuf) {
        let mem = Arc::new(MemVfs::new());
        mem.add_dir("/x");
        mem.add_file(format!("/x/{name}"), bytes);
        let vfs = ArchiveVfs::new(mem.clone());
        (vfs, mem, PathBuf::from(format!("/x/{name}!")))
    }

    fn read(vfs: &ArchiveVfs, path: &Path) -> Vec<u8> {
        let mut bytes = Vec::new();
        vfs.open_read(path)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    }

    /// Every file under `dir`, as a path-to-bytes map, found only through the trait.
    fn walk(vfs: &ArchiveVfs, dir: &Path, found: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in vfs.list_dir(dir).unwrap() {
            if entry.is_dir {
                walk(vfs, &entry.path, found);
            } else {
                found.insert(entry.path.clone(), read(vfs, &entry.path));
            }
        }
    }

    fn names(vfs: &ArchiveVfs, dir: &Path) -> Vec<String> {
        vfs.list_dir(dir)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]

        /// Whatever files an archive holds, walking it as folders gives back exactly those paths
        /// with exactly those bytes, in every format, with the folders in between made up.
        #[test]
        fn walking_an_archive_gives_back_what_was_put_in(
            files in proptest::collection::btree_map(
                "[a-z]{1,5}(/[a-z]{1,5}){0,2}",
                proptest::collection::vec(any::<u8>(), 0..300),
                0..10,
            ),
        ) {
            // A name that is a folder for another file cannot also be a file.
            let names: Vec<&String> = files.keys().collect();
            let clash = names.iter().any(|a| names.iter().any(|b| b.starts_with(&format!("{a}/"))));
            prop_assume!(!clash);
            let items: Vec<Item> = files.iter().map(|(n, b)| file(n, b)).collect();
            for (name, bytes) in every_format(&items) {
                let (vfs, _mem, root) = vfs_with(name, bytes);
                let mut found = BTreeMap::new();
                walk(&vfs, &root, &mut found);
                let expected: BTreeMap<PathBuf, Vec<u8>> =
                    files.iter().map(|(n, b)| (root.join(n), b.clone())).collect();
                prop_assert_eq!(found, expected, "{}", name);
            }
        }

        /// Reading any range of a window onto a buffer matches the same range of the buffer.
        #[test]
        fn a_section_reads_exactly_its_window(
            data in proptest::collection::vec(any::<u8>(), 1..200),
            start in 0usize..100,
            len in 0usize..150,
            seek_to in 0u64..250,
        ) {
            let start = start.min(data.len());
            let len = len.min(data.len() - start);
            let mut section = index::Section::for_test(
                Box::new(io::Cursor::new(data.clone())), start as u64, len as u64);
            let window = &data[start..start + len];
            let mut all = Vec::new();
            section.read_to_end(&mut all).unwrap();
            prop_assert_eq!(&all[..], window);
            section.seek(SeekFrom::Start(seek_to)).unwrap();
            let mut rest = Vec::new();
            section.read_to_end(&mut rest).unwrap();
            let from = (seek_to as usize).min(len);
            prop_assert_eq!(&rest[..], &window[from..]);
        }
    }

    #[test]
    fn folders_list_first_then_names_ignoring_case_and_missing_parents_are_made() {
        let items = [
            file("b/z.txt", b"1"),
            file("A.txt", b"2"),
            file("c.txt", b"3"),
            file("Z/y", b"4"),
        ];
        for (name, bytes) in every_format(&items) {
            let (vfs, _mem, root) = vfs_with(name, bytes);
            assert_eq!(names(&vfs, &root), ["b", "Z", "A.txt", "c.txt"], "{name}");
            assert!(
                vfs.is_dir(&root.join("b")),
                "{name}: a parent no entry names is still a folder"
            );
            assert_eq!(names(&vfs, &root.join("b")), ["z.txt"], "{name}");
        }
    }

    #[test]
    fn a_listing_reports_size_kind_and_the_path_to_use() {
        let items = [Item::Dir("d/".into()), file("d/f.txt", b"hello")];
        for (name, bytes) in every_format(&items) {
            let (vfs, _mem, root) = vfs_with(name, bytes);
            let listed = vfs.list_dir(&root.join("d")).unwrap();
            assert_eq!(listed.len(), 1, "{name}");
            assert_eq!(listed[0].path, root.join("d/f.txt"), "{name}");
            assert_eq!(listed[0].size, 5, "{name}");
            assert!(!listed[0].is_dir, "{name}");
            let top = vfs.list_dir(&root).unwrap();
            assert!(top[0].is_dir && top[0].size == 0, "{name}");
            assert!(
                matches!(
                    vfs.list_dir(&root.join("d/f.txt")),
                    Err(VfsError::NotADirectory(_))
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn opened_files_read_and_seek_in_every_format() {
        let data: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        let items = [file("pad.bin", &[7; 700]), file("d/data.bin", &data)];
        for (name, bytes) in every_format(&items) {
            let (vfs, _mem, root) = vfs_with(name, bytes);
            let path = root.join("d/data.bin");
            assert_eq!(read(&vfs, &path), data, "{name}");
            let mut open = vfs.open_read(&path).unwrap();
            open.seek(SeekFrom::Start(1000)).unwrap();
            let mut chunk = [0u8; 16];
            open.read_exact(&mut chunk).unwrap();
            assert_eq!(chunk, data[1000..1016], "{name}");
            open.seek(SeekFrom::End(-4)).unwrap();
            let mut tail = Vec::new();
            open.read_to_end(&mut tail).unwrap();
            assert_eq!(tail, data[data.len() - 4..], "{name}");
            assert!(
                vfs.open_read(&root.join("d")).is_err(),
                "{name}: a folder cannot be read"
            );
            assert!(matches!(
                vfs.open_read(&root.join("nope")),
                Err(VfsError::NotFound(_))
            ));
        }
    }

    #[test]
    fn metadata_carries_size_time_and_mode() {
        let items = [file("f.txt", b"12345")];
        let (vfs, _mem, root) = vfs_with("a.zip", zip_bytes(&items));
        let meta = vfs.metadata(&root.join("f.txt")).unwrap();
        assert_eq!((meta.kind, meta.len), (FileKind::File, 5));
        assert_eq!(meta.mode, Some(0o100_640));
        // 2024-02-29 12:30:10 UTC, a leap day.
        assert_eq!(
            meta.modified,
            Some(UNIX_EPOCH + Duration::from_secs(1_709_209_810))
        );
        assert_eq!(vfs.metadata(&root).unwrap().kind, FileKind::Dir);

        let (vfs, _mem, root) = vfs_with("a.tar", tar_bytes(&items));
        let meta = vfs.metadata(&root.join("f.txt")).unwrap();
        assert_eq!(meta.mode, Some(0o100_644));
        assert_eq!(
            meta.modified,
            Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000))
        );
    }

    #[test]
    fn links_inside_a_tar_lead_only_inside_it() {
        let items = [
            file("real/f.txt", b"abcd"),
            Item::Link("to_dir".into(), "real".into()),
            Item::Link("real/up".into(), "../real/f.txt".into()),
            Item::Link("dangling".into(), "missing".into()),
            Item::Link("outside".into(), "../../etc/passwd".into()),
            Item::Link("absolute".into(), "/etc/passwd".into()),
            Item::Link("loop_a".into(), "loop_b".into()),
            Item::Link("loop_b".into(), "loop_a".into()),
        ];
        for name in ["a.tar", "a.tar.gz"] {
            let bytes = if name == "a.tar" {
                tar_bytes(&items)
            } else {
                gzip(&tar_bytes(&items))
            };
            let (vfs, _mem, root) = vfs_with(name, bytes);
            let listed = vfs.list_dir(&root).unwrap();
            let find = |n: &str| listed.iter().find(|e| e.name == n).unwrap();
            assert!(
                find("to_dir").is_dir,
                "{name}: a link to a folder lists as a folder"
            );
            assert!(
                !find("dangling").is_dir && find("dangling").size == 0,
                "{name}"
            );

            assert_eq!(read(&vfs, &root.join("to_dir/f.txt")), b"abcd", "{name}");
            assert_eq!(read(&vfs, &root.join("real/up")), b"abcd", "{name}");
            assert_eq!(
                vfs.symlink_metadata(&root.join("to_dir")).unwrap().kind,
                FileKind::Symlink
            );
            assert_eq!(
                vfs.metadata(&root.join("to_dir")).unwrap().kind,
                FileKind::Dir
            );
            assert_eq!(
                vfs.read_link(&root.join("to_dir")).unwrap(),
                Path::new("real")
            );
            assert!(vfs.read_link(&root.join("real")).is_err());
            for dead in ["dangling", "outside", "absolute", "loop_a"] {
                assert!(vfs.open_read(&root.join(dead)).is_err(), "{name}: {dead}");
                assert!(!vfs.exists(&root.join(dead)), "{name}: {dead}");
                assert!(
                    vfs.symlink_metadata(&root.join(dead)).is_ok(),
                    "{name}: {dead} itself exists"
                );
            }
        }
    }

    #[test]
    fn an_entry_climbing_out_is_left_out_and_unsafe_names_are_defused() {
        let items = [
            file("../evil.txt", b"x"),
            file("ok/../../evil2.txt", b"x"),
            file("ok.txt", b"y"),
            file("esc\u{1b}]0;pwned\u{7}.txt", b"z"),
            file("rtl\u{202e}txt.exe", b"z"),
        ];
        for (name, bytes) in every_format(&items) {
            let (vfs, _mem, root) = vfs_with(name, bytes);
            let listed = names(&vfs, &root);
            assert_eq!(listed.len(), 3, "{name}: {listed:?}");
            assert!(listed.contains(&"ok.txt".to_owned()), "{name}");
            for shown in &listed {
                assert!(
                    !shown.chars().any(|c| c.is_control() || c == '\u{202e}'),
                    "{name}: {shown:?} is not safe to draw"
                );
            }
            assert!(!vfs.exists(&root.join("../evil.txt")), "{name}");
        }
    }

    #[test]
    fn a_damaged_archive_is_an_error_not_an_empty_folder() {
        for name in ["a.zip", "a.tar.gz"] {
            let (vfs, _mem, root) = vfs_with(name, b"this is not an archive at all".repeat(40));
            assert!(
                matches!(vfs.list_dir(&root), Err(VfsError::Io { .. })),
                "{name}"
            );
        }
        let (vfs, _mem, root) = vfs_with("a.zip", Vec::new());
        assert!(vfs.list_dir(&root).is_err());
        let (vfs, _mem, root) = vfs_with("a.zip", vec![0]);
        assert!(vfs.list_dir(&root).is_err());
    }

    #[test]
    fn a_missing_archive_or_a_folder_named_like_one_is_not_found() {
        let mem = Arc::new(MemVfs::new());
        mem.add_dir("/x/dir.zip");
        let vfs = ArchiveVfs::new(mem);
        assert!(matches!(
            vfs.list_dir(Path::new("/x/gone.zip!")),
            Err(VfsError::NotFound(_))
        ));
        assert!(matches!(
            vfs.list_dir(Path::new("/x/dir.zip!")),
            Err(VfsError::NotFound(_))
        ));
        assert!(matches!(
            vfs.list_dir(Path::new("/x/gone.zip!/..")),
            Err(VfsError::NotFound(_))
        ));
    }

    #[test]
    fn every_way_of_changing_an_archive_is_refused_and_nothing_else_is() {
        let (vfs, mem, root) = vfs_with("a.zip", zip_bytes(&[file("f", b"1")]));
        let inside = root.join("new");
        let refused = |result: Result<(), VfsError>| {
            assert!(
                matches!(result, Err(VfsError::Unsupported(_))),
                "{result:?}"
            );
        };
        refused(vfs.create_dir(&inside));
        refused(vfs.create_file(&inside));
        refused(vfs.touch(&inside));
        refused(vfs.remove_file(&root.join("f")));
        refused(vfs.remove_dir_all(&root));
        refused(vfs.rename(&root.join("f"), Path::new("/x/out")));
        refused(vfs.rename(Path::new("/x/a.zip"), &inside));
        refused(vfs.replace(Path::new("/x/a.zip"), &inside));
        refused(vfs.copy_file(Path::new("/x/a.zip"), &inside));
        refused(vfs.set_mode(&root.join("f"), 0o600));
        refused(vfs.create_symlink(Path::new("t"), &inside));
        assert!(matches!(
            vfs.create_write(&inside),
            Err(VfsError::Unsupported(_))
        ));

        // The archive file beside them is an ordinary file.
        vfs.create_dir(Path::new("/x/made")).unwrap();
        assert!(mem.exists(Path::new("/x/made")));
        vfs.rename(Path::new("/x/made"), Path::new("/x/moved"))
            .unwrap();
        assert!(vfs.is_dir(Path::new("/x/moved")));
        assert_eq!(vfs.local_path(&root), None);
    }

    #[test]
    fn copying_out_makes_an_identical_new_file_and_never_overwrites() {
        let data: Vec<u8> = (0..250u8).collect();
        for (name, bytes) in every_format(&[file("d/f.bin", &data)]) {
            let (vfs, mem, root) = vfs_with(name, bytes);
            let out = Path::new("/x/copy.bin");
            vfs.copy_file(&root.join("d/f.bin"), out).unwrap();
            assert_eq!(read(&vfs, out), data, "{name}");
            assert!(mem.exists(out));
            assert!(
                matches!(
                    vfs.copy_file(&root.join("d/f.bin"), out),
                    Err(VfsError::AlreadyExists(_))
                ),
                "{name}"
            );
            assert!(
                matches!(
                    vfs.copy_file(&root.join("nope"), Path::new("/x/other")),
                    Err(VfsError::NotFound(_))
                ),
                "{name}"
            );
            assert!(
                !mem.exists(Path::new("/x/other")),
                "{name}: a failed copy leaves nothing"
            );
        }
    }

    #[test]
    fn an_archive_that_changed_on_disk_is_read_again() {
        let (vfs, mem, root) = vfs_with("a.zip", zip_bytes(&[file("one", b"1")]));
        assert_eq!(names(&vfs, &root), ["one"]);
        mem.add_file(
            "/x/a.zip",
            zip_bytes(&[file("one", b"1"), file("two", b"22")]),
        );
        assert_eq!(names(&vfs, &root), ["one", "two"]);
    }

    #[test]
    fn scanning_describes_each_entry_itself() {
        let items = [
            file("f.txt", b"abc"),
            Item::Link("l".into(), "f.txt".into()),
        ];
        let (vfs, _mem, root) = vfs_with("a.tar", tar_bytes(&items));
        let scanned = vfs.scan_dir(&root).unwrap();
        let link = scanned.iter().find(|e| e.name == "l").unwrap();
        assert_eq!(link.metadata.as_ref().unwrap().kind, FileKind::Symlink);
        let plain = scanned.iter().find(|e| e.name == "f.txt").unwrap();
        assert_eq!(plain.metadata.as_ref().unwrap().len, 3);
        assert_eq!(plain.path, root.join("f.txt"));
    }

    #[test]
    fn paths_that_are_not_in_an_archive_pass_straight_through() {
        let mem = Arc::new(MemVfs::new());
        mem.add_file("/x/plain.txt", b"hi".to_vec());
        mem.add_file("/x/a.zip", b"not read unless asked".to_vec());
        let vfs = ArchiveVfs::new(mem);
        assert_eq!(names(&vfs, Path::new("/x")), ["a.zip", "plain.txt"]);
        assert_eq!(read(&vfs, Path::new("/x/plain.txt")), b"hi");
        // The archive itself is a file here, so a bad one does not break listing its folder.
        assert!(!vfs.is_dir(Path::new("/x/a.zip")));
    }

    #[test]
    fn civil_dates_become_the_right_seconds() {
        assert_eq!(index::civil_seconds_for_test(1970, 1, 1), 0);
        assert_eq!(index::civil_seconds_for_test(2000, 1, 1), 946_684_800);
        assert_eq!(index::civil_seconds_for_test(1980, 1, 1), 315_532_800);
        assert_eq!(index::civil_seconds_for_test(2024, 2, 29), 1_709_164_800);
        assert_eq!(index::civil_seconds_for_test(2100, 3, 1), 4_107_542_400);
    }

    /// Indexing a big zip, over the render thread's budget if it ever stops being cheap. Run with
    /// `cargo test -p vfs_archive --release -- --ignored --nocapture`.
    #[test]
    #[ignore = "benchmark"]
    fn indexing_fifty_thousand_entries_is_fast() {
        let items: Vec<Item> = (0..50_000)
            .map(|i| file(&format!("d{}/f{i}.txt", i % 100), b"x"))
            .collect();
        let bytes = zip_bytes(&items);
        let (vfs, _mem, root) = vfs_with("big.zip", bytes);
        let started = std::time::Instant::now();
        assert_eq!(names(&vfs, &root).len(), 100);
        let cold = started.elapsed();
        let started = std::time::Instant::now();
        assert_eq!(names(&vfs, &root.join("d7")).len(), 500);
        let warm = started.elapsed();
        eprintln!("cold index of 50,000 entries: {cold:?}, warm listing: {warm:?}");
        assert!(cold < Duration::from_millis(500), "cold {cold:?}");
        assert!(warm < Duration::from_millis(20), "warm {warm:?}");
    }
}
