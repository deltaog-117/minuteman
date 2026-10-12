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

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use plugin_api::{Answer, Bytes, IoKind, MAX_VFS_READ, Reply};
use proptest::prelude::*;
use serde::Serialize;
use serde_json::Value;
use shared::MemVfs;

use super::*;

const HOME: &str = "ssh://h";

fn at(path: &str) -> PathBuf {
    PathBuf::from(format!("{HOME}{path}"))
}

/// A plugin that serves one machine, `ssh://h`, from memory, and records what it was asked.
struct Stand {
    files: Mutex<BTreeMap<PathBuf, Vec<u8>>>,
    epoch: AtomicU64,
    next_handle: AtomicU64,
    /// What each open handle is for: the file's path, and whether it is being written.
    open: Mutex<HashMap<u64, (PathBuf, bool)>>,
    calls: Mutex<Vec<VfsRequest>>,
    down: AtomicBool,
    /// The folder a bare `ssh://h` opens at, or `None` to fail the request.
    home: Option<PathBuf>,
}

fn json(value: &impl Serialize) -> Result<Value, VfsFault> {
    Ok(serde_json::to_value(value).expect("serializes"))
}

fn empty() -> Result<Value, VfsFault> {
    Ok(Value::Object(serde_json::Map::new()))
}

impl Stand {
    fn new(files: &[(&str, &[u8])]) -> Arc<Self> {
        Arc::new(Self {
            files: Mutex::new(
                files
                    .iter()
                    .map(|(name, bytes)| (at(name), bytes.to_vec()))
                    .collect(),
            ),
            epoch: AtomicU64::new(1),
            next_handle: AtomicU64::new(0),
            open: Mutex::default(),
            calls: Mutex::default(),
            down: AtomicBool::new(false),
            home: Some(at("/home/me")),
        })
    }

    fn calls(&self) -> Vec<VfsRequest> {
        self.calls.lock().unwrap().clone()
    }

    fn count(&self, wanted: impl Fn(&VfsRequest) -> bool) -> usize {
        self.calls().iter().filter(|call| wanted(call)).count()
    }

    fn file(&self, path: &str) -> Option<Vec<u8>> {
        self.files.lock().unwrap().get(&at(path)).cloned()
    }

    fn handle_for(&self, path: PathBuf, writing: bool) -> u64 {
        let handle = self.next_handle.fetch_add(1, Ordering::SeqCst) + 1;
        self.open.lock().unwrap().insert(handle, (path, writing));
        handle
    }

    fn answer(&self, request: &VfsRequest) -> Result<Value, VfsFault> {
        match request {
            VfsRequest::Metadata { path, .. } => {
                let files = self.files.lock().unwrap();
                match files.get(path) {
                    Some(bytes) => json(&WireMeta {
                        kind: WireKind::File,
                        len: bytes.len() as u64,
                        mode: Some(0o100_644),
                        modified: Some(1_700_000_000),
                    }),
                    None => Err(VfsFault::NotFound),
                }
            }
            VfsRequest::Open { path } => {
                let len = self
                    .files
                    .lock()
                    .unwrap()
                    .get(path)
                    .map(Vec::len)
                    .ok_or(VfsFault::NotFound)?;
                json(&Opened {
                    handle: self.handle_for(path.clone(), false),
                    len: len as u64,
                })
            }
            VfsRequest::Read {
                handle,
                offset,
                len,
            } => {
                let (path, _) = self
                    .open
                    .lock()
                    .unwrap()
                    .get(handle)
                    .cloned()
                    .ok_or(VfsFault::BadHandle)?;
                let files = self.files.lock().unwrap();
                let bytes = &files[&path];
                let start = usize::try_from(*offset).unwrap().min(bytes.len());
                let end =
                    (start + usize::try_from((*len).min(MAX_VFS_READ)).unwrap()).min(bytes.len());
                json(&Bytes {
                    data: bytes[start..end].to_vec(),
                })
            }
            VfsRequest::Create { path } => {
                let mut files = self.files.lock().unwrap();
                if files.contains_key(path) {
                    return Err(VfsFault::AlreadyExists);
                }
                files.insert(path.clone(), Vec::new());
                drop(files);
                json(&Opened {
                    handle: self.handle_for(path.clone(), true),
                    len: 0,
                })
            }
            VfsRequest::Write {
                handle,
                offset,
                data,
            } => {
                let (path, writing) = self
                    .open
                    .lock()
                    .unwrap()
                    .get(handle)
                    .cloned()
                    .ok_or(VfsFault::BadHandle)?;
                assert!(writing, "a write names a file being written");
                assert!(data.len() as u64 <= MAX_VFS_READ, "a write fits one window");
                let mut files = self.files.lock().unwrap();
                let bytes = files.get_mut(&path).unwrap();
                let end = usize::try_from(*offset).unwrap() + data.len();
                if bytes.len() < end {
                    bytes.resize(end, 0);
                }
                bytes[usize::try_from(*offset).unwrap()..end].copy_from_slice(data);
                empty()
            }
            VfsRequest::Flush { .. } | VfsRequest::SetMode { .. } => empty(),
            VfsRequest::Close { handle } => {
                self.open.lock().unwrap().remove(handle);
                empty()
            }
            VfsRequest::Rename { from, to } => {
                let mut files = self.files.lock().unwrap();
                let bytes = files.remove(from).ok_or(VfsFault::NotFound)?;
                files.insert(to.clone(), bytes);
                empty()
            }
            VfsRequest::Home { path } => self.home.as_ref().map_or_else(
                || {
                    Err(VfsFault::Io {
                        kind: IoKind::NotConnected,
                        message: format!("cannot connect to {}", path.display()),
                    })
                },
                |home| json(&Resolved { path: home.clone() }),
            ),
            other => panic!("this stand-in does not answer {other:?}"),
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
        self.answer(request)
            .map(|value| Answer { epoch, value })
            .map_err(PeerError::Fault)
    }
}

fn vfs_over(stand: &Arc<Stand>) -> (SshVfs, Arc<MemVfs>) {
    let mem = Arc::new(MemVfs::new());
    mem.add_dir("/x");
    mem.add_file("/x/plain.txt", b"hi".to_vec());
    let peer: Arc<dyn Peer> = Arc::clone(stand) as Arc<dyn Peer>;
    (SshVfs::new(Arc::clone(&mem) as Arc<dyn Vfs>, peer), mem)
}

fn read_all(vfs: &dyn Vfs, path: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    vfs.open_read(path)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

#[test]
fn a_path_on_this_machine_never_reaches_the_plugin() {
    let stand = Stand::new(&[]);
    let (vfs, mem) = vfs_over(&stand);
    assert_eq!(read_all(&vfs, Path::new("/x/plain.txt")), b"hi");
    assert!(vfs.exists(Path::new("/x")));
    vfs.create_dir(Path::new("/x/made")).unwrap();
    assert!(mem.exists(Path::new("/x/made")));
    vfs.copy_file(Path::new("/x/plain.txt"), Path::new("/x/copy.txt"))
        .unwrap();
    assert_eq!(
        vfs.local_path(Path::new("/x")),
        mem.local_path(Path::new("/x"))
    );
    assert_eq!(vfs.local_path(&at("/f")), None);
    assert!(stand.calls().is_empty(), "{:?}", stand.calls());
}

#[test]
fn with_no_plugin_running_a_remote_path_says_why_and_the_rest_still_works() {
    let stand = Stand::new(&[("/f", b"1")]);
    stand.down.store(true, Ordering::SeqCst);
    let (vfs, _mem) = vfs_over(&stand);
    let Err(VfsError::Io { source, .. }) = vfs.list_dir(&at("/")) else {
        panic!("an io error naming the plugin");
    };
    assert!(source.to_string().contains("mman-ssh"), "{source}");
    assert!(!vfs.exists(&at("/f")));
    assert!(!vfs.is_dir(&at("/")));
    assert_eq!(read_all(&vfs, Path::new("/x/plain.txt")), b"hi");
}

#[test]
fn stepping_up_out_of_a_machines_root_names_nothing() {
    let stand = Stand::new(&[]);
    let (vfs, _mem) = vfs_over(&stand);
    let beyond = Path::new("ssh:");
    assert!(matches!(vfs.metadata(beyond), Err(VfsError::NotFound(_))));
    assert!(!vfs.exists(beyond));
    assert!(!vfs.is_dir(beyond));
    assert!(stand.calls().is_empty());
}

#[test]
fn a_move_between_places_is_refused_so_the_caller_copies_and_deletes() {
    let stand = Stand::new(&[("/f", b"1")]);
    let (vfs, _mem) = vfs_over(&stand);
    let crosses = |result: Result<(), VfsError>| {
        matches!(
            result,
            Err(VfsError::Io { source, .. }) if source.kind() == io::ErrorKind::CrossesDevices
        )
    };
    assert!(crosses(vfs.rename(Path::new("/x/plain.txt"), &at("/g"))));
    assert!(crosses(vfs.rename(&at("/f"), Path::new("/x/f"))));
    assert!(crosses(vfs.replace(&at("/f"), Path::new("/x/f"))));
    assert!(crosses(vfs.rename(&at("/f"), Path::new("ssh://other/f"))));
    assert!(stand.calls().is_empty(), "nothing is asked of the plugin");
}

#[test]
fn a_move_within_one_machine_is_one_request() {
    let stand = Stand::new(&[("/f", b"1")]);
    let (vfs, _mem) = vfs_over(&stand);
    vfs.rename(&at("/f"), &at("/g")).unwrap();
    assert_eq!(stand.file("/g"), Some(b"1".to_vec()));
    assert_eq!(stand.calls().len(), 1);
}

#[test]
fn a_copy_between_this_machine_and_a_remote_one_streams_both_ways() {
    let stand = Stand::new(&[]);
    let (vfs, mem) = vfs_over(&stand);
    vfs.copy_file(Path::new("/x/plain.txt"), &at("/up.txt"))
        .unwrap();
    assert_eq!(stand.file("/up.txt"), Some(b"hi".to_vec()));
    vfs.copy_file(&at("/up.txt"), Path::new("/x/down.txt"))
        .unwrap();
    assert_eq!(
        read_all(mem.as_ref(), Path::new("/x/down.txt")),
        b"hi".to_vec()
    );
    let again = vfs.copy_file(Path::new("/x/plain.txt"), &at("/up.txt"));
    assert!(
        matches!(again, Err(VfsError::AlreadyExists(_))),
        "{again:?}"
    );
}

#[test]
fn a_bare_machine_opens_at_the_login_home_and_a_path_is_taken_as_written() {
    let stand = Stand::new(&[]);
    let (vfs, _mem) = vfs_over(&stand);
    let current = Path::new("/x");
    assert_eq!(
        vfs.resolve_typed(Path::new("ssh://h"), current),
        at("/home/me")
    );
    assert_eq!(stand.calls().len(), 1);
    assert_eq!(vfs.resolve_typed(&at("/tmp"), current), at("/tmp"));
    assert_eq!(vfs.resolve_typed(Path::new("ssh://h/"), current), at("/"));
    assert_eq!(
        vfs.resolve_typed(Path::new("sub"), current),
        PathBuf::from("/x/sub")
    );
    assert_eq!(
        vfs.resolve_typed(Path::new("/abs"), current),
        PathBuf::from("/abs")
    );
    assert_eq!(stand.calls().len(), 1, "only the bare machine asked");

    let mut broken = Arc::try_unwrap(Stand::new(&[])).ok().expect("one owner");
    broken.home = None;
    let (vfs, _mem) = vfs_over(&Arc::new(broken));
    assert_eq!(
        vfs.resolve_typed(Path::new("ssh://h"), current),
        PathBuf::from("ssh://h"),
        "a machine that cannot be reached is left as typed, to fail where it is listed"
    );
}

#[test]
fn a_file_written_in_small_pieces_goes_in_few_windows_and_is_closed() {
    let body: Vec<u8> = (0..(1u32 << 20)).map(|i| (i % 251) as u8).collect();
    let stand = Stand::new(&[]);
    let (vfs, _mem) = vfs_over(&stand);
    let mut writer = vfs.create_write(&at("/big")).unwrap();
    for piece in body.chunks(8 << 10) {
        writer.write_all(piece).unwrap();
    }
    writer.flush().unwrap();
    drop(writer);
    assert_eq!(stand.file("/big"), Some(body));
    assert_eq!(stand.count(|r| matches!(r, VfsRequest::Write { .. })), 4);
    assert_eq!(stand.count(|r| matches!(r, VfsRequest::Flush { .. })), 1);
    assert_eq!(stand.count(|r| matches!(r, VfsRequest::Close { .. })), 1);
}

/// A plugin that is slow to answer: a submitted request is run only when its reply is waited for,
/// so the requests a writer has in flight pile up the way they do behind a busy plugin.
struct Lagging {
    stand: Arc<Stand>,
    outstanding: AtomicUsize,
    peak: AtomicUsize,
    /// Fail every write that is waited for.
    refuse_writes: AtomicBool,
}

struct Deferred {
    peer: Arc<Lagging>,
    request: VfsRequest,
    after: Option<Epoch>,
}

impl Reply for Deferred {
    fn wait(self: Box<Self>) -> Result<Answer, PeerError> {
        self.peer.outstanding.fetch_sub(1, Ordering::SeqCst);
        if self.peer.refuse_writes.load(Ordering::SeqCst)
            && matches!(self.request, VfsRequest::Write { .. })
        {
            return Err(PeerError::Fault(VfsFault::Io {
                kind: IoKind::Other,
                message: "disk full".to_string(),
            }));
        }
        self.peer.stand.call(&self.request, self.after)
    }
}

impl Peer for Lagging {
    fn call(&self, request: &VfsRequest, after: Option<Epoch>) -> Result<Answer, PeerError> {
        self.stand.call(request, after)
    }
}

/// `Peer::submit` needs the `Arc` the reply keeps, so the lagging peer is used through this.
struct LaggingPeer(Arc<Lagging>);

impl Peer for LaggingPeer {
    fn call(&self, request: &VfsRequest, after: Option<Epoch>) -> Result<Answer, PeerError> {
        self.0.call(request, after)
    }

    fn submit(
        &self,
        request: &VfsRequest,
        after: Option<Epoch>,
    ) -> Result<Box<dyn Reply>, PeerError> {
        let now = self.0.outstanding.fetch_add(1, Ordering::SeqCst) + 1;
        self.0.peak.fetch_max(now, Ordering::SeqCst);
        Ok(Box::new(Deferred {
            peer: Arc::clone(&self.0),
            request: request.clone(),
            after,
        }))
    }
}

fn lagging_vfs(stand: &Arc<Stand>) -> (SshVfs, Arc<Lagging>) {
    let lagging = Arc::new(Lagging {
        stand: Arc::clone(stand),
        outstanding: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        refuse_writes: AtomicBool::new(false),
    });
    let vfs = SshVfs::new(
        Arc::new(MemVfs::new()) as Arc<dyn Vfs>,
        Arc::new(LaggingPeer(Arc::clone(&lagging))) as Arc<dyn Peer>,
    );
    (vfs, lagging)
}

#[test]
fn windows_are_sent_ahead_of_the_replies_within_a_bound_and_all_arrive() {
    let body: Vec<u8> = (0..(4u32 << 20)).map(|i| (i % 251) as u8).collect();
    let stand = Stand::new(&[]);
    let (vfs, lagging) = lagging_vfs(&stand);
    let mut writer = vfs.create_write(&at("/big")).unwrap();
    for piece in body.chunks(8 << 10) {
        writer.write_all(piece).unwrap();
    }
    writer.flush().unwrap();
    drop(writer);
    let peak = lagging.peak.load(Ordering::SeqCst);
    assert!(peak > 1, "windows overlapped: peak {peak}");
    assert!(peak <= 5, "the bound held: peak {peak}");
    assert_eq!(lagging.outstanding.load(Ordering::SeqCst), 0);
    assert_eq!(stand.file("/big"), Some(body));
}

#[test]
fn a_write_that_fails_after_it_was_sent_is_reported_by_flush() {
    let stand = Stand::new(&[]);
    let (vfs, lagging) = lagging_vfs(&stand);
    lagging.refuse_writes.store(true, Ordering::SeqCst);
    let mut writer = vfs.create_write(&at("/out")).unwrap();
    writer.write_all(b"some bytes").unwrap();
    let error = writer.flush().expect_err("the refused write is reported");
    assert!(error.to_string().contains("disk full"), "{error}");
}

#[test]
fn the_close_waits_for_the_writes_still_on_their_way() {
    let stand = Stand::new(&[]);
    let (vfs, _lagging) = lagging_vfs(&stand);
    let mut writer = vfs.create_write(&at("/out")).unwrap();
    writer.write_all(&vec![7u8; 600 << 10]).unwrap();
    drop(writer);
    let calls = stand.calls();
    let close = calls
        .iter()
        .position(|r| matches!(r, VfsRequest::Close { .. }))
        .expect("closed");
    let last_write = calls
        .iter()
        .rposition(|r| matches!(r, VfsRequest::Write { .. }))
        .expect("written");
    assert!(last_write < close, "{calls:?}");
    assert_eq!(stand.file("/out").map(|f| f.len()), Some(600 << 10));
}

#[test]
fn a_writer_that_is_only_dropped_still_delivers_what_it_holds() {
    let stand = Stand::new(&[]);
    let (vfs, _mem) = vfs_over(&stand);
    let mut writer = vfs.create_write(&at("/out")).unwrap();
    writer.write_all(b"unflushed").unwrap();
    drop(writer);
    assert_eq!(stand.file("/out"), Some(b"unflushed".to_vec()));
    assert_eq!(stand.count(|r| matches!(r, VfsRequest::Close { .. })), 1);
}

#[test]
fn a_new_file_is_refused_where_one_exists() {
    let stand = Stand::new(&[("/there", b"1")]);
    let (vfs, _mem) = vfs_over(&stand);
    assert!(matches!(
        vfs.create_write(&at("/there")),
        Err(VfsError::AlreadyExists(_))
    ));
}

#[test]
fn reading_a_file_straight_through_costs_few_round_trips() {
    let body = vec![9u8; 1 << 20];
    let stand = Stand::new(&[("/big", &body)]);
    let (vfs, _mem) = vfs_over(&stand);
    let mut reader = io::BufReader::new(vfs.open_read(&at("/big")).unwrap());
    io::copy(&mut reader, &mut io::sink()).unwrap();
    drop(reader);
    let reads = stand.count(|r| matches!(r, VfsRequest::Read { .. }));
    assert!(reads <= 8, "{reads} reads for 1 MiB");
    assert_eq!(stand.count(|r| matches!(r, VfsRequest::Close { .. })), 1);
}

#[test]
fn a_reader_from_before_a_restart_is_never_answered_by_the_new_process() {
    let stand = Stand::new(&[("/f", b"abcdef")]);
    let (vfs, _mem) = vfs_over(&stand);
    let mut reader = vfs.open_read(&at("/f")).unwrap();
    stand.epoch.fetch_add(1, Ordering::SeqCst);
    let mut text = String::new();
    let error = reader.read_to_string(&mut text).unwrap_err();
    assert!(error.to_string().contains("restarted"), "{error}");
}

#[test]
fn every_way_a_request_fails_keeps_its_meaning() {
    let path = at("/f");
    let kind_of = |error: PeerError| match error_of(error, &path) {
        VfsError::Io { source, .. } => Some(source.kind()),
        _ => None,
    };
    assert!(matches!(
        error_of(PeerError::Fault(VfsFault::NotFound), &path),
        VfsError::NotFound(_)
    ));
    assert!(matches!(
        error_of(PeerError::Fault(VfsFault::NotADirectory), &path),
        VfsError::NotADirectory(_)
    ));
    assert!(matches!(
        error_of(PeerError::Fault(VfsFault::AlreadyExists), &path),
        VfsError::AlreadyExists(_)
    ));
    assert!(matches!(
        error_of(
            PeerError::Fault(VfsFault::Unsupported { reason: "x".into() }),
            &path
        ),
        VfsError::Unsupported(_)
    ));
    for kind in [
        IoKind::PermissionDenied,
        IoKind::NotConnected,
        IoKind::BrokenPipe,
        IoKind::TimedOut,
    ] {
        let fault = VfsFault::Io {
            kind,
            message: "m".into(),
        };
        assert_eq!(
            kind_of(PeerError::Fault(fault)),
            Some(io::ErrorKind::from(kind))
        );
    }
    assert_eq!(
        kind_of(PeerError::Unavailable),
        Some(io::ErrorKind::Unsupported)
    );
    for error in [PeerError::Stopped, PeerError::TimedOut, PeerError::Stale] {
        assert_eq!(kind_of(error), Some(io::ErrorKind::Other));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// However a file is written (pieces of any size, seeks forward and back), the file the
    /// plugin ends up with is the one a local file would be, and no request is longer than a
    /// window.
    #[test]
    fn a_written_file_matches_the_same_writes_made_locally(
        steps in proptest::collection::vec(
            (proptest::option::of(0u64..700_000), proptest::collection::vec(any::<u8>(), 0..40_000)),
            1..14,
        ),
    ) {
        let stand = Stand::new(&[]);
        let (vfs, _mem) = vfs_over(&stand);
        let mut writer = vfs.create_write(&at("/f")).unwrap();
        let mut model = io::Cursor::new(Vec::<u8>::new());
        for (seek, data) in &steps {
            if let Some(to) = seek {
                writer.seek(SeekFrom::Start(*to)).unwrap();
                model.seek(SeekFrom::Start(*to)).unwrap();
            }
            writer.write_all(data).unwrap();
            model.write_all(data).unwrap();
        }
        writer.flush().unwrap();
        drop(writer);
        prop_assert_eq!(stand.file("/f").unwrap(), model.into_inner());
    }

    /// However a file is read (any offsets, lengths and seeks), the bytes are those of the file.
    #[test]
    fn reads_match_the_file_for_any_seek(
        body in proptest::collection::vec(any::<u8>(), 0..70_000),
        reads in proptest::collection::vec((0u64..80_000, 0usize..9_000), 1..12),
    ) {
        let stand = Stand::new(&[("/f", &body)]);
        let (vfs, _mem) = vfs_over(&stand);
        let mut reader = vfs.open_read(&at("/f")).unwrap();
        for (offset, len) in reads {
            reader.seek(SeekFrom::Start(offset)).unwrap();
            let mut got = vec![0u8; len];
            let mut filled = 0;
            while filled < len {
                let n = reader.read(&mut got[filled..]).unwrap();
                if n == 0 { break; }
                filled += n;
            }
            let start = usize::try_from(offset).unwrap().min(body.len());
            let end = (start + len).min(body.len());
            prop_assert_eq!(&got[..filled], &body[start..end]);
        }
    }
}
