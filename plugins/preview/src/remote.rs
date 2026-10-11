//! Reading files through the host. The plugin has no business opening a path itself — the folder
//! being browsed may be on another machine or inside an archive — so every read is a `read_file`
//! request the host answers from whatever serves the browsed paths. [`RemoteVfs`] puts that behind
//! the `Vfs` trait, which is what lets the archive, hex and text code from before the move run
//! unchanged: they ask for a file, a length and a seek, and this turns each into a request.
//!
//! A request costs a round trip, so a reader fetches a window of the file at a time and answers
//! small reads from it. The window starts small and doubles while reads continue where the last
//! ended, since a tar header is 512 bytes and a text file is read straight through.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use plugin_api::{ChunkKind, FileChunk, METHOD_READ_FILE, ReadFile};
use serde_json::Value;
use shared::{DirEntryInfo, FileKind, Metadata, ReadSeek, ScanEntry, Vfs, VfsError, WriteSeek};

/// What the plugin can ask of the host: one request, answered or refused.
pub trait Remote: Send + Sync {
    /// Sends the request `method` and waits for its reply.
    ///
    /// # Errors
    ///
    /// The host's reason when it refused, or why no answer came (the host went away, or the
    /// answer is no longer wanted).
    fn call(&self, method: &str, params: Value) -> Result<Value, String>;
}

/// The first window a reader fetches, and the window it falls back to after a seek.
const FIRST_WINDOW: usize = 8 << 10;
/// The largest window a reader grows to.
const MAX_WINDOW: usize = 256 << 10;

/// A read-only `Vfs` over a [`Remote`].
pub struct RemoteVfs {
    remote: Arc<dyn Remote>,
}

impl RemoteVfs {
    pub fn new(remote: Arc<dyn Remote>) -> Self {
        Self { remote }
    }

    fn chunk(&self, path: &Path, offset: u64, len: u64) -> Result<FileChunk, VfsError> {
        fetch(self.remote.as_ref(), path, offset, len).map_err(|message| VfsError::Io {
            path: path.to_path_buf(),
            source: io::Error::other(message),
        })
    }
}

fn fetch(remote: &dyn Remote, path: &Path, offset: u64, len: u64) -> Result<FileChunk, String> {
    let params = serde_json::to_value(ReadFile {
        path: path.to_path_buf(),
        offset,
        len,
    })
    .map_err(|e| e.to_string())?;
    let reply = remote.call(METHOD_READ_FILE, params)?;
    serde_json::from_value(reply).map_err(|e| format!("unreadable reply to read_file: {e}"))
}

fn kind_of(kind: ChunkKind) -> FileKind {
    match kind {
        ChunkKind::File => FileKind::File,
        ChunkKind::Dir => FileKind::Dir,
        ChunkKind::Other => FileKind::Other,
    }
}

fn read_only<T>() -> Result<T, VfsError> {
    Err(VfsError::Unsupported("the preview only reads files"))
}

impl Vfs for RemoteVfs {
    fn list_dir(&self, _path: &Path) -> Result<Vec<DirEntryInfo>, VfsError> {
        read_only()
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.chunk(path, 0, 0)
            .is_ok_and(|chunk| chunk.kind == ChunkKind::Dir)
    }

    fn exists(&self, path: &Path) -> bool {
        self.chunk(path, 0, 0).is_ok()
    }

    fn metadata(&self, path: &Path) -> Result<Metadata, VfsError> {
        let chunk = self.chunk(path, 0, 0)?;
        Ok(Metadata {
            kind: kind_of(chunk.kind),
            len: chunk.size,
            allocated: None,
            mode: None,
            uid: None,
            gid: None,
            nlink: None,
            modified: None,
            accessed: None,
            created: None,
            device: None,
            inode: None,
        })
    }

    fn symlink_metadata(&self, path: &Path) -> Result<Metadata, VfsError> {
        // The host follows links, and nothing here looks at a link as a link.
        self.metadata(path)
    }

    fn read_link(&self, _path: &Path) -> Result<PathBuf, VfsError> {
        read_only()
    }

    fn open_read(&self, path: &Path) -> Result<Box<dyn ReadSeek>, VfsError> {
        let chunk = self.chunk(path, 0, 0)?;
        if chunk.kind != ChunkKind::File {
            return Err(VfsError::Io {
                path: path.to_path_buf(),
                source: io::Error::other("not a regular file"),
            });
        }
        Ok(Box::new(RangeReader {
            remote: Arc::clone(&self.remote),
            path: path.to_path_buf(),
            size: chunk.size,
            pos: 0,
            window: Vec::new(),
            window_start: 0,
            next_len: FIRST_WINDOW,
        }))
    }

    fn scan_dir(&self, _path: &Path) -> Result<Vec<ScanEntry>, VfsError> {
        read_only()
    }

    fn local_path(&self, path: &Path) -> Option<PathBuf> {
        self.chunk(path, 0, 0).ok()?.local_path
    }

    fn create_write(&self, _path: &Path) -> Result<Box<dyn WriteSeek>, VfsError> {
        read_only()
    }

    fn create_symlink(&self, _target: &Path, _link: &Path) -> Result<(), VfsError> {
        read_only()
    }

    fn set_mode(&self, _path: &Path, _mode: u32) -> Result<(), VfsError> {
        read_only()
    }

    fn replace(&self, _src: &Path, _dst: &Path) -> Result<(), VfsError> {
        read_only()
    }

    fn create_dir(&self, _path: &Path) -> Result<(), VfsError> {
        read_only()
    }

    fn create_file(&self, _path: &Path) -> Result<(), VfsError> {
        read_only()
    }

    fn touch(&self, _path: &Path) -> Result<(), VfsError> {
        read_only()
    }

    fn copy_file(&self, _src: &Path, _dst: &Path) -> Result<(), VfsError> {
        read_only()
    }

    fn rename(&self, _src: &Path, _dst: &Path) -> Result<(), VfsError> {
        read_only()
    }

    fn remove_file(&self, _path: &Path) -> Result<(), VfsError> {
        read_only()
    }

    fn remove_dir_all(&self, _path: &Path) -> Result<(), VfsError> {
        read_only()
    }
}

/// A file read in windows. `window` holds the bytes that start at `window_start`; a read inside
/// it costs nothing, one outside fetches a new window.
struct RangeReader {
    remote: Arc<dyn Remote>,
    path: PathBuf,
    /// The length the file had when it was opened: reads end there even if it has grown since.
    size: u64,
    pos: u64,
    window: Vec<u8>,
    window_start: u64,
    /// How much the next fetch asks for.
    next_len: usize,
}

impl RangeReader {
    fn window_end(&self) -> u64 {
        self.window_start + self.window.len() as u64
    }

    /// Makes `pos` fall inside `window`, fetching if it does not. An empty window afterwards means
    /// the file ended early (it shrank since it was opened).
    fn fill(&mut self) -> io::Result<()> {
        let inside = self.pos >= self.window_start && self.pos < self.window_end();
        if inside {
            return Ok(());
        }
        // Reading on from where the last window ended is a scan, so ask for more each time; a
        // jump is a seek to a header or a footer, so go back to a small window.
        let continues = !self.window.is_empty() && self.pos == self.window_end();
        self.next_len = if continues {
            (self.next_len * 2).min(MAX_WINDOW)
        } else {
            FIRST_WINDOW
        };
        let wanted = (self.size - self.pos).min(self.next_len as u64);
        let chunk =
            fetch(self.remote.as_ref(), &self.path, self.pos, wanted).map_err(io::Error::other)?;
        self.window = chunk.data;
        self.window_start = self.pos;
        Ok(())
    }
}

impl Read for RangeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.pos >= self.size {
            return Ok(0);
        }
        self.fill()?;
        let from = usize::try_from(self.pos - self.window_start)
            .map_err(|_| io::Error::other("offset out of range"))?;
        let available = self.window.get(from..).unwrap_or_default();
        let count = available.len().min(out.len());
        out[..count].copy_from_slice(&available[..count]);
        self.pos += count as u64;
        Ok(count)
    }
}

impl Seek for RangeReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::Current(delta) => self.pos.checked_add_signed(delta),
            SeekFrom::End(delta) => self.size.checked_add_signed(delta),
        };
        self.pos = target.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek to a position before the start of the file",
            )
        })?;
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use shared::MemVfs;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The host's side of `read_file`, answered from a backend and counting its requests.
    struct Host {
        vfs: MemVfs,
        calls: AtomicUsize,
    }

    impl Host {
        fn new(vfs: MemVfs) -> Arc<Self> {
            Arc::new(Self {
                vfs,
                calls: AtomicUsize::new(0),
            })
        }
    }

    impl Remote for Host {
        fn call(&self, method: &str, params: Value) -> Result<Value, String> {
            assert_eq!(method, METHOD_READ_FILE);
            self.calls.fetch_add(1, Ordering::SeqCst);
            let request: ReadFile = serde_json::from_value(params).map_err(|e| e.to_string())?;
            let meta = self
                .vfs
                .metadata(&request.path)
                .map_err(|e| e.to_string())?;
            let kind = match meta.kind {
                FileKind::File => ChunkKind::File,
                FileKind::Dir => ChunkKind::Dir,
                _ => ChunkKind::Other,
            };
            let mut data = Vec::new();
            if kind == ChunkKind::File && request.len > 0 {
                let mut file = self
                    .vfs
                    .open_read(&request.path)
                    .map_err(|e| e.to_string())?;
                file.seek(SeekFrom::Start(request.offset))
                    .map_err(|e| e.to_string())?;
                file.take(request.len)
                    .read_to_end(&mut data)
                    .map_err(|e| e.to_string())?;
            }
            serde_json::to_value(FileChunk {
                kind,
                size: meta.len,
                local_path: None,
                data,
            })
            .map_err(|e| e.to_string())
        }
    }

    fn remote_with(files: &[(&str, Vec<u8>)]) -> (Arc<Host>, RemoteVfs) {
        let mem = MemVfs::new();
        mem.add_dir("/d");
        for (name, bytes) in files {
            mem.add_file(name, bytes.clone());
        }
        let host = Host::new(mem);
        let vfs = RemoteVfs::new(Arc::clone(&host) as Arc<dyn Remote>);
        (host, vfs)
    }

    #[test]
    fn metadata_and_existence_come_from_a_zero_length_read() {
        let (host, vfs) = remote_with(&[("/f", b"hello".to_vec())]);
        let meta = vfs.metadata(Path::new("/f")).unwrap();
        assert_eq!((meta.kind, meta.len), (FileKind::File, 5));
        assert!(vfs.exists(Path::new("/f")));
        assert!(!vfs.exists(Path::new("/nope")));
        assert!(vfs.is_dir(Path::new("/d")));
        assert!(!vfs.is_dir(Path::new("/f")));
        assert!(vfs.metadata(Path::new("/nope")).is_err());
        assert_eq!(host.calls.load(Ordering::SeqCst), 6);
    }

    #[test]
    fn a_file_reads_back_whole_and_a_folder_does_not_open() {
        let body: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let (_, vfs) = remote_with(&[("/f", body.clone())]);
        let mut read = Vec::new();
        vfs.open_read(Path::new("/f"))
            .unwrap()
            .read_to_end(&mut read)
            .unwrap();
        assert_eq!(read, body);
        assert!(vfs.open_read(Path::new("/d")).is_err());
        assert!(vfs.open_read(Path::new("/nope")).is_err());
    }

    #[test]
    fn writing_is_refused() {
        let (_, vfs) = remote_with(&[]);
        assert!(matches!(
            vfs.create_file(Path::new("/x")),
            Err(VfsError::Unsupported(_))
        ));
        assert!(vfs.remove_file(Path::new("/x")).is_err());
        assert!(vfs.list_dir(Path::new("/")).is_err());
    }

    #[test]
    fn reading_straight_through_costs_few_round_trips() {
        let body = vec![9u8; 1 << 20];
        let (host, vfs) = remote_with(&[("/f", body)]);
        let before = host.calls.load(Ordering::SeqCst);
        let mut reader = io::BufReader::new(vfs.open_read(Path::new("/f")).unwrap());
        io::copy(&mut reader, &mut io::sink()).unwrap();
        let used = host.calls.load(Ordering::SeqCst) - before;
        // One to open, then windows that double from 8 KiB to 256 KiB.
        assert!(used <= 12, "{used} requests for a megabyte");
    }

    #[test]
    fn a_seek_to_the_end_and_back_reads_the_footer() {
        let body: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        let (_, vfs) = remote_with(&[("/f", body.clone())]);
        let mut reader = vfs.open_read(Path::new("/f")).unwrap();
        assert_eq!(reader.seek(SeekFrom::End(-10)).unwrap(), 49_990);
        let mut tail = Vec::new();
        reader.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, &body[49_990..]);
        assert!(reader.seek(SeekFrom::Current(-60_000)).is_err());
        reader.seek(SeekFrom::Start(3)).unwrap();
        let mut three = [0u8; 4];
        reader.read_exact(&mut three).unwrap();
        assert_eq!(three, body[3..7]);
    }

    #[test]
    fn an_archive_is_listed_through_the_host() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("inside.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        io::Write::write_all(&mut writer, b"hi").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let (_, vfs) = remote_with(&[("/a.zip", bytes)]);

        let listing = crate::archive::list(&vfs, Path::new("/a.zip")).expect("a zip lists");
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].name, "inside.txt");
    }

    proptest! {
        /// However the reads and seeks are interleaved, the bytes are the ones a plain in-memory
        /// reader gives, and the windows never make a read return stale or misplaced data.
        #[test]
        fn any_sequence_of_reads_and_seeks_matches_a_plain_reader(
            body in proptest::collection::vec(any::<u8>(), 0..40_000),
            steps in proptest::collection::vec((any::<bool>(), 0u64..45_000, 0usize..9_000), 0..25),
        ) {
            let (_, vfs) = remote_with(&[("/f", body.clone())]);
            let mut remote = vfs.open_read(Path::new("/f")).unwrap();
            let mut reference = Cursor::new(body);
            for (seek, at, count) in steps {
                if seek {
                    prop_assert_eq!(
                        remote.seek(SeekFrom::Start(at)).unwrap(),
                        reference.seek(SeekFrom::Start(at)).unwrap()
                    );
                } else {
                    let mut a = vec![0u8; count];
                    let mut b = vec![0u8; count];
                    let got = remote.read(&mut a).unwrap();
                    let want = reference.read(&mut b).unwrap();
                    // A window may hand back fewer bytes than asked, but never more, and never
                    // none while the file has more; what it hands back is right.
                    prop_assert!(got <= want);
                    prop_assert!(got > 0 || want == 0);
                    prop_assert_eq!(&a[..got], &b[..got]);
                    // Keep the two readers at the same place.
                    reference.seek(SeekFrom::Start(remote.stream_position().unwrap())).unwrap();
                }
            }
        }
    }
}
