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

//! The plugin's `Server` answering requests against a real `sshd` on a loopback port, which
//! serves this machine's own disk, so a test can seed files with `std::fs` and check what the
//! plugin left there. A machine without `sshd` skips the tests that need it (and says so).

#[path = "../../../crates/vfs_remote/tests/harness/sshd.rs"]
mod sshd;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use plugin_api::{
    Bytes, MAX_OPEN_HANDLES, MAX_VFS_READ, Opened, Resolved, VfsFault, VfsRequest, WireEntry,
    WireKind, WireMeta,
};
use proptest::prelude::*;
use remoter::{ConnectOptions, HostKeys, Server};
use serde::de::DeserializeOwned;
use sshd::{Sshd, scratch};

fn server_for(sshd: &Sshd) -> Server {
    Server::new(ConnectOptions {
        host_keys: HostKeys::AcceptNew,
        known_hosts_file: Some(sshd.dir.join("known_hosts")),
        identity: Some(sshd.dir.join("user_key")),
        connect_timeout: Duration::from_secs(10),
        asker: remoter::Asker::default(),
        askpass_program: None,
    })
    .expect("a server")
}

fn ask<T: DeserializeOwned>(server: &Server, request: VfsRequest) -> Result<T, VfsFault> {
    server
        .answer(&request)
        .map(|value| serde_json::from_value(value).expect("the reply has the shape asked"))
}

fn nothing(server: &Server, request: VfsRequest) -> Result<(), VfsFault> {
    server.answer(&request).map(|_| ())
}

fn create(server: &Server, path: &Path) -> Result<Opened, VfsFault> {
    ask(
        server,
        VfsRequest::Create {
            path: path.to_path_buf(),
        },
    )
}

fn write(server: &Server, handle: u64, offset: u64, data: &[u8]) -> Result<(), VfsFault> {
    nothing(
        server,
        VfsRequest::Write {
            handle,
            offset,
            data: data.to_vec(),
        },
    )
}

fn open(server: &Server, path: &Path) -> Result<Opened, VfsFault> {
    ask(
        server,
        VfsRequest::Open {
            path: path.to_path_buf(),
        },
    )
}

fn read(server: &Server, handle: u64, offset: u64, len: u64) -> Result<Vec<u8>, VfsFault> {
    ask::<Bytes>(
        server,
        VfsRequest::Read {
            handle,
            offset,
            len,
        },
    )
    .map(|bytes| bytes.data)
}

fn close(server: &Server, handle: u64) {
    nothing(server, VfsRequest::Close { handle }).expect("close never fails");
}

#[test]
fn a_path_that_is_not_a_remote_address_is_refused_without_connecting() {
    let server = Server::new(ConnectOptions::default()).expect("a server");
    let refused = server.answer(&VfsRequest::List {
        path: "/etc".into(),
    });
    assert!(
        matches!(refused, Err(VfsFault::Unsupported { .. })),
        "{refused:?}"
    );
    let beyond = server.answer(&VfsRequest::Metadata {
        path: "ssh:".into(),
        follow: true,
    });
    assert_eq!(beyond, Err(VfsFault::NotFound));
    let malformed = server.answer(&VfsRequest::List {
        path: "ssh://bad host/x".into(),
    });
    assert!(
        matches!(malformed, Err(VfsFault::Io { .. })),
        "{malformed:?}"
    );
    assert_eq!(server.open_count(), 0);
}

#[test]
fn a_move_between_two_machines_is_refused() {
    let server = Server::new(ConnectOptions::default()).expect("a server");
    for request in [
        VfsRequest::Rename {
            from: "ssh://a/x".into(),
            to: "ssh://b/x".into(),
        },
        VfsRequest::Replace {
            from: "ssh://a/x".into(),
            to: "ssh://b/x".into(),
        },
        VfsRequest::CopyFile {
            from: "ssh://a/x".into(),
            to: "ssh://b/x".into(),
        },
    ] {
        let refused = server.answer(&request);
        assert!(
            matches!(refused, Err(VfsFault::Unsupported { .. })),
            "{request:?}: {refused:?}"
        );
    }
}

#[test]
fn a_folder_is_listed_described_and_resolved_over_the_wire() {
    let Some(sshd) = Sshd::start("serve-list") else {
        return;
    };
    let work = scratch("serve-list");
    fs::write(work.join("hello.txt"), b"hello").unwrap();
    fs::create_dir(work.join("sub")).unwrap();
    std::os::unix::fs::symlink("hello.txt", work.join("link")).unwrap();
    let server = server_for(&sshd);

    let listed: Vec<WireEntry> = ask(
        &server,
        VfsRequest::List {
            path: sshd.remote(&work),
        },
    )
    .unwrap();
    let names: Vec<_> = listed.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["sub", "hello.txt", "link"]);
    assert!(listed[0].is_dir);
    assert_eq!(listed[1].path, sshd.remote(&work.join("hello.txt")));

    let meta: WireMeta = ask(
        &server,
        VfsRequest::Metadata {
            path: sshd.remote(&work.join("hello.txt")),
            follow: true,
        },
    )
    .unwrap();
    assert_eq!((meta.kind, meta.len), (WireKind::File, 5));
    let itself: WireMeta = ask(
        &server,
        VfsRequest::Metadata {
            path: sshd.remote(&work.join("link")),
            follow: false,
        },
    )
    .unwrap();
    assert_eq!(itself.kind, WireKind::Symlink);

    let target: plugin_api::LinkTarget = ask(
        &server,
        VfsRequest::ReadLink {
            path: sshd.remote(&work.join("link")),
        },
    )
    .unwrap();
    assert_eq!(target.target, PathBuf::from("hello.txt"));

    // A folder that is not there lists as "not a directory", as on the local disk; asking about
    // the entry itself says it is missing.
    let missing = server.answer(&VfsRequest::List {
        path: sshd.remote(&work.join("nope")),
    });
    assert_eq!(missing, Err(VfsFault::NotADirectory));
    let absent = server.answer(&VfsRequest::Metadata {
        path: sshd.remote(&work.join("nope")),
        follow: true,
    });
    assert_eq!(absent, Err(VfsFault::NotFound));

    let home: Resolved = ask(
        &server,
        VfsRequest::Home {
            path: PathBuf::from(format!("ssh://127.0.0.1:{}", sshd.port)),
        },
    )
    .unwrap();
    let home_dir = PathBuf::from(std::env::var_os("HOME").expect("a home folder"));
    assert_eq!(
        sshd.local_of(&home.path),
        fs::canonicalize(home_dir).unwrap()
    );
}

#[test]
fn a_new_file_is_written_at_offsets_and_a_read_handle_cannot_write() {
    let Some(sshd) = Sshd::start("serve-write") else {
        return;
    };
    let work = scratch("serve-write");
    let server = server_for(&sshd);
    let path = sshd.remote(&work.join("out.bin"));

    let created = create(&server, &path).unwrap();
    assert_eq!(created.len, 0);
    write(&server, created.handle, 0, b"abcdef").unwrap();
    // A zip goes back to patch a header it wrote first.
    write(&server, created.handle, 2, b"XY").unwrap();
    nothing(
        &server,
        VfsRequest::Flush {
            handle: created.handle,
        },
    )
    .unwrap();
    close(&server, created.handle);
    assert_eq!(fs::read(work.join("out.bin")).unwrap(), b"abXYef");

    assert_eq!(create(&server, &path), Err(VfsFault::AlreadyExists));

    let opened = open(&server, &path).unwrap();
    assert_eq!(opened.len, 6);
    assert_eq!(read(&server, opened.handle, 1, 3).unwrap(), b"bXY");
    assert_eq!(
        write(&server, opened.handle, 0, b"no"),
        Err(VfsFault::BadHandle),
        "a file opened to read cannot be written"
    );
    assert_eq!(
        nothing(
            &server,
            VfsRequest::Flush {
                handle: opened.handle
            }
        ),
        Err(VfsFault::BadHandle)
    );
    let writing = create(&server, &sshd.remote(&work.join("w.bin"))).unwrap();
    assert_eq!(
        read(&server, writing.handle, 0, 1),
        Err(VfsFault::BadHandle),
        "a file opened to write cannot be read"
    );
    close(&server, opened.handle);
    assert_eq!(read(&server, opened.handle, 0, 1), Err(VfsFault::BadHandle));
    close(&server, opened.handle);
    close(&server, writing.handle);
    assert_eq!(server.open_count(), 0);
}

#[test]
fn a_file_closed_without_a_flush_still_reaches_the_disk() {
    let Some(sshd) = Sshd::start("serve-close") else {
        return;
    };
    let work = scratch("serve-close");
    let server = server_for(&sshd);
    let created = create(&server, &sshd.remote(&work.join("f"))).unwrap();
    write(&server, created.handle, 0, b"kept").unwrap();
    close(&server, created.handle);
    assert_eq!(fs::read(work.join("f")).unwrap(), b"kept");
}

#[test]
fn one_read_never_returns_more_than_the_cap() {
    let Some(sshd) = Sshd::start("serve-cap") else {
        return;
    };
    let work = scratch("serve-cap");
    fs::write(work.join("big"), vec![7u8; (MAX_VFS_READ as usize) * 2]).unwrap();
    let server = server_for(&sshd);
    let opened = open(&server, &sshd.remote(&work.join("big"))).unwrap();
    let got = read(&server, opened.handle, 0, u64::MAX).unwrap();
    assert_eq!(got.len() as u64, MAX_VFS_READ);
    let too_long = write(
        &server,
        create(&server, &sshd.remote(&work.join("w")))
            .unwrap()
            .handle,
        0,
        &vec![0u8; MAX_VFS_READ as usize + 1],
    );
    assert!(matches!(too_long, Err(VfsFault::Io { .. })), "{too_long:?}");
}

#[test]
fn too_many_open_files_are_refused_until_one_is_closed() {
    let Some(sshd) = Sshd::start("serve-full") else {
        return;
    };
    let work = scratch("serve-full");
    fs::write(work.join("f"), b"1").unwrap();
    let server = server_for(&sshd);
    let path = sshd.remote(&work.join("f"));
    let handles: Vec<u64> = (0..MAX_OPEN_HANDLES)
        .map(|_| open(&server, &path).unwrap().handle)
        .collect();
    assert_eq!(open(&server, &path), Err(VfsFault::TooManyOpen));
    assert_eq!(
        create(&server, &sshd.remote(&work.join("new"))),
        Err(VfsFault::TooManyOpen)
    );
    assert!(
        !work.join("new").exists(),
        "a refused create leaves nothing behind"
    );
    close(&server, handles[0]);
    assert!(open(&server, &path).is_ok());
}

#[test]
fn the_requests_that_change_things_change_the_remote_disk() {
    let Some(sshd) = Sshd::start("serve-change") else {
        return;
    };
    let work = scratch("serve-change");
    fs::write(work.join("a"), b"alpha").unwrap();
    let server = server_for(&sshd);
    let at = |name: &str| sshd.remote(&work.join(name));

    nothing(&server, VfsRequest::CreateDir { path: at("d") }).unwrap();
    assert!(work.join("d").is_dir());
    nothing(&server, VfsRequest::CreateFile { path: at("e") }).unwrap();
    assert_eq!(
        nothing(&server, VfsRequest::CreateFile { path: at("e") }),
        Err(VfsFault::AlreadyExists)
    );
    nothing(&server, VfsRequest::Touch { path: at("t") }).unwrap();
    assert!(work.join("t").is_file());
    nothing(
        &server,
        VfsRequest::CopyFile {
            from: at("a"),
            to: at("b"),
        },
    )
    .unwrap();
    assert_eq!(fs::read(work.join("b")).unwrap(), b"alpha");
    nothing(
        &server,
        VfsRequest::Rename {
            from: at("b"),
            to: at("c"),
        },
    )
    .unwrap();
    assert!(!work.join("b").exists() && work.join("c").exists());
    assert_eq!(
        nothing(
            &server,
            VfsRequest::Rename {
                from: at("c"),
                to: at("a")
            }
        ),
        Err(VfsFault::AlreadyExists),
        "a move never replaces"
    );
    nothing(
        &server,
        VfsRequest::Replace {
            from: at("c"),
            to: at("a"),
        },
    )
    .unwrap();
    assert_eq!(fs::read(work.join("a")).unwrap(), b"alpha");
    assert!(!work.join("c").exists());
    nothing(
        &server,
        VfsRequest::SetMode {
            path: at("a"),
            mode: 0o600,
        },
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(work.join("a")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    nothing(
        &server,
        VfsRequest::Symlink {
            target: "a".into(),
            link: at("ln"),
        },
    )
    .unwrap();
    assert_eq!(fs::read_link(work.join("ln")).unwrap(), PathBuf::from("a"));
    nothing(&server, VfsRequest::RemoveFile { path: at("a") }).unwrap();
    assert!(!work.join("a").exists());
    fs::create_dir_all(work.join("tree/inner")).unwrap();
    fs::write(work.join("tree/inner/f"), b"x").unwrap();
    nothing(&server, VfsRequest::RemoveDirAll { path: at("tree") }).unwrap();
    assert!(!work.join("tree").exists());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    /// However a new file is written (windows at any offsets, overlapping or past the end), the
    /// file that is left on the remote disk is the one the same writes make locally.
    #[test]
    fn a_file_written_by_offset_matches_the_same_writes_made_locally(
        writes in proptest::collection::vec(
            (0u64..100_000, proptest::collection::vec(any::<u8>(), 0..6_000)),
            1..10,
        ),
    ) {
        let Some(sshd) = Sshd::start("serve-prop") else {
            return Ok(());
        };
        let work = scratch("serve-prop");
        let server = server_for(&sshd);
        let created = create(&server, &sshd.remote(&work.join("f"))).unwrap();
        let mut model = Vec::<u8>::new();
        for (offset, data) in &writes {
            write(&server, created.handle, *offset, data).unwrap();
            let end = usize::try_from(*offset).unwrap() + data.len();
            if model.len() < end {
                model.resize(end, 0);
            }
            model[usize::try_from(*offset).unwrap()..end].copy_from_slice(data);
        }
        close(&server, created.handle);
        prop_assert_eq!(fs::read(work.join("f")).unwrap(), model);
    }
}
