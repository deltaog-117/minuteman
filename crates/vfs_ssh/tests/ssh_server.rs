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

//! Runs the `Vfs` conformance suite, and the cross-machine behaviour, against a real `sshd`
//! started on a loopback port for the test. The server serves this machine's own disk to the
//! current user, so a test can seed files with `std::fs` and read them back over SFTP.
//!
//! A machine without `sshd`, `ssh` or `ssh-keygen` skips these tests (and says so) instead of
//! failing, since they cannot run there.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use shared::conformance::{Seed, check_backend};
use shared::{LocalVfs, Vfs, VfsError};
use vfs_ssh::{ConnectOptions, HostKeys, RoutedVfs};

fn on_path(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .chain(["/usr/sbin", "/sbin", "/usr/local/sbin"])
        .map(|dir| Path::new(dir).join(program))
        .find(|candidate| candidate.is_file())
}

struct Sshd {
    child: Child,
    port: u16,
    dir: PathBuf,
}

impl Sshd {
    /// `None` (after saying why) when the tools are missing.
    fn start(label: &str) -> Option<Self> {
        let (Some(sshd), Some(keygen), Some(_)) =
            (on_path("sshd"), on_path("ssh-keygen"), on_path("ssh"))
        else {
            eprintln!("skipping {label}: sshd, ssh and ssh-keygen are needed");
            return None;
        };
        let dir = std::env::temp_dir().join(format!("mman-sshd-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("a scratch folder");
        for key in ["host_key", "user_key"] {
            let made = Command::new(&keygen)
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(dir.join(key))
                .status()
                .expect("ssh-keygen runs");
            assert!(made.success(), "ssh-keygen made {key}");
        }
        fs::copy(dir.join("user_key.pub"), dir.join("authorized_keys")).expect("authorize the key");
        let port = {
            let probe = TcpListener::bind("127.0.0.1:0").expect("a free port");
            probe.local_addr().expect("an address").port()
        };
        let config = format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {d}/host_key\nAuthorizedKeysFile {d}/authorized_keys\n\
             PasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nStrictModes no\n\
             PidFile {d}/sshd.pid\nSubsystem sftp internal-sftp\n",
            d = dir.display()
        );
        fs::write(dir.join("sshd_config"), config).expect("write the config");
        let child = Command::new(&sshd)
            .args(["-D", "-e", "-f"])
            .arg(dir.join("sshd_config"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("sshd starts");
        let server = Self { child, port, dir };
        let deadline = Instant::now() + Duration::from_secs(10);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "sshd never began listening");
            std::thread::sleep(Duration::from_millis(50));
        }
        Some(server)
    }

    fn options(&self, host_keys: HostKeys) -> ConnectOptions {
        ConnectOptions {
            host_keys,
            known_hosts_file: Some(self.dir.join("known_hosts")),
            identity: Some(self.dir.join("user_key")),
            connect_timeout: Duration::from_secs(10),
        }
    }

    fn vfs(&self) -> RoutedVfs {
        RoutedVfs::with_options(Arc::new(LocalVfs), self.options(HostKeys::AcceptNew))
            .expect("a router")
    }

    /// The path of `local` as seen over SSH.
    fn remote(&self, local: &Path) -> PathBuf {
        PathBuf::from(format!("ssh://127.0.0.1:{}{}", self.port, local.display()))
    }

    fn local_of(&self, remote: &Path) -> PathBuf {
        let prefix = format!("ssh://127.0.0.1:{}", self.port);
        PathBuf::from(
            remote
                .to_str()
                .and_then(|text| text.strip_prefix(&prefix))
                .expect("a path on this server"),
        )
    }
}

impl Drop for Sshd {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Seeds through the server's own disk, since the trait cannot create these.
struct DiskSeed<'a>(&'a Sshd);

impl Seed for DiskSeed<'_> {
    fn dir(&self, path: &Path) {
        fs::create_dir_all(self.0.local_of(path)).expect("make a folder");
    }

    fn file(&self, path: &Path, bytes: &[u8]) {
        let local = self.0.local_of(path);
        fs::create_dir_all(local.parent().expect("a parent")).expect("make parents");
        fs::write(local, bytes).expect("write a file");
    }

    fn symlink(&self, link: &Path, target: &Path) {
        std::os::unix::fs::symlink(self.link_text(target), self.0.local_of(link))
            .expect("make a symlink");
    }

    fn link_text(&self, target: &Path) -> PathBuf {
        let prefix = format!("ssh://127.0.0.1:{}", self.0.port);
        target
            .to_str()
            .and_then(|text| text.strip_prefix(&prefix))
            .map_or_else(|| target.to_path_buf(), PathBuf::from)
    }

    /// SFTP v3 carries times as whole seconds.
    fn time_resolution(&self) -> Duration {
        Duration::from_millis(1100)
    }
}

/// A scratch folder that removes itself, so a failed assertion leaves nothing behind.
struct Scratch(PathBuf);

impl std::ops::Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scratch(label: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("mman-ssh-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("a scratch folder");
    Scratch(dir)
}

#[test]
fn the_remote_backend_meets_the_vfs_contract() {
    let Some(server) = Sshd::start("conformance") else {
        return;
    };
    let work = scratch("conformance");
    let vfs = server.vfs();
    check_backend(&vfs, &DiskSeed(&server), &server.remote(&work));
}

#[test]
fn a_remote_folder_lists_and_a_file_reads_back() {
    let Some(server) = Sshd::start("basic") else {
        return;
    };
    let work = scratch("basic");
    fs::write(work.join("hello.txt"), b"hello over ssh").unwrap();
    fs::create_dir(work.join("sub")).unwrap();
    let vfs = server.vfs();

    let listing = vfs.list_dir(&server.remote(&work)).unwrap();
    let names: Vec<_> = listing.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["sub", "hello.txt"]);
    assert!(listing[0].is_dir);

    let mut text = String::new();
    vfs.open_read(&server.remote(&work.join("hello.txt")))
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert_eq!(text, "hello over ssh");
    assert_eq!(vfs.local_path(&server.remote(&work)), None);
}

#[test]
fn an_unknown_host_is_refused_under_strict_host_keys() {
    let Some(server) = Sshd::start("strict") else {
        return;
    };
    let work = scratch("strict");
    let vfs = RoutedVfs::with_options(Arc::new(LocalVfs), server.options(HostKeys::Strict))
        .expect("a router");

    let outcome = vfs.list_dir(&server.remote(&work));

    match outcome {
        Err(VfsError::Io { source, .. }) => {
            assert_eq!(source.kind(), std::io::ErrorKind::NotConnected, "{source}");
        }
        other => panic!("an unknown host key must be refused, got {other:?}"),
    }
}

#[test]
fn a_machine_named_without_a_path_opens_at_the_login_home() {
    let Some(server) = Sshd::start("home") else {
        return;
    };
    let vfs = server.vfs();
    let typed = PathBuf::from(format!("ssh://127.0.0.1:{}", server.port));

    let opened = vfs.resolve_typed(&typed, Path::new("/"));

    let home = PathBuf::from(std::env::var_os("HOME").expect("a home folder"));
    assert_eq!(server.local_of(&opened), fs::canonicalize(home).unwrap());
    assert_eq!(
        vfs.resolve_typed(&server.remote(Path::new("/tmp")), Path::new("/")),
        server.remote(Path::new("/tmp"))
    );
}

#[test]
fn files_and_folders_copy_between_the_disk_and_a_machine_both_ways() {
    let Some(server) = Sshd::start("copy") else {
        return;
    };
    let work = scratch("copy");
    fs::create_dir_all(work.join("from/inner")).unwrap();
    fs::write(work.join("from/a.txt"), b"alpha").unwrap();
    fs::write(work.join("from/inner/b.txt"), vec![7u8; 600_000]).unwrap();
    let vfs = server.vfs();
    let policy = file_ops::ConflictPolicy::Abort;

    file_ops::copy(
        &vfs,
        &work.join("from"),
        &server.remote(&work.join("to_remote")),
        policy,
    )
    .unwrap();
    assert_eq!(fs::read(work.join("to_remote/a.txt")).unwrap(), b"alpha");
    assert_eq!(
        fs::read(work.join("to_remote/inner/b.txt")).unwrap().len(),
        600_000
    );

    file_ops::copy(
        &vfs,
        &server.remote(&work.join("to_remote")),
        &work.join("back"),
        policy,
    )
    .unwrap();
    assert_eq!(
        fs::read(work.join("back/inner/b.txt")).unwrap(),
        vec![7u8; 600_000]
    );
}

#[test]
fn a_move_between_the_disk_and_a_machine_falls_back_to_copy_and_delete() {
    let Some(server) = Sshd::start("move") else {
        return;
    };
    let work = scratch("move");
    fs::write(work.join("note.txt"), b"moving").unwrap();
    let vfs = server.vfs();

    file_ops::mv(
        &vfs,
        &work.join("note.txt"),
        &server.remote(&work.join("moved.txt")),
        file_ops::ConflictPolicy::Abort,
    )
    .unwrap();

    assert!(!work.join("note.txt").exists());
    assert_eq!(fs::read(work.join("moved.txt")).unwrap(), b"moving");
}

#[test]
fn a_writer_that_is_only_dropped_still_delivers_its_bytes() {
    let Some(server) = Sshd::start("drop") else {
        return;
    };
    let work = scratch("drop");
    let vfs = server.vfs();
    let path = server.remote(&work.join("out.bin"));

    let mut writer = vfs.create_write(&path).unwrap();
    writer.write_all(b"unflushed").unwrap();
    drop(writer);

    assert_eq!(fs::read(work.join("out.bin")).unwrap(), b"unflushed");
}

#[test]
fn browsing_a_remote_folder_works_and_stops_at_the_machines_root() {
    let Some(server) = Sshd::start("browse") else {
        return;
    };
    let work = scratch("browse");
    fs::create_dir_all(work.join("inner")).unwrap();
    fs::write(work.join("inner/leaf.txt"), b"leaf").unwrap();
    let vfs = server.vfs();
    let start = server.remote(&work);

    let mut browser = browser::BrowserState::new(&vfs, start.clone()).unwrap();
    browser.enter(&vfs).unwrap();
    assert_eq!(browser.current_dir(), server.remote(&work.join("inner")));
    assert_eq!(browser.current_entries()[0].name, "leaf.txt");
    browser.leave(&vfs).unwrap();
    assert_eq!(browser.current_dir(), start);

    // `:cd` takes a typed address, relative paths stay on the same machine, and the machine's root
    // has nothing above it: stepping up from there changes nothing.
    browser.goto(&vfs, Path::new("inner")).unwrap();
    assert_eq!(browser.current_dir(), server.remote(&work.join("inner")));
    browser
        .goto(
            &vfs,
            &PathBuf::from(format!("ssh://127.0.0.1:{}/", server.port)),
        )
        .unwrap();
    let root = browser.current_dir().to_path_buf();
    assert!(browser.leave(&vfs).is_err());
    assert_eq!(browser.current_dir(), root);
    assert!(!browser.current_entries().is_empty());
}

#[test]
fn a_host_already_in_known_hosts_connects_under_strict_host_keys() {
    let Some(server) = Sshd::start("trusted") else {
        return;
    };
    let work = scratch("trusted");
    let key = fs::read_to_string(server.dir.join("host_key.pub")).unwrap();
    let mut fields = key.split_whitespace();
    let (kind, material) = (fields.next().unwrap(), fields.next().unwrap());
    fs::write(
        server.dir.join("known_hosts"),
        format!("[127.0.0.1]:{} {kind} {material}\n", server.port),
    )
    .unwrap();
    let vfs = RoutedVfs::with_options(Arc::new(LocalVfs), server.options(HostKeys::Strict))
        .expect("a router");

    let listing = vfs.list_dir(&server.remote(&work));

    assert!(listing.is_ok(), "{listing:?}");
}
