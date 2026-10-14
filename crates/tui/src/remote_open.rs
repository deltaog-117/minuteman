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
//! Where the copy of a remote file goes while a local program opens it.
//!
//! A program on this machine cannot read an `ssh://` path, so the file is fetched into a cache
//! folder first. Each remote path gets its own folder (named from a hash of the whole address, so
//! two `notes.txt` on different machines never meet) holding the file under its original name,
//! which keeps the extension a program picks its behaviour from. Folders untouched for a day are
//! removed the next time one is made. Nothing here talks to the disk except through a `Vfs`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use shared::{Location, Remote, Vfs, VfsError};

/// How long a fetched copy is kept without being fetched again.
const KEEP: Duration = Duration::from_secs(24 * 60 * 60);

/// The folder every fetched copy lives under: `$XDG_CACHE_HOME/minuteman/remote-open`, or
/// `~/.cache/...` when that is unset. `None` when neither variable names an absolute folder.
pub fn cache_root() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let base = var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")))?;
    Some(base.join("minuteman").join("remote-open"))
}

/// Where the copy of `remote` goes under `root`, or `None` for a path with no file name.
pub fn copy_path(root: &Path, remote: &Path) -> Option<PathBuf> {
    // The name of the file on the machine, not the machine's own (`ssh://box/` ends in `box`).
    let name = match Remote::locate(remote) {
        Ok(Location::Remote(at)) => at.path().file_name()?.to_owned(),
        _ => remote.file_name()?.to_owned(),
    };
    let mut hasher = DefaultHasher::new();
    remote.hash(&mut hasher);
    Some(root.join(format!("{:016x}", hasher.finish())).join(name))
}

/// Makes the folder for `remote`'s copy (clearing out old ones first) and returns the path to
/// write the copy to.
///
/// # Errors
///
/// A path with no file name, or a cache folder that cannot be made.
pub fn prepare(vfs: &dyn Vfs, root: &Path, remote: &Path) -> Result<PathBuf, VfsError> {
    let copy = copy_path(root, remote).ok_or_else(|| VfsError::NotFound(remote.to_path_buf()))?;
    sweep(vfs, root, SystemTime::now());
    let folder = copy.parent().unwrap_or(root);
    let missing: Vec<&Path> = folder.ancestors().take_while(|d| !vfs.exists(d)).collect();
    for dir in missing.into_iter().rev() {
        vfs.create_dir(dir)?;
    }
    Ok(copy)
}

/// Removes the copies under `root` last written more than a day before `now`. Best effort: a
/// folder that cannot be read or removed is left for next time.
fn sweep(vfs: &dyn Vfs, root: &Path, now: SystemTime) {
    let Ok(entries) = vfs.list_dir(root) else {
        return;
    };
    for entry in entries {
        let stale = entry
            .modified
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age > KEEP);
        if entry.is_dir && stale {
            let _ = vfs.remove_dir_all(&entry.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::MemVfs;

    #[test]
    fn a_copy_keeps_the_name_and_each_address_gets_its_own_folder() {
        let root = Path::new("/c");
        let a = copy_path(root, Path::new("ssh://one/x/notes.txt")).unwrap();
        let b = copy_path(root, Path::new("ssh://two/x/notes.txt")).unwrap();
        assert_eq!(a.file_name(), Some("notes.txt".as_ref()));
        assert_ne!(a.parent(), b.parent());
        assert_eq!(
            a,
            copy_path(root, Path::new("ssh://one/x/notes.txt")).unwrap()
        );
        assert_eq!(a.parent().and_then(Path::parent), Some(root));
    }

    #[test]
    fn a_path_with_no_file_name_has_no_copy() {
        assert_eq!(copy_path(Path::new("/c"), Path::new("ssh://one/")), None);
        assert_eq!(
            copy_path(Path::new("/c"), Path::new("ssh://one/x/..")),
            None
        );
    }

    #[test]
    fn preparing_makes_the_folders_and_an_old_copy_is_swept() {
        let vfs = MemVfs::new();
        let root = Path::new("/cache/minuteman/remote-open");
        let remote = Path::new("ssh://box/etc/hosts");
        let copy = prepare(&vfs, root, remote).unwrap();
        assert!(vfs.is_dir(copy.parent().unwrap()));
        assert_eq!(prepare(&vfs, root, remote).unwrap(), copy, "again is fine");

        sweep(&vfs, root, SystemTime::now() + KEEP * 2);
        assert!(!vfs.exists(copy.parent().unwrap()), "stale folder removed");
        assert!(vfs.is_dir(root), "the root stays");
    }

    #[test]
    fn a_fresh_copy_survives_the_sweep() {
        let vfs = MemVfs::new();
        let root = Path::new("/cache/r");
        let copy = prepare(&vfs, root, Path::new("ssh://box/a.txt")).unwrap();
        sweep(&vfs, root, SystemTime::now());
        assert!(vfs.is_dir(copy.parent().unwrap()));
    }
}
