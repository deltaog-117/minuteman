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

//! The tests every [`Vfs`] backend must pass. A backend's own test calls [`check_backend`] with an
//! instance, a [`Seed`] that can put files on its store by some means other than the trait (the
//! trait has no "write bytes" or "make a symlink", since nothing in the file manager needs them
//! yet), and a folder to work in. Each scenario takes a fresh folder of its own under that root, so
//! they cannot disturb one another.
//!
//! This is what keeps the trait honest: a method that quietly only makes sense for the local disk
//! fails here against the in-memory backend, before a remote one is built on it.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::VfsError;
use crate::vfs::{FileKind, Vfs};

/// Puts fixtures on a backend's store directly, for what the trait cannot create.
pub trait Seed {
    /// Makes the folder `path` and any above it.
    fn dir(&self, path: &Path);
    /// Makes the file `path` holding `bytes`, and any folders above it.
    fn file(&self, path: &Path, bytes: &[u8]);
    /// Makes the symlink `link` pointing at `target`, which need not exist.
    fn symlink(&self, link: &Path, target: &Path);
}

/// One scenario: what to check about a backend, in a folder of its own.
type Scenario = fn(&dyn Vfs, &dyn Seed, &Path);

/// Runs every scenario against `vfs`, working under `root` (which `seed` makes).
///
/// # Panics
///
/// On the first expectation a backend does not meet, naming it.
pub fn check_backend(vfs: &dyn Vfs, seed: &dyn Seed, root: &Path) {
    seed.dir(root);
    let scenarios: [(&str, Scenario); 12] = [
        ("listing", listing_is_sorted_with_folders_first),
        ("listing-links", listing_follows_links_for_display),
        (
            "create",
            creating_refuses_what_is_there_and_a_missing_parent,
        ),
        ("touch", touch_creates_then_refreshes),
        ("copy", copy_never_overwrites),
        ("rename", rename_never_overwrites_and_moves_trees),
        ("remove", removing_files_and_trees),
        ("metadata", metadata_follows_links_only_when_asked),
        ("links", reading_links),
        ("read", reading_and_seeking),
        ("scan", scanning_reports_entries_themselves),
        ("walk", a_link_loop_is_reported_not_followed),
    ];
    for (name, scenario) in scenarios {
        let case = root.join(name);
        seed.dir(&case);
        scenario(vfs, seed, &case);
    }
}

fn listing_is_sorted_with_folders_first(vfs: &dyn Vfs, seed: &dyn Seed, dir: &Path) {
    seed.dir(&dir.join("b_dir"));
    seed.dir(&dir.join("a_dir"));
    seed.file(&dir.join("Z.txt"), b"zzz");
    seed.file(&dir.join("a_file.txt"), b"12345");

    let entries = vfs.list_dir(dir).unwrap();

    assert_eq!(
        entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["a_dir", "b_dir", "a_file.txt", "Z.txt"],
        "folders first, then names ignoring case"
    );
    let file = &entries[2];
    assert!(!file.is_dir);
    assert_eq!(file.size, 5);
    assert_eq!(file.path, dir.join("a_file.txt"));
    assert!(entries[0].is_dir);
    assert!(
        matches!(
            vfs.list_dir(&dir.join("a_file.txt")),
            Err(VfsError::NotADirectory(_))
        ),
        "listing a file is NotADirectory"
    );
}

fn listing_follows_links_for_display(vfs: &dyn Vfs, seed: &dyn Seed, dir: &Path) {
    seed.dir(&dir.join("real"));
    seed.file(&dir.join("target.txt"), b"abcd");
    seed.symlink(&dir.join("to_dir"), &dir.join("real"));
    seed.symlink(&dir.join("to_file"), &dir.join("target.txt"));
    seed.symlink(&dir.join("dangling"), &dir.join("missing"));

    let entries = vfs.list_dir(dir).unwrap();
    let find = |name: &str| entries.iter().find(|e| e.name == name).unwrap();

    assert!(
        find("to_dir").is_dir,
        "a link to a folder lists as a folder"
    );
    assert!(!find("to_file").is_dir);
    assert_eq!(
        find("to_file").size,
        4,
        "a link lists with its target's size"
    );
    assert!(!find("dangling").is_dir);
    assert_eq!(find("dangling").size, 0, "a broken link lists as empty");
    assert_eq!(find("dangling").modified, None);
}

fn creating_refuses_what_is_there_and_a_missing_parent(vfs: &dyn Vfs, seed: &dyn Seed, dir: &Path) {
    vfs.create_dir(&dir.join("d")).unwrap();
    vfs.create_file(&dir.join("f")).unwrap();
    assert!(vfs.is_dir(&dir.join("d")) && !vfs.is_dir(&dir.join("f")));
    assert!(vfs.exists(&dir.join("f")));

    assert!(matches!(
        vfs.create_dir(&dir.join("d")),
        Err(VfsError::AlreadyExists(_))
    ));
    assert!(matches!(
        vfs.create_file(&dir.join("f")),
        Err(VfsError::AlreadyExists(_))
    ));
    assert!(matches!(
        vfs.create_dir(&dir.join("no/such/parent")),
        Err(VfsError::NotFound(_))
    ));
    assert!(matches!(
        vfs.create_file(&dir.join("no/such/parent")),
        Err(VfsError::NotFound(_))
    ));

    seed.file(&dir.join("keep"), b"keep me");
    assert!(vfs.create_file(&dir.join("keep")).is_err());
    let mut kept = String::new();
    vfs.open_read(&dir.join("keep"))
        .unwrap()
        .read_to_string(&mut kept)
        .unwrap();
    assert_eq!(kept, "keep me", "create_file never truncates");
}

fn touch_creates_then_refreshes(vfs: &dyn Vfs, seed: &dyn Seed, dir: &Path) {
    let fresh = dir.join("fresh");
    vfs.touch(&fresh).unwrap();
    assert_eq!(
        vfs.metadata(&fresh).unwrap().len,
        0,
        "a touched file is empty"
    );

    seed.file(&dir.join("old"), b"content");
    let before = vfs.metadata(&dir.join("old")).unwrap().modified.unwrap();
    std::thread::sleep(Duration::from_millis(20));
    vfs.touch(&dir.join("old")).unwrap();
    let after = vfs.metadata(&dir.join("old")).unwrap();

    assert!(after.modified.unwrap() > before, "touch refreshes the time");
    assert_eq!(after.len, 7, "and leaves the content alone");
}

fn copy_never_overwrites(vfs: &dyn Vfs, seed: &dyn Seed, dir: &Path) {
    seed.file(&dir.join("a"), b"alpha");
    seed.file(&dir.join("b"), b"beta");

    vfs.copy_file(&dir.join("a"), &dir.join("c")).unwrap();
    let mut copied = Vec::new();
    vfs.open_read(&dir.join("c"))
        .unwrap()
        .read_to_end(&mut copied)
        .unwrap();
    assert_eq!(copied, b"alpha");
    assert!(vfs.exists(&dir.join("a")), "the source is still there");

    assert!(matches!(
        vfs.copy_file(&dir.join("a"), &dir.join("b")),
        Err(VfsError::AlreadyExists(_))
    ));
    assert!(matches!(
        vfs.copy_file(&dir.join("nothing"), &dir.join("d")),
        Err(VfsError::NotFound(_))
    ));
}

fn rename_never_overwrites_and_moves_trees(vfs: &dyn Vfs, seed: &dyn Seed, dir: &Path) {
    seed.file(&dir.join("tree/inner/leaf"), b"leaf");
    seed.file(&dir.join("other"), b"other");

    vfs.rename(&dir.join("tree"), &dir.join("moved")).unwrap();

    assert!(!vfs.exists(&dir.join("tree")));
    assert!(
        vfs.exists(&dir.join("moved/inner/leaf")),
        "a folder moves with its contents"
    );
    assert!(matches!(
        vfs.rename(&dir.join("moved"), &dir.join("other")),
        Err(VfsError::AlreadyExists(_))
    ));
    assert!(
        vfs.exists(&dir.join("moved")),
        "a refused rename changes nothing"
    );
    assert!(vfs.rename(&dir.join("nothing"), &dir.join("x")).is_err());
    assert!(
        vfs.rename(&dir.join("moved"), &dir.join("moved/inside"))
            .is_err(),
        "a folder cannot move into itself"
    );
}

fn removing_files_and_trees(vfs: &dyn Vfs, seed: &dyn Seed, dir: &Path) {
    seed.file(&dir.join("f"), b"x");
    seed.file(&dir.join("tree/a/b"), b"y");
    seed.file(&dir.join("keep"), b"keep");
    seed.symlink(&dir.join("link"), &dir.join("keep"));

    vfs.remove_file(&dir.join("f")).unwrap();
    assert!(!vfs.exists(&dir.join("f")));
    assert!(matches!(
        vfs.remove_file(&dir.join("f")),
        Err(VfsError::NotFound(_))
    ));
    assert!(
        vfs.remove_file(&dir.join("tree")).is_err(),
        "remove_file refuses a folder"
    );

    vfs.remove_dir_all(&dir.join("tree")).unwrap();
    assert!(!vfs.exists(&dir.join("tree")));
    assert!(
        vfs.remove_dir_all(&dir.join("keep")).is_err(),
        "remove_dir_all refuses a file"
    );

    vfs.remove_file(&dir.join("link")).unwrap();
    assert!(
        vfs.exists(&dir.join("keep")),
        "removing a link leaves its target"
    );
}

fn metadata_follows_links_only_when_asked(vfs: &dyn Vfs, seed: &dyn Seed, dir: &Path) {
    seed.file(&dir.join("file"), b"12345678");
    seed.dir(&dir.join("folder"));
    seed.symlink(&dir.join("link"), &dir.join("file"));
    seed.symlink(&dir.join("dangling"), &dir.join("missing"));

    let file = vfs.metadata(&dir.join("file")).unwrap();
    assert_eq!((file.kind, file.len), (FileKind::File, 8));
    assert!(file.modified.is_some());
    assert_eq!(
        vfs.metadata(&dir.join("folder")).unwrap().kind,
        FileKind::Dir
    );

    assert_eq!(
        vfs.metadata(&dir.join("link")).unwrap().kind,
        FileKind::File
    );
    assert_eq!(
        vfs.symlink_metadata(&dir.join("link")).unwrap().kind,
        FileKind::Symlink,
        "symlink_metadata describes the link itself"
    );

    assert!(matches!(
        vfs.metadata(&dir.join("dangling")),
        Err(VfsError::NotFound(_))
    ));
    assert_eq!(
        vfs.symlink_metadata(&dir.join("dangling")).unwrap().kind,
        FileKind::Symlink,
        "a broken link can still be described"
    );
    assert!(!vfs.exists(&dir.join("dangling")));
    assert!(matches!(
        vfs.metadata(&dir.join("absent")),
        Err(VfsError::NotFound(_))
    ));

    // What a backend reports must at least be consistent: an allocated size is never below the
    // bytes a regular file holds, unless it is sparse or inlined, so only check it is present or
    // absent as a whole.
    if let Some(allocated) = file.allocated {
        assert!(
            allocated < 1 << 40,
            "an allocated size is a plausible number"
        );
    }
}

fn reading_links(vfs: &dyn Vfs, seed: &dyn Seed, dir: &Path) {
    seed.file(&dir.join("file"), b"x");
    seed.symlink(&dir.join("rel"), Path::new("file"));
    seed.symlink(&dir.join("abs"), &dir.join("file"));
    seed.symlink(&dir.join("dangling"), Path::new("missing"));

    assert_eq!(
        vfs.read_link(&dir.join("rel")).unwrap(),
        PathBuf::from("file")
    );
    assert_eq!(vfs.read_link(&dir.join("abs")).unwrap(), dir.join("file"));
    assert_eq!(
        vfs.read_link(&dir.join("dangling")).unwrap(),
        PathBuf::from("missing"),
        "the target is returned as written, even when it is not there"
    );
    assert!(
        vfs.read_link(&dir.join("file")).is_err(),
        "a file is not a link"
    );
    assert!(matches!(
        vfs.read_link(&dir.join("absent")),
        Err(VfsError::NotFound(_))
    ));
}

fn reading_and_seeking(vfs: &dyn Vfs, seed: &dyn Seed, dir: &Path) {
    seed.file(&dir.join("f"), b"0123456789");
    seed.dir(&dir.join("d"));
    seed.symlink(&dir.join("link"), &dir.join("f"));

    let mut reader = vfs.open_read(&dir.join("f")).unwrap();
    let mut head = [0u8; 4];
    reader.read_exact(&mut head).unwrap();
    assert_eq!(&head, b"0123");
    reader.seek(SeekFrom::Start(7)).unwrap();
    let mut tail = String::new();
    reader.read_to_string(&mut tail).unwrap();
    assert_eq!(tail, "789");
    reader.seek(SeekFrom::End(-2)).unwrap();
    let mut last = [0u8; 2];
    reader.read_exact(&mut last).unwrap();
    assert_eq!(&last, b"89");

    let mut through_link = String::new();
    vfs.open_read(&dir.join("link"))
        .unwrap()
        .read_to_string(&mut through_link)
        .unwrap();
    assert_eq!(through_link, "0123456789", "a link is followed");

    assert!(
        vfs.open_read(&dir.join("d")).is_err(),
        "a folder cannot be read as a file"
    );
    assert!(matches!(
        vfs.open_read(&dir.join("absent")),
        Err(VfsError::NotFound(_))
    ));
}

fn scanning_reports_entries_themselves(vfs: &dyn Vfs, seed: &dyn Seed, dir: &Path) {
    seed.file(&dir.join("file"), b"abc");
    seed.dir(&dir.join("folder"));
    seed.symlink(&dir.join("link"), &dir.join("folder"));

    let mut scanned = vfs.scan_dir(dir).unwrap();
    scanned.sort_by(|a, b| a.name.cmp(&b.name));

    assert_eq!(
        scanned.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["file", "folder", "link"]
    );
    let kind = |entry: &crate::vfs::ScanEntry| entry.metadata.as_ref().unwrap().kind;
    assert_eq!(kind(&scanned[0]), FileKind::File);
    assert_eq!(kind(&scanned[1]), FileKind::Dir);
    assert_eq!(
        kind(&scanned[2]),
        FileKind::Symlink,
        "a scan reports a link as a link, never as what it points at"
    );
    assert_eq!(scanned[0].metadata.as_ref().unwrap().len, 3);
    assert_eq!(scanned[0].path, dir.join("file"));
    // Entries of one folder are on one filesystem, which is what a scan relies on to stay on it.
    let device = |entry: &crate::vfs::ScanEntry| entry.metadata.as_ref().unwrap().device;
    assert_eq!(device(&scanned[0]), device(&scanned[1]));
    assert!(
        vfs.scan_dir(&dir.join("file")).is_err(),
        "a file cannot be scanned"
    );
}

fn a_link_loop_is_reported_not_followed(vfs: &dyn Vfs, seed: &dyn Seed, dir: &Path) {
    seed.dir(&dir.join("sub"));
    seed.symlink(&dir.join("sub/again"), dir);
    seed.symlink(&dir.join("a"), &dir.join("b"));
    seed.symlink(&dir.join("b"), &dir.join("a"));

    // The link back to the folder above is just an entry: scanning it does not loop.
    let scanned = vfs.scan_dir(&dir.join("sub")).unwrap();
    assert_eq!(scanned.len(), 1);
    assert_eq!(
        scanned[0].metadata.as_ref().unwrap().kind,
        FileKind::Symlink
    );

    // A cycle of links has no target: it does not exist, and reading it is an error, not a hang.
    assert!(!vfs.exists(&dir.join("a")));
    assert!(vfs.metadata(&dir.join("a")).is_err());
    assert!(vfs.open_read(&dir.join("a")).is_err());
    assert_eq!(
        vfs.symlink_metadata(&dir.join("a")).unwrap().kind,
        FileKind::Symlink
    );
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::{LocalVfs, MemVfs};

    impl Seed for MemVfs {
        fn dir(&self, path: &Path) {
            self.add_dir(path);
        }
        fn file(&self, path: &Path, bytes: &[u8]) {
            self.add_file(path, bytes);
        }
        fn symlink(&self, link: &Path, target: &Path) {
            self.add_symlink(link, target);
        }
    }

    /// Seeds the real disk with `std`.
    struct LocalSeed;

    impl Seed for LocalSeed {
        fn dir(&self, path: &Path) {
            std::fs::create_dir_all(path).unwrap();
        }
        fn file(&self, path: &Path, bytes: &[u8]) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        fn symlink(&self, link: &Path, target: &Path) {
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, link).unwrap();
            #[cfg(not(unix))]
            let _ = (link, target);
        }
    }

    #[test]
    fn a_backend_can_be_shared_between_threads() {
        fn shareable<T: Send + Sync + ?Sized>() {}
        shareable::<LocalVfs>();
        shareable::<MemVfs>();
        shareable::<dyn Vfs>();
    }

    #[test]
    fn only_the_local_disk_has_a_local_path() {
        let path = Path::new("/some/file");
        assert_eq!(LocalVfs.local_path(path), Some(path.to_path_buf()));
        assert_eq!(MemVfs::new().local_path(path), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_local_disk_reports_owner_links_and_allocated_size() {
        let dir = std::env::temp_dir().join(format!("minuteman-meta-{}", std::process::id()));
        LocalSeed.file(&dir.join("f"), &[7u8; 5000]);

        let meta = LocalVfs.metadata(&dir.join("f")).unwrap();

        assert_eq!(meta.len, 5000);
        assert!(
            meta.allocated
                .is_some_and(|bytes| bytes >= 5000 && bytes % 512 == 0)
        );
        assert_eq!(meta.nlink, Some(1));
        assert!(meta.uid.is_some() && meta.gid.is_some());
        assert!(meta.device.is_some() && meta.inode.is_some());
        assert!(meta.mode.is_some_and(|mode| mode & 0o170_000 == 0o100_000));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_in_memory_backend_reports_whole_blocks_and_distinct_inodes() {
        let memory = MemVfs::new();
        memory.add_file("/a", vec![0u8; 5000]);
        memory.add_file("/b", vec![0u8; 1]);

        let a = memory.metadata(Path::new("/a")).unwrap();
        let b = memory.metadata(Path::new("/b")).unwrap();

        assert_eq!(a.allocated, Some(8192));
        assert_eq!(b.allocated, Some(4096));
        assert_ne!(a.inode, b.inode);
    }

    #[test]
    fn a_sparse_backend_still_passes_and_leaves_the_optional_fields_out() {
        let sparse = MemVfs::new().sparse();
        check_backend(&sparse, &sparse, Path::new("/work"));

        let meta = sparse.metadata(Path::new("/work/listing")).unwrap();
        assert_eq!(meta.kind, FileKind::Dir);
        assert!(meta.modified.is_some());
        assert!(
            meta.allocated.is_none()
                && meta.mode.is_none()
                && meta.uid.is_none()
                && meta.gid.is_none()
                && meta.nlink.is_none()
                && meta.device.is_none()
                && meta.inode.is_none()
        );
    }

    #[test]
    fn the_in_memory_backend_passes() {
        let memory = MemVfs::new();
        check_backend(&memory, &memory, Path::new("/work"));
    }

    #[cfg(unix)]
    #[test]
    fn the_local_disk_passes() {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "minuteman-conformance-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);

        check_backend(&LocalVfs, &LocalSeed, &root);

        std::fs::remove_dir_all(&root).unwrap();
    }
}
