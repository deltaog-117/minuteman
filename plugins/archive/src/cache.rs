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

//! The table of contents of a large `.tar.gz`, kept on this machine's disk between runs.
//!
//! A gzip stream cannot be listed without being inflated end to end, so opening a big `.tar.gz`
//! costs the same every time the plugin starts afresh. The entries are stored once, keyed by the
//! archive's path, and used again while the file still has the size and time it had.
//!
//! The cache only ever saves time, so nothing here is allowed to fail loudly: a file that is
//! missing, damaged, of another version or for another archive is a miss, and a store that cannot
//! be written is dropped. Stored entries go back through the same builder as freshly read ones,
//! so the limits on entries and the tree's rules hold for them as well. The folder and its files
//! are private to the user, since a listing names what is inside.

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, BufWriter, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::index::{Entry, Stamp};

/// Changes whenever what is stored changes shape, so an older file is a miss and not a misreading.
const VERSION: u32 = 1;
/// Listings kept; storing one more removes the least recently written.
const MAX_FILES: usize = 32;
/// A stored listing larger than this is not read: 200,000 entries come to a small fraction of it.
const MAX_FILE_BYTES: u64 = 256 << 20;
/// An archive that was indexed faster than this is not stored, since reading it again is as quick
/// as reading the cache and the folder would fill with listings that save nothing.
const STORE_AFTER: Duration = Duration::from_millis(250);

#[derive(Serialize)]
struct Record<'a> {
    version: u32,
    path: &'a str,
    stamp: Stamp,
    entries: &'a [Entry],
}

#[derive(Deserialize)]
struct Stored {
    version: u32,
    path: String,
    stamp: Stamp,
    entries: Vec<Entry>,
}

#[derive(Debug, Clone)]
pub struct DiskCache {
    dir: PathBuf,
    store_after: Duration,
}

impl DiskCache {
    /// The cache in the user's cache folder (`~/.cache/minuteman/archive-index` on Linux), or
    /// `None` on a machine that has no home to put it in.
    pub fn in_user_cache() -> Option<Self> {
        directories::ProjectDirs::from("", "", "minuteman")
            .map(|dirs| Self::at(dirs.cache_dir().join("archive-index")))
    }

    pub fn at(dir: PathBuf) -> Self {
        Self {
            dir,
            store_after: STORE_AFTER,
        }
    }

    /// The same cache, storing any archive that took at least `store_after` to index.
    #[cfg(test)]
    pub(crate) fn storing_after(mut self, store_after: Duration) -> Self {
        self.store_after = store_after;
        self
    }

    /// Whether indexing that took `took` is worth keeping.
    pub(crate) fn worth_storing(&self, took: Duration) -> bool {
        took >= self.store_after
    }

    fn file_for(&self, archive: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        archive.hash(&mut hasher);
        self.dir.join(format!("{:016x}.json", hasher.finish()))
    }

    /// The entries stored for `archive` when it looked like `stamp`, or `None` for any reason
    /// they cannot be used.
    pub(crate) fn load(&self, archive: &Path, stamp: Stamp) -> Option<Vec<Entry>> {
        let archive = archive.to_str()?;
        let file = File::open(self.file_for(archive)).ok()?;
        if file.metadata().ok()?.len() > MAX_FILE_BYTES {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES).read_to_end(&mut bytes).ok()?;
        let stored: Stored = serde_json::from_slice(&bytes).ok()?;
        // The name is a hash, so another archive can land on the same file; the content decides.
        (stored.version == VERSION && stored.path == archive && stored.stamp == stamp)
            .then_some(stored.entries)
    }

    /// Keeps `entries` as the listing of `archive` when it looked like `stamp`.
    ///
    /// # Errors
    ///
    /// Whatever stops the folder or the file being written; the caller can ignore it.
    pub(crate) fn store(&self, archive: &Path, stamp: Stamp, entries: &[Entry]) -> io::Result<()> {
        let archive = archive
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not text"))?;
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)?;
        let target = self.file_for(archive);
        // Written whole beside the target and renamed over it, so a reader never sees half a file.
        let partial = target.with_extension(format!("tmp{}", std::process::id()));
        let written = (|| {
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&partial)?;
            let mut out = BufWriter::new(file);
            serde_json::to_writer(
                &mut out,
                &Record {
                    version: VERSION,
                    path: archive,
                    stamp,
                    entries,
                },
            )?;
            out.flush()?;
            fs::rename(&partial, &target)
        })();
        if written.is_err() {
            let _ = fs::remove_file(&partial);
            return written;
        }
        self.prune();
        Ok(())
    }

    /// Removes the oldest listings beyond [`MAX_FILES`].
    fn prune(&self) {
        let Ok(read) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut listings: Vec<_> = read
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
            .collect();
        if listings.len() <= MAX_FILES {
            return;
        }
        listings.sort();
        for (_, path) in &listings[..listings.len() - MAX_FILES] {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::SystemTime;

    use shared::Format;

    use super::*;
    use crate::index::Index;

    fn scratch(label: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "minuteman-archive-cache-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn stamp(len: u64) -> Stamp {
        Stamp {
            len,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
        }
    }

    /// A tar.gz of a few files, as the entries the indexer read from it.
    fn entries() -> Vec<Entry> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, body) in [("d/a.txt", &b"alpha"[..]), ("d/b.txt", b"beta"), ("c", b"")] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            builder.append_data(&mut header, name, body).unwrap();
        }
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&builder.into_inner().unwrap()).unwrap();
        let bytes = encoder.finish().unwrap();
        let (_, entries) =
            Index::build_recording(Box::new(io::Cursor::new(bytes)), Format::TarGz, stamp(1))
                .unwrap();
        entries
    }

    #[test]
    fn what_is_stored_comes_back_for_the_same_archive_and_stamp() {
        let cache = DiskCache::at(scratch("roundtrip"));
        let archive = Path::new("/x/a.tar.gz");
        cache.store(archive, stamp(7), &entries()).unwrap();
        let loaded = cache.load(archive, stamp(7)).expect("a hit");
        let index = Index::from_entries(loaded, stamp(7)).unwrap();
        let at = index.resolve("d/b.txt", true).expect("the file is in it");
        assert_eq!(index.node(at).size, 4);
        let _ = fs::remove_dir_all(&cache.dir);
    }

    #[test]
    fn a_changed_archive_or_another_one_is_a_miss() {
        let cache = DiskCache::at(scratch("miss"));
        let archive = Path::new("/x/a.tar.gz");
        cache.store(archive, stamp(7), &entries()).unwrap();
        assert!(cache.load(archive, stamp(8)).is_none(), "size changed");
        let later = Stamp {
            modified: Some(SystemTime::UNIX_EPOCH),
            ..stamp(7)
        };
        assert!(cache.load(archive, later).is_none(), "time changed");
        assert!(cache.load(Path::new("/x/b.tar.gz"), stamp(7)).is_none());
        let _ = fs::remove_dir_all(&cache.dir);
    }

    #[test]
    fn a_damaged_or_foreign_file_is_a_miss_and_is_replaced_by_the_next_store() {
        let cache = DiskCache::at(scratch("damaged"));
        let archive = Path::new("/x/a.tar.gz");
        cache.store(archive, stamp(7), &entries()).unwrap();
        let file = cache.file_for("/x/a.tar.gz");
        fs::write(&file, b"{ not a listing").unwrap();
        assert!(cache.load(archive, stamp(7)).is_none());
        cache.store(archive, stamp(7), &entries()).unwrap();
        assert!(cache.load(archive, stamp(7)).is_some());
        let _ = fs::remove_dir_all(&cache.dir);
    }

    #[test]
    fn the_folder_and_files_are_private_and_only_the_newest_listings_stay() {
        use std::os::unix::fs::PermissionsExt;
        let cache = DiskCache::at(scratch("private"));
        for number in 0..MAX_FILES + 5 {
            let archive = PathBuf::from(format!("/x/{number}.tar.gz"));
            cache.store(&archive, stamp(1), &entries()).unwrap();
        }
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&cache.dir), 0o700);
        let files: Vec<_> = fs::read_dir(&cache.dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(files.len(), MAX_FILES);
        assert!(files.iter().all(|file| mode(file) == 0o600));
        let _ = fs::remove_dir_all(&cache.dir);
    }
}
