//! The proxy against a stand-in plugin: a fixed tree of files answering the wire requests. What the
//! real plugin does with an archive is tested in its own crate; what is tested here is that every
//! call becomes the right request, every reply the right answer, and that a path outside an
//! archive never reaches the plugin.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use plugin_api::{
    Answer, Bytes, IoKind, LinkTarget, MAX_VFS_READ, Opened, WireKind, WireMeta, WireScan,
};
use proptest::prelude::*;
use serde::Serialize;
use serde_json::Value;
use shared::MemVfs;

use super::*;

const ROOT: &str = "/x/a.zip!";

/// A plugin that serves one archive, `/x/a.zip`, holding the files given to it.
struct Stand {
    files: BTreeMap<String, Vec<u8>>,
    links: BTreeMap<String, String>,
    /// The process the plugin is on: raised by `restart`, as a new process would be.
    epoch: AtomicU64,
    next_handle: AtomicU64,
    open: Mutex<HashMap<u64, Vec<u8>>>,
    calls: Mutex<Vec<VfsRequest>>,
    down: AtomicBool,
    /// When set, every `open` is answered with this failure.
    refuse_open: Mutex<Option<VfsFault>>,
}

impl Stand {
    fn new(files: &[(&str, &[u8])]) -> Arc<Self> {
        Arc::new(Self {
            files: files
                .iter()
                .map(|(name, bytes)| ((*name).to_string(), bytes.to_vec()))
                .collect(),
            links: BTreeMap::new(),
            epoch: AtomicU64::new(1),
            next_handle: AtomicU64::new(0),
            open: Mutex::default(),
            calls: Mutex::default(),
            down: AtomicBool::new(false),
            refuse_open: Mutex::default(),
        })
    }

    fn with_link(files: &[(&str, &[u8])], from: &str, to: &str) -> Arc<Self> {
        let mut stand = Arc::try_unwrap(Self::new(files)).ok().expect("one owner");
        stand.links.insert(from.to_string(), to.to_string());
        Arc::new(stand)
    }

    fn restart(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.open.lock().unwrap().clear();
        self.next_handle.store(0, Ordering::SeqCst);
    }

    fn calls(&self) -> Vec<VfsRequest> {
        self.calls.lock().unwrap().clone()
    }

    fn count(&self, wanted: impl Fn(&VfsRequest) -> bool) -> usize {
        self.calls().iter().filter(|call| wanted(call)).count()
    }

    /// The name of `path` inside the archive, `""` for its root.
    fn entry(path: &Path) -> Option<String> {
        let text = path.to_str()?;
        let rest = text.strip_prefix(ROOT)?;
        Some(rest.trim_start_matches('/').to_string())
    }

    fn is_folder(&self, entry: &str) -> bool {
        entry.is_empty()
            || self
                .files
                .keys()
                .chain(self.links.keys())
                .any(|name| name.starts_with(&format!("{entry}/")))
    }

    fn children(&self, folder: &str) -> Vec<String> {
        let prefix = if folder.is_empty() {
            String::new()
        } else {
            format!("{folder}/")
        };
        let mut names: Vec<String> = self
            .files
            .keys()
            .chain(self.links.keys())
            .filter_map(|name| name.strip_prefix(&prefix))
            .map(|rest| rest.split('/').next().unwrap_or(rest).to_string())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    fn meta(&self, entry: &str, follow: bool) -> Result<WireMeta, VfsFault> {
        let kind_len = if self.is_folder(entry) {
            (WireKind::Dir, 0)
        } else if let Some(bytes) = self.files.get(entry) {
            (WireKind::File, bytes.len() as u64)
        } else if self.links.contains_key(entry) {
            if follow {
                return Err(VfsFault::NotFound);
            }
            (WireKind::Symlink, 0)
        } else {
            return Err(VfsFault::NotFound);
        };
        Ok(WireMeta {
            kind: kind_len.0,
            len: kind_len.1,
            mode: Some(0o100_644),
            modified: Some(1_700_000_000),
        })
    }

    fn reply(value: &impl Serialize) -> Result<Value, PeerError> {
        Ok(serde_json::to_value(value).unwrap())
    }

    fn serve(&self, request: &VfsRequest) -> Result<Value, PeerError> {
        let fault = PeerError::Fault;
        match request {
            VfsRequest::Metadata { path, follow } => {
                let entry = Self::entry(path).ok_or(fault(VfsFault::NotFound))?;
                Self::reply(&self.meta(&entry, *follow).map_err(fault)?)
            }
            VfsRequest::List { path } => {
                let entry = Self::entry(path).ok_or(fault(VfsFault::NotFound))?;
                if !self.is_folder(&entry) {
                    return Err(fault(if self.files.contains_key(&entry) {
                        VfsFault::NotADirectory
                    } else {
                        VfsFault::NotFound
                    }));
                }
                let listed: Vec<WireEntry> = self
                    .children(&entry)
                    .into_iter()
                    .map(|name| {
                        let child = if entry.is_empty() {
                            name.clone()
                        } else {
                            format!("{entry}/{name}")
                        };
                        let is_dir = self.is_folder(&child);
                        WireEntry {
                            name,
                            path: Path::new(ROOT).join(&child),
                            is_dir,
                            size: self.files.get(&child).map_or(0, |b| b.len() as u64),
                            modified: Some(1_700_000_000),
                            mode: Some(0o100_644),
                        }
                    })
                    .collect();
                Self::reply(&listed)
            }
            VfsRequest::Scan { path } => {
                let entry = Self::entry(path).ok_or(fault(VfsFault::NotFound))?;
                let scanned: Vec<WireScan> = self
                    .children(&entry)
                    .into_iter()
                    .map(|name| {
                        let child = if entry.is_empty() {
                            name.clone()
                        } else {
                            format!("{entry}/{name}")
                        };
                        WireScan {
                            meta: self.meta(&child, false).ok(),
                            path: Path::new(ROOT).join(&child),
                            name,
                        }
                    })
                    .collect();
                Self::reply(&scanned)
            }
            VfsRequest::ReadLink { path } => {
                let entry = Self::entry(path).ok_or(fault(VfsFault::NotFound))?;
                let target = self.links.get(&entry).ok_or(fault(VfsFault::Io {
                    kind: IoKind::InvalidInput,
                    message: "not a symbolic link".into(),
                }))?;
                Self::reply(&LinkTarget {
                    target: PathBuf::from(target),
                })
            }
            VfsRequest::Open { path } => {
                if let Some(refusal) = self.refuse_open.lock().unwrap().clone() {
                    return Err(fault(refusal));
                }
                let entry = Self::entry(path).ok_or(fault(VfsFault::NotFound))?;
                let bytes = self.files.get(&entry).ok_or(fault(VfsFault::NotFound))?;
                let handle = self.next_handle.fetch_add(1, Ordering::SeqCst) + 1;
                self.open.lock().unwrap().insert(handle, bytes.clone());
                Self::reply(&Opened {
                    handle,
                    len: bytes.len() as u64,
                })
            }
            VfsRequest::Read {
                handle,
                offset,
                len,
            } => {
                let open = self.open.lock().unwrap();
                let bytes = open.get(handle).ok_or(fault(VfsFault::BadHandle))?;
                let start = usize::try_from(*offset).unwrap().min(bytes.len());
                let end =
                    (start + usize::try_from((*len).min(MAX_VFS_READ)).unwrap()).min(bytes.len());
                Self::reply(&Bytes {
                    data: bytes[start..end].to_vec(),
                })
            }
            VfsRequest::Close { handle } => {
                self.open.lock().unwrap().remove(handle);
                Self::reply(&serde_json::json!({}))
            }
            // The host never sends these to the plugin that serves archives.
            _ => Err(fault(VfsFault::Unsupported {
                reason: "an archive is read-only".into(),
            })),
        }
    }
}

impl Peer for Stand {
    fn call(&self, request: &VfsRequest, after: Option<Epoch>) -> Result<Answer, PeerError> {
        self.calls.lock().unwrap().push(request.clone());
        if self.down.load(Ordering::SeqCst) {
            return Err(PeerError::Unavailable);
        }
        let epoch = Epoch(self.epoch.load(Ordering::SeqCst));
        if after.is_some_and(|after| after != epoch) {
            return Err(PeerError::Stale);
        }
        self.serve(request).map(|value| Answer { epoch, value })
    }
}

fn vfs_over(stand: &Arc<Stand>) -> (ArchiveVfs, Arc<MemVfs>) {
    let mem = Arc::new(MemVfs::new());
    mem.add_dir("/x");
    mem.add_file("/x/plain.txt", b"hi".to_vec());
    let peer: Arc<dyn Peer> = stand.clone();
    (ArchiveVfs::new(mem.clone(), peer), mem)
}

fn read_all(vfs: &ArchiveVfs, path: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    vfs.open_read(Path::new(path))
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

#[test]
fn a_path_outside_an_archive_never_reaches_the_plugin() {
    let stand = Stand::new(&[("f", b"1")]);
    let (vfs, _mem) = vfs_over(&stand);
    let plain = Path::new("/x/plain.txt");
    assert_eq!(read_all(&vfs, "/x/plain.txt"), b"hi");
    assert!(vfs.exists(plain) && !vfs.is_dir(plain));
    assert_eq!(vfs.list_dir(Path::new("/x")).unwrap().len(), 1);
    assert_eq!(vfs.metadata(plain).unwrap().len, 2);
    // An archive file is a file until it is entered with the mark.
    assert!(!vfs.is_dir(Path::new("/x/a.zip")));
    assert!(stand.calls().is_empty(), "{:?}", stand.calls());
}

#[test]
fn a_climb_out_of_the_archive_is_not_found_without_asking() {
    let stand = Stand::new(&[("f", b"1")]);
    let (vfs, _mem) = vfs_over(&stand);
    assert!(matches!(
        vfs.list_dir(Path::new("/x/a.zip!/..")),
        Err(VfsError::NotFound(_))
    ));
    assert!(!vfs.exists(Path::new("/x/a.zip!/..")));
    assert!(!vfs.is_dir(Path::new("/x/a.zip!/..")));
    assert!(stand.calls().is_empty());
}

#[test]
fn listing_and_describing_inside_an_archive_go_to_the_plugin() {
    let stand = Stand::new(&[("d/f.txt", b"hello"), ("top.txt", b"abc")]);
    let (vfs, _mem) = vfs_over(&stand);
    let root = Path::new(ROOT);

    let listed = vfs.list_dir(root).unwrap();
    let names: Vec<&str> = listed.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["d", "top.txt"]);
    assert!(listed[0].is_dir);
    assert_eq!(listed[1].size, 3);
    assert_eq!(listed[1].path, root.join("top.txt"));
    assert_eq!(
        listed[1].modified,
        Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000))
    );

    assert!(vfs.is_dir(&root.join("d")));
    assert!(!vfs.is_dir(&root.join("top.txt")));
    assert!(vfs.exists(&root.join("d/f.txt")));
    assert!(!vfs.exists(&root.join("nope")));
    let meta = vfs.metadata(&root.join("d/f.txt")).unwrap();
    assert_eq!(
        (meta.kind, meta.len, meta.mode),
        (FileKind::File, 5, Some(0o100_644))
    );

    let scanned = vfs.scan_dir(&root.join("d")).unwrap();
    assert_eq!(scanned[0].name, "f.txt");
    assert_eq!(scanned[0].metadata.as_ref().map(|m| m.len), Some(5));
}

#[test]
fn a_link_is_described_as_a_link_and_followed_only_when_asked() {
    let stand = Stand::with_link(&[("real", b"abcd")], "alias", "real");
    let (vfs, _mem) = vfs_over(&stand);
    let alias = Path::new("/x/a.zip!/alias");
    assert_eq!(vfs.symlink_metadata(alias).unwrap().kind, FileKind::Symlink);
    assert!(vfs.metadata(alias).is_err());
    assert_eq!(vfs.read_link(alias).unwrap(), Path::new("real"));
    assert!(vfs.read_link(Path::new("/x/a.zip!/real")).is_err());
}

#[test]
fn failures_keep_their_meaning_on_the_way_back() {
    let stand = Stand::new(&[("f.txt", b"1")]);
    let (vfs, _mem) = vfs_over(&stand);
    assert!(matches!(
        vfs.list_dir(Path::new("/x/a.zip!/nope")),
        Err(VfsError::NotFound(_))
    ));
    assert!(matches!(
        vfs.list_dir(Path::new("/x/a.zip!/f.txt")),
        Err(VfsError::NotADirectory(_))
    ));

    *stand.refuse_open.lock().unwrap() = Some(VfsFault::Io {
        kind: IoKind::PermissionDenied,
        message: "encrypted: extract it to give the password".into(),
    });
    let Err(VfsError::Io { source, .. }) = vfs.open_read(Path::new("/x/a.zip!/f.txt")) else {
        panic!("a refused open is an io error");
    };
    assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
    assert!(source.to_string().contains("extract it"));
}

#[test]
fn every_way_of_changing_an_archive_is_refused_without_asking_and_nothing_else_is() {
    let stand = Stand::new(&[("f", b"1")]);
    let (vfs, mem) = vfs_over(&stand);
    let inside = Path::new("/x/a.zip!/new");
    let refused = |result: Result<(), VfsError>| {
        assert!(
            matches!(result, Err(VfsError::Unsupported(_))),
            "{result:?}"
        );
    };
    refused(vfs.create_dir(inside));
    refused(vfs.create_file(inside));
    refused(vfs.touch(inside));
    refused(vfs.remove_file(Path::new("/x/a.zip!/f")));
    refused(vfs.remove_dir_all(Path::new(ROOT)));
    refused(vfs.rename(Path::new("/x/a.zip!/f"), Path::new("/x/out")));
    refused(vfs.rename(Path::new("/x/plain.txt"), inside));
    refused(vfs.replace(Path::new("/x/plain.txt"), inside));
    refused(vfs.copy_file(Path::new("/x/plain.txt"), inside));
    refused(vfs.set_mode(Path::new("/x/a.zip!/f"), 0o600));
    refused(vfs.create_symlink(Path::new("t"), inside));
    assert!(matches!(
        vfs.create_write(inside),
        Err(VfsError::Unsupported(_))
    ));
    assert!(stand.calls().is_empty());

    // Beside the archive, everything still works.
    vfs.create_dir(Path::new("/x/made")).unwrap();
    assert!(mem.exists(Path::new("/x/made")));
    assert_eq!(vfs.local_path(Path::new(ROOT)), None);
}

#[test]
fn with_no_plugin_running_an_archive_says_why_and_the_rest_still_works() {
    let stand = Stand::new(&[("f", b"1")]);
    stand.down.store(true, Ordering::SeqCst);
    let (vfs, _mem) = vfs_over(&stand);
    let Err(VfsError::Io { source, .. }) = vfs.list_dir(Path::new(ROOT)) else {
        panic!("an io error naming the plugin");
    };
    assert!(source.to_string().contains("mman-archive"), "{source}");
    assert!(!vfs.exists(Path::new(ROOT)));
    assert!(!vfs.is_dir(Path::new(ROOT)));
    assert_eq!(read_all(&vfs, "/x/plain.txt"), b"hi");
}

#[test]
fn reading_a_file_straight_through_costs_few_round_trips() {
    let body = vec![9u8; 1 << 20];
    let stand = Stand::new(&[("big", &body)]);
    let (vfs, _mem) = vfs_over(&stand);
    let mut reader = io::BufReader::new(vfs.open_read(Path::new("/x/a.zip!/big")).unwrap());
    io::copy(&mut reader, &mut io::sink()).unwrap();
    let reads = stand.count(|call| matches!(call, VfsRequest::Read { .. }));
    // Windows that double from 16 KiB to the cap, so a megabyte is a handful of requests.
    assert!(reads <= 8, "{reads} reads for a megabyte");
}

#[test]
fn a_seek_to_the_end_and_back_reads_the_footer() {
    let body: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
    let stand = Stand::new(&[("f", &body)]);
    let (vfs, _mem) = vfs_over(&stand);
    let mut reader = vfs.open_read(Path::new("/x/a.zip!/f")).unwrap();
    assert_eq!(reader.seek(SeekFrom::End(-10)).unwrap(), 49_990);
    let mut tail = Vec::new();
    reader.read_to_end(&mut tail).unwrap();
    assert_eq!(tail, &body[49_990..]);
    assert!(reader.seek(SeekFrom::Current(-60_000)).is_err());
    reader.seek(SeekFrom::Start(3)).unwrap();
    let mut four = [0u8; 4];
    reader.read_exact(&mut four).unwrap();
    assert_eq!(four, body[3..7]);
}

#[test]
fn dropping_a_reader_lets_go_of_the_file() {
    let stand = Stand::new(&[("f", b"hello")]);
    let (vfs, _mem) = vfs_over(&stand);
    let reader = vfs.open_read(Path::new("/x/a.zip!/f")).unwrap();
    assert_eq!(stand.open.lock().unwrap().len(), 1);
    drop(reader);
    assert!(stand.open.lock().unwrap().is_empty());
    assert_eq!(
        stand.count(|call| matches!(call, VfsRequest::Close { .. })),
        1
    );
}

#[test]
fn a_reader_does_not_survive_a_restart_of_the_plugin() {
    let stand = Stand::new(&[("f", b"hello world")]);
    let (vfs, _mem) = vfs_over(&stand);
    let mut reader = vfs.open_read(Path::new("/x/a.zip!/f")).unwrap();
    stand.restart();
    // The new process numbers handles from 1 again: the old reader's handle would name a file the
    // new process opened for someone else, if the request were let through.
    let other = vfs.open_read(Path::new("/x/a.zip!/f")).unwrap();
    let mut out = Vec::new();
    let error = reader.read_to_end(&mut out).unwrap_err();
    assert!(out.is_empty(), "no bytes from a stale handle");
    assert!(error.to_string().contains("restarted"), "{error}");
    drop(other);
}

#[test]
fn a_short_answer_before_the_end_is_an_error_not_a_loop() {
    struct Short(Arc<Stand>);
    impl Peer for Short {
        fn call(&self, request: &VfsRequest, after: Option<Epoch>) -> Result<Answer, PeerError> {
            if matches!(request, VfsRequest::Read { .. }) {
                return Ok(Answer {
                    epoch: Epoch(1),
                    value: serde_json::to_value(Bytes { data: Vec::new() }).unwrap(),
                });
            }
            self.0.call(request, after)
        }
    }
    let stand = Stand::new(&[("f", b"hello")]);
    let mem = Arc::new(MemVfs::new());
    let vfs = ArchiveVfs::new(mem, Arc::new(Short(stand)));
    let mut out = Vec::new();
    let error = vfs
        .open_read(Path::new("/x/a.zip!/f"))
        .unwrap()
        .read_to_end(&mut out)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn copying_out_makes_an_identical_new_file_and_never_overwrites() {
    let data: Vec<u8> = (0..250u8).collect();
    let stand = Stand::new(&[("d/f.bin", &data)]);
    let (vfs, mem) = vfs_over(&stand);
    let from = Path::new("/x/a.zip!/d/f.bin");
    let out = Path::new("/x/copy.bin");
    vfs.copy_file(from, out).unwrap();
    assert_eq!(read_all(&vfs, "/x/copy.bin"), data);
    assert!(mem.exists(out));
    assert!(matches!(
        vfs.copy_file(from, out),
        Err(VfsError::AlreadyExists(_))
    ));
    assert!(matches!(
        vfs.copy_file(Path::new("/x/a.zip!/nope"), Path::new("/x/other")),
        Err(VfsError::NotFound(_))
    ));
    assert!(
        !mem.exists(Path::new("/x/other")),
        "a failed copy leaves nothing"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// However reads and seeks are interleaved, the bytes are the ones a plain in-memory reader
    /// gives: a window never makes a read return stale or misplaced data.
    #[test]
    fn any_sequence_of_reads_and_seeks_matches_a_plain_reader(
        body in proptest::collection::vec(any::<u8>(), 0..80_000),
        steps in proptest::collection::vec((any::<bool>(), 0u64..90_000, 0usize..20_000), 0..25),
    ) {
        let stand = Stand::new(&[("f", &body)]);
        let (vfs, _mem) = vfs_over(&stand);
        let mut remote = vfs.open_read(Path::new("/x/a.zip!/f")).unwrap();
        let mut reference = io::Cursor::new(body);
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
                prop_assert!(got <= want);
                prop_assert!(got > 0 || want == 0);
                prop_assert_eq!(&a[..got], &b[..got]);
                reference.seek(SeekFrom::Start(remote.stream_position().unwrap())).unwrap();
            }
        }
    }

    /// Routing is decided by the path's text alone, so a path with no archive in it never costs a
    /// request, whatever it looks like.
    #[test]
    fn a_path_with_no_archive_in_it_is_never_sent(text in "/[a-z/ ._-]{0,30}") {
        let stand = Stand::new(&[("f", b"1")]);
        let (vfs, _mem) = vfs_over(&stand);
        let path = Path::new(&text);
        let _ = vfs.list_dir(path);
        let _ = vfs.exists(path);
        let _ = vfs.metadata(path);
        prop_assert!(stand.calls().is_empty());
    }
}
