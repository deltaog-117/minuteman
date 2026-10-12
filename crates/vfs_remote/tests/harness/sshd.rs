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

//! A real `sshd` on a loopback port serving this machine's own disk to the current user, and a
//! scratch folder that removes itself. Shared by the tests of the plugin and of the host side.

#![allow(dead_code)]

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn on_path(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .chain(["/usr/sbin", "/sbin", "/usr/local/sbin"])
        .map(|dir| Path::new(dir).join(program))
        .find(|candidate| candidate.is_file())
}

pub struct Sshd {
    child: Child,
    pub port: u16,
    pub dir: PathBuf,
}

impl Sshd {
    /// `None` (after saying why) when the tools are missing.
    pub fn start(label: &str) -> Option<Self> {
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
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {d}/host_key\nAuthorizedKeysFile {d}/authorized_keys\n             PasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nStrictModes no\n             PidFile {d}/sshd.pid\nSubsystem sftp internal-sftp\n",
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
            thread::sleep(Duration::from_millis(50));
        }
        Some(server)
    }

    /// The environment that points the plugin at this server's key and `known_hosts`.
    pub fn env(&self, accept_new: bool) -> Vec<(&'static str, String)> {
        let mut env = vec![
            (
                "MMAN_SSH_KNOWN_HOSTS",
                self.dir.join("known_hosts").display().to_string(),
            ),
            (
                "MMAN_SSH_IDENTITY",
                self.dir.join("user_key").display().to_string(),
            ),
        ];
        if accept_new {
            env.push(("MMAN_SSH_ACCEPT_NEW", "1".to_string()));
        }
        env
    }

    /// The path of `local` as seen over SSH.
    pub fn remote(&self, local: &Path) -> PathBuf {
        PathBuf::from(format!("ssh://127.0.0.1:{}{}", self.port, local.display()))
    }

    pub fn local_of(&self, remote: &Path) -> PathBuf {
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

/// A scratch folder that removes itself, so a failed assertion leaves nothing behind.
pub struct Scratch(PathBuf);

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

pub fn scratch(label: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("mman-ssh-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("a scratch folder");
    Scratch(dir)
}
