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

//! An in-memory filesystem: a second [`Vfs`] backend that needs no disk. It exists to prove the
//! trait is not quietly shaped like the local disk (the same conformance suite runs against both),
//! and to let a test build a tree of any shape, symlinks included, without touching the real one.
//!
//! Paths are absolute and resolved the way a real filesystem does: `..` and symlinks (up to a hop
//! limit) are followed, a missing parent is `NotFound`, and nothing is overwritten.

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::io::{self, Cursor};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

use crate::error::VfsError;
use crate::vfs::{DirEntryInfo, FileKind, Metadata, ReadSeek, ScanEntry, Vfs};

/// The size unit "allocated" is rounded up to, like a filesystem's block.
const BLOCK: u64 = 4096;

/// Symlinks followed before a path is called a loop.
const MAX_LINK_HOPS: u32 = 40;

#[derive(Debug, Clone)]
enum Content {
    File(Vec<u8>),
    Dir,
    Symlink(PathBuf),
}

#[derive(Debug, Clone)]
struct Node {
    content: Content,
    inode: u64,
    modified: SystemTime,
}

#[derive(Debug)]
struct State {
    nodes: BTreeMap<PathBuf, Node>,
    next_inode: u64,
}

/// A filesystem held in memory. Share it between threads freely: it is `Send + Sync`.
#[derive(Debug)]
pub struct MemVfs {
    state: Mutex<State>,
    /// Report only what every backend must (kind, length, times), like a remote one that has no
    /// owner, link count or allocated size to give.
    sparse: bool,
}

impl Default for MemVfs {
    fn default() -> Self {
        Self::new()
    }
}

fn io_error(path: &Path, kind: io::ErrorKind, message: &str) -> VfsError {
    VfsError::Io {
        path: path.to_path_buf(),
        source: io::Error::new(kind, message.to_owned()),
    }
}

/// `path`'s parts, `..` kept and everything else that is not a name dropped.
fn parts(path: &Path) -> impl Iterator<Item = OsString> + '_ {
    path.components().filter_map(|c| match c {
        Component::Normal(name) => Some(name.to_os_string()),
        Component::ParentDir => Some("..".into()),
        _ => None,
    })
}

impl State {
    fn fresh_inode(&mut self) -> u64 {
        self.next_inode += 1;
        self.next_inode
    }

    /// `path` with `..` and symlinks resolved. The last component is followed only when
    /// `follow_last` is set, so a link can be looked at as a link. Existence is not checked here.
    fn resolve(&self, path: &Path, follow_last: bool) -> Result<PathBuf, VfsError> {
        let mut out = PathBuf::from("/");
        let mut pending: VecDeque<OsString> = parts(path).collect();
        let mut hops = 0;
        while let Some(part) = pending.pop_front() {
            if part == ".." {
                out.pop();
                continue;
            }
            out.push(&part);
            let Some(Node {
                content: Content::Symlink(target),
                ..
            }) = self.nodes.get(&out)
            else {
                continue;
            };
            if !follow_last && pending.is_empty() {
                continue;
            }
            hops += 1;
            if hops > MAX_LINK_HOPS {
                return Err(io_error(
                    path,
                    io::ErrorKind::Other,
                    "too many levels of symbolic links",
                ));
            }
            out.pop();
            if target.is_absolute() {
                out = PathBuf::from("/");
            }
            for part in parts(target).collect::<Vec<_>>().into_iter().rev() {
                pending.push_front(part);
            }
        }
        Ok(out)
    }

    /// The node at `path` and where it really is, or `NotFound`.
    fn find(&self, path: &Path, follow_last: bool) -> Result<(PathBuf, &Node), VfsError> {
        let at = self.resolve(path, follow_last)?;
        match self.nodes.get(&at) {
            Some(node) => Ok((at, node)),
            None => Err(VfsError::NotFound(path.to_path_buf())),
        }
    }

    fn exists(&self, path: &Path) -> bool {
        self.find(path, true).is_ok()
    }

    /// Adds a new node at `path`: the parent must be a folder, and nothing may be there already.
    fn insert_new(&mut self, path: &Path, content: Content) -> Result<PathBuf, VfsError> {
        let at = self.resolve(path, false)?;
        if self.nodes.contains_key(&at) {
            return Err(VfsError::AlreadyExists(path.to_path_buf()));
        }
        let parent = at.parent().unwrap_or(Path::new("/"));
        match self.nodes.get(parent) {
            Some(Node {
                content: Content::Dir,
                ..
            }) => {}
            Some(_) => return Err(VfsError::NotADirectory(parent.to_path_buf())),
            None => return Err(VfsError::NotFound(path.to_path_buf())),
        }
        let inode = self.fresh_inode();
        self.nodes.insert(
            at.clone(),
            Node {
                content,
                inode,
                modified: SystemTime::now(),
            },
        );
        Ok(at)
    }

    /// The paths directly inside the folder at `dir`.
    fn children(&self, dir: &Path) -> Vec<(&PathBuf, &Node)> {
        self.nodes
            .range(dir.to_path_buf()..)
            .take_while(|(path, _)| path.starts_with(dir))
            .filter(|(path, _)| path.parent() == Some(dir))
            .collect()
    }

    /// `at` and everything under it.
    fn subtree(&self, at: &Path) -> Vec<PathBuf> {
        self.nodes
            .range(at.to_path_buf()..)
            .take_while(|(path, _)| path.starts_with(at))
            .map(|(path, _)| path.clone())
            .collect()
    }
}

fn stat(node: &Node, sparse: bool) -> Metadata {
    let (kind, len, allocated, mode) = match &node.content {
        Content::File(data) => {
            let len = data.len() as u64;
            (FileKind::File, len, len.div_ceil(BLOCK) * BLOCK, 0o100_644)
        }
        Content::Dir => (FileKind::Dir, BLOCK, BLOCK, 0o040_755),
        Content::Symlink(target) => (
            FileKind::Symlink,
            target.as_os_str().len() as u64,
            0,
            0o120_777,
        ),
    };
    fn full<T>(sparse: bool, value: T) -> Option<T> {
        (!sparse).then_some(value)
    }
    Metadata {
        kind,
        len,
        allocated: full(sparse, allocated),
        mode: full(sparse, mode),
        uid: full(sparse, 1000),
        gid: full(sparse, 1000),
        nlink: full(sparse, 1),
        modified: Some(node.modified),
        accessed: Some(node.modified),
        created: Some(node.modified),
        device: full(sparse, 1),
        inode: full(sparse, node.inode),
    }
}

impl MemVfs {
    /// An empty filesystem holding just the root folder.
    pub fn new() -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            PathBuf::from("/"),
            Node {
                content: Content::Dir,
                inode: 1,
                modified: SystemTime::now(),
            },
        );
        Self {
            state: Mutex::new(State {
                nodes,
                next_inode: 1,
            }),
            sparse: false,
        }
    }

    /// The same filesystem, reporting only kind, length and times: the owner, permissions, link
    /// count, allocated size and device and inode numbers are `None`, as a backend that does not
    /// have them would leave them.
    pub fn sparse(mut self) -> Self {
        self.sparse = true;
        self
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock leaves the tree as it was, so a later caller can carry on.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Adds `path` with `content`, making any missing folders above it, replacing a file that is
    /// already there. For building a fixture: it panics where a real operation would fail.
    fn put(&self, path: &Path, content: Content) {
        let mut state = self.state();
        let at = state
            .resolve(path, false)
            .expect("a resolvable fixture path");
        let mut folder = PathBuf::from("/");
        for part in at.parent().unwrap_or(Path::new("/")).components().skip(1) {
            folder.push(part);
            if !state.nodes.contains_key(&folder) {
                state
                    .insert_new(&folder, Content::Dir)
                    .expect("a folder can be made");
            }
        }
        if let Some(existing) = state.nodes.get_mut(&at) {
            assert!(
                matches!(
                    (&existing.content, &content),
                    (Content::File(_), Content::File(_))
                ),
                "fixture clashes with what is at {}",
                at.display()
            );
            existing.content = content;
            existing.modified = SystemTime::now();
        } else {
            state
                .insert_new(&at, content)
                .expect("a fixture can be added");
        }
    }

    /// Makes the folder `path` and any above it (a fixture helper, like `mkdir -p`).
    pub fn add_dir(&self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        let mut state = self.state();
        let mut folder = PathBuf::from("/");
        for part in path.components().skip(1) {
            folder.push(part);
            if !state.nodes.contains_key(&folder) {
                state
                    .insert_new(&folder, Content::Dir)
                    .expect("a folder can be made");
            }
        }
    }

    /// Adds (or replaces) the file `path` holding `bytes`, making any folders above it.
    pub fn add_file(&self, path: impl AsRef<Path>, bytes: impl Into<Vec<u8>>) {
        self.put(path.as_ref(), Content::File(bytes.into()));
    }

    /// Adds the symlink `link` pointing at `target` (which need not exist).
    pub fn add_symlink(&self, link: impl AsRef<Path>, target: impl AsRef<Path>) {
        self.put(
            link.as_ref(),
            Content::Symlink(target.as_ref().to_path_buf()),
        );
    }
}

impl Vfs for MemVfs {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, VfsError> {
        let state = self.state();
        let (at, node) = state
            .find(path, true)
            .map_err(|_| VfsError::NotADirectory(path.to_path_buf()))?;
        if !matches!(node.content, Content::Dir) {
            return Err(VfsError::NotADirectory(path.to_path_buf()));
        }
        let mut entries: Vec<DirEntryInfo> = state
            .children(&at)
            .into_iter()
            .map(|(child, _)| {
                // Like the local listing: what a link points at decides the kind and the size, and
                // a broken link is an empty non-folder.
                let followed = state
                    .find(child, true)
                    .ok()
                    .map(|(_, node)| stat(node, self.sparse));
                DirEntryInfo {
                    name: child
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                    path: path.join(child.file_name().unwrap_or_default()),
                    is_dir: followed.as_ref().is_some_and(|m| m.kind == FileKind::Dir),
                    size: followed.as_ref().map_or(0, |m| m.len),
                    modified: followed.as_ref().and_then(|m| m.modified),
                    mode: followed.as_ref().and_then(|m| m.mode),
                }
            })
            .collect();
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });
        Ok(entries)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.state()
            .find(path, true)
            .is_ok_and(|(_, node)| matches!(node.content, Content::Dir))
    }

    fn exists(&self, path: &Path) -> bool {
        self.state().exists(path)
    }

    fn metadata(&self, path: &Path) -> Result<Metadata, VfsError> {
        self.state()
            .find(path, true)
            .map(|(_, node)| stat(node, self.sparse))
    }

    fn symlink_metadata(&self, path: &Path) -> Result<Metadata, VfsError> {
        self.state()
            .find(path, false)
            .map(|(_, node)| stat(node, self.sparse))
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, VfsError> {
        match &self.state().find(path, false)?.1.content {
            Content::Symlink(target) => Ok(target.clone()),
            _ => Err(io_error(path, io::ErrorKind::InvalidInput, "not a symlink")),
        }
    }

    fn open_read(&self, path: &Path) -> Result<Box<dyn ReadSeek>, VfsError> {
        match &self.state().find(path, true)?.1.content {
            Content::File(data) => Ok(Box::new(Cursor::new(data.clone()))),
            _ => Err(io_error(path, io::ErrorKind::InvalidInput, "not a file")),
        }
    }

    fn scan_dir(&self, path: &Path) -> Result<Vec<ScanEntry>, VfsError> {
        let state = self.state();
        let (at, node) = state.find(path, true)?;
        if !matches!(node.content, Content::Dir) {
            return Err(VfsError::NotADirectory(path.to_path_buf()));
        }
        Ok(state
            .children(&at)
            .into_iter()
            .map(|(child, node)| ScanEntry {
                path: path.join(child.file_name().unwrap_or_default()),
                name: child
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                metadata: Some(stat(node, self.sparse)),
            })
            .collect())
    }

    fn local_path(&self, _path: &Path) -> Option<PathBuf> {
        None
    }

    fn create_dir(&self, path: &Path) -> Result<(), VfsError> {
        self.state().insert_new(path, Content::Dir).map(drop)
    }

    fn create_file(&self, path: &Path) -> Result<(), VfsError> {
        self.state()
            .insert_new(path, Content::File(Vec::new()))
            .map(drop)
    }

    fn touch(&self, path: &Path) -> Result<(), VfsError> {
        let mut state = self.state();
        let at = state.resolve(path, true)?;
        match state.nodes.get_mut(&at) {
            Some(node) if matches!(node.content, Content::File(_)) => {
                node.modified = SystemTime::now();
                Ok(())
            }
            Some(_) => Err(io_error(
                path,
                io::ErrorKind::InvalidInput,
                "is a directory",
            )),
            None => state.insert_new(path, Content::File(Vec::new())).map(drop),
        }
    }

    fn copy_file(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        let mut state = self.state();
        let data = match &state.find(src, true)?.1.content {
            Content::File(data) => data.clone(),
            _ => return Err(io_error(src, io::ErrorKind::InvalidInput, "not a file")),
        };
        if state.exists(dst) {
            return Err(VfsError::AlreadyExists(dst.to_path_buf()));
        }
        state.insert_new(dst, Content::File(data)).map(drop)
    }

    fn rename(&self, src: &Path, dst: &Path) -> Result<(), VfsError> {
        let mut state = self.state();
        let from = state.find(src, false)?.0;
        if state.exists(dst) {
            return Err(VfsError::AlreadyExists(dst.to_path_buf()));
        }
        let to = state.resolve(dst, false)?;
        if to.starts_with(&from) {
            return Err(io_error(
                src,
                io::ErrorKind::InvalidInput,
                "cannot move a folder into itself",
            ));
        }
        let parent = to.parent().unwrap_or(Path::new("/"));
        match state.nodes.get(parent) {
            Some(Node {
                content: Content::Dir,
                ..
            }) => {}
            Some(_) => return Err(VfsError::NotADirectory(parent.to_path_buf())),
            None => return Err(VfsError::NotFound(dst.to_path_buf())),
        }
        for old in state.subtree(&from) {
            let node = state.nodes.remove(&old).expect("listed a moment ago");
            let rest = old.strip_prefix(&from).expect("under the moved path");
            state.nodes.insert(to.join(rest), node);
        }
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> Result<(), VfsError> {
        let mut state = self.state();
        let (at, node) = state.find(path, false)?;
        if matches!(node.content, Content::Dir) {
            return Err(io_error(
                path,
                io::ErrorKind::InvalidInput,
                "is a directory",
            ));
        }
        state.nodes.remove(&at);
        Ok(())
    }

    fn remove_dir_all(&self, path: &Path) -> Result<(), VfsError> {
        let mut state = self.state();
        let (at, node) = state.find(path, false)?;
        match node.content {
            Content::File(_) => {
                return Err(io_error(
                    path,
                    io::ErrorKind::InvalidInput,
                    "not a directory",
                ));
            }
            // Removing a link removes the link, never what it points at.
            Content::Symlink(_) => {
                state.nodes.remove(&at);
                return Ok(());
            }
            Content::Dir => {}
        }
        if at == Path::new("/") {
            return Err(io_error(
                path,
                io::ErrorKind::InvalidInput,
                "cannot remove the root",
            ));
        }
        for gone in state.subtree(&at) {
            state.nodes.remove(&gone);
        }
        Ok(())
    }
}
