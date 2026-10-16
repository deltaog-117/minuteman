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

//! Starting the `ssh` process that holds a machine's connection (the "master"), which every SFTP
//! channel then rides on.
//!
//! The `openssh` crate can start it, but only with `BatchMode=yes`, which forbids every prompt, so
//! a password, a key's passphrase or an unknown host key can never be answered. This starts it
//! instead, without batch mode and with the plugin's own executable as `SSH_ASKPASS` (see
//! `askpass`), and hands the finished connection to the crate to use (`Session::resume`).
//!
//! Each attempt gets a folder of its own under the user's runtime folder, which holds the control
//! socket, `ssh`'s log and, while the attempt lasts, the socket its helper asks through. A failed
//! attempt removes it; one that worked keeps it for the life of the connection, and a later
//! attempt sweeps what old ones left.

use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use openssh_sftp_client::openssh::Session;
use openssh_sftp_client::{Sftp, SftpOptions};
use shared::ssh_path::Target;
use tokio::process::Command;

use crate::askpass::{ENV_SOCKET, Relay};
use crate::link::{ConnectOptions, HostKeys};

/// How long the master lives with no one using it. While the application runs, its SFTP session is
/// in use, so this only matters when the application dies without closing the session: the process
/// then goes away by itself instead of holding a connection open for good.
const IDLE_BEFORE_CLOSE: Duration = Duration::from_secs(10);
/// A silent peer is noticed after about three of these, so a dropped network fails a call instead
/// of hanging it.
const KEEP_ALIVE: Duration = Duration::from_secs(15);
/// A folder of an attempt that has no connection and has not changed for this long is left over.
/// Longer than a question waits for its answer, so an attempt held up by one is never swept.
const STALE_AFTER: Duration = Duration::from_secs(10 * 60);
const FOLDER_PREFIX: &str = ".mman-ssh-";

/// Why a connection could not be made.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConnectError {
    /// `ssh` said no, in its own words.
    #[error("{0}")]
    Refused(String),
    #[error("could not prepare the connection: {0}")]
    Setup(#[source] io::Error),
    #[error(transparent)]
    Sftp(#[from] openssh_sftp_client::Error),
}

/// An attempt's folder, removed with the guard unless the attempt worked.
struct Folder {
    path: PathBuf,
    keep: bool,
}

impl Drop for Folder {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Where the attempts' folders go. A Unix socket's path is limited to about 100 bytes and the
/// `openssh` crate defaults to somewhere under `$HOME`, which a long home folder can overflow, so
/// use a short, private one: the per-user runtime folder, else `/tmp`.
fn control_directory() -> PathBuf {
    short_private_directory(std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
}

fn short_private_directory(runtime: Option<PathBuf>) -> PathBuf {
    // The socket's name adds about 50 bytes (`/.mman-ssh-NNNNNNN-NN/master.XXXXXXXXXXXXXXXX`).
    const ROOM: usize = 50;
    runtime
        .filter(|dir| dir.is_absolute() && dir.as_os_str().len() <= ROOM)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// A new folder only the user can open.
fn make_folder(root: &Path) -> io::Result<Folder> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    loop {
        let name = format!(
            "{FOLDER_PREFIX}{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = root.join(name);
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => return Ok(Folder { path, keep: false }),
            // Left by a process that had this number before.
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
}

/// Removes the folders of attempts that ended long ago. A folder with a control socket still
/// belongs to a live connection, and a young one may be an attempt in progress in another process.
fn sweep(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ours = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(FOLDER_PREFIX));
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > STALE_AFTER);
        if ours && old && !path.join("master").exists() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

/// What `ssh` says when it fails: the last line it logged, without its program name.
fn failure_text(log: &Path, status: std::process::ExitStatus) -> String {
    fs::read_to_string(log)
        .ok()
        .and_then(|text| {
            text.lines()
                .rev()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(|line| line.strip_prefix("ssh: ").unwrap_or(line).to_owned())
        })
        .unwrap_or_else(|| format!("ssh failed ({status})"))
}

/// The `-o StrictHostKeyChecking=` value. A host that may ask is asked; one that cannot refuses
/// every key it does not already know.
fn host_key_policy(policy: HostKeys, can_ask: bool) -> &'static str {
    match policy {
        HostKeys::AcceptNew => "accept-new",
        HostKeys::Ask if can_ask => "ask",
        HostKeys::Ask | HostKeys::Strict => "yes",
    }
}

pub(crate) async fn connect(
    target: &Target,
    options: &ConnectOptions,
) -> Result<Sftp, ConnectError> {
    let root = control_directory();
    sweep(&root);
    let mut folder = make_folder(&root).map_err(ConnectError::Setup)?;
    let control = folder.path.join("master");
    let log = folder.path.join("log");
    let asker = options.asker.get();
    let relay = match &asker {
        Some(asker) => {
            Some(Relay::start(&folder.path, asker.clone()).map_err(ConnectError::Setup)?)
        }
        None => None,
    };

    let mut ssh = Command::new("ssh");
    ssh.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // Abandoning the attempt must not leave a process waiting for a password nobody will give.
        .kill_on_drop(true)
        .arg("-E")
        .arg(&log)
        .arg("-S")
        .arg(&control)
        .args(["-M", "-f", "-N"])
        .arg("-o")
        .arg(format!("ControlPersist={}s", IDLE_BEFORE_CLOSE.as_secs()))
        .arg("-o")
        .arg(format!(
            "StrictHostKeyChecking={}",
            host_key_policy(options.host_keys, relay.is_some())
        ))
        .arg("-o")
        .arg(format!(
            "ConnectTimeout={}",
            options.connect_timeout.as_secs().max(1)
        ))
        .arg("-o")
        .arg(format!("ServerAliveInterval={}", KEEP_ALIVE.as_secs()));
    if let Some(relay) = &relay {
        let program = match &options.askpass_program {
            Some(program) => program.clone(),
            None => std::env::current_exe().map_err(ConnectError::Setup)?,
        };
        ssh.env("SSH_ASKPASS", program)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env(ENV_SOCKET, relay.path());
    } else {
        ssh.args(["-o", "BatchMode=yes"]);
    }
    if let Some(file) = &options.known_hosts_file {
        let mut option = std::ffi::OsString::from("UserKnownHostsFile=");
        option.push(file);
        ssh.arg("-o").arg(option);
    }
    if let Some(identity) = &options.identity {
        ssh.args(["-o", "IdentitiesOnly=yes"])
            .arg("-i")
            .arg(identity);
    }
    ssh.arg(target.destination());

    let status = ssh.status().await.map_err(ConnectError::Setup)?;
    if !status.success() {
        return Err(ConnectError::Refused(failure_text(&log, status)));
    }
    drop(relay);
    let session = Session::resume(control.into_boxed_path(), Some(log.into_boxed_path()));
    let sftp = Sftp::from_session(session, SftpOptions::new()).await?;
    folder.keep = true;
    Ok(sftp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_runtime_folder_is_used_and_a_long_or_missing_one_falls_back_to_tmp() {
        let run = PathBuf::from("/run/user/1000");
        assert_eq!(short_private_directory(Some(run.clone())), run);
        assert_eq!(short_private_directory(None), PathBuf::from("/tmp"));
        let long = PathBuf::from(format!("/run/{}", "x".repeat(80)));
        assert_eq!(short_private_directory(Some(long)), PathBuf::from("/tmp"));
        assert_eq!(
            short_private_directory(Some(PathBuf::from("relative"))),
            PathBuf::from("/tmp")
        );
    }

    #[test]
    fn an_unknown_key_is_asked_about_only_where_there_is_someone_to_ask() {
        assert_eq!(host_key_policy(HostKeys::Ask, true), "ask");
        assert_eq!(host_key_policy(HostKeys::Ask, false), "yes");
        assert_eq!(host_key_policy(HostKeys::Strict, true), "yes");
        assert_eq!(host_key_policy(HostKeys::AcceptNew, false), "accept-new");
    }

    #[test]
    fn a_folder_is_private_and_goes_unless_kept() {
        let root = std::env::temp_dir().join(format!("mman-master-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = {
            let folder = make_folder(&root).unwrap();
            let mode = fs::metadata(&folder.path).unwrap().permissions();
            assert_eq!(
                std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
                0o700
            );
            folder.path.clone()
        };
        assert!(!path.exists());
        let kept = {
            let mut folder = make_folder(&root).unwrap();
            folder.keep = true;
            folder.path.clone()
        };
        assert!(kept.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_young_folder_survives_the_sweep() {
        let root = std::env::temp_dir().join(format!("mman-sweep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(format!("{FOLDER_PREFIX}1-1"))).unwrap();
        sweep(&root);
        assert!(root.join(format!("{FOLDER_PREFIX}1-1")).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_last_line_ssh_logged_is_what_the_user_is_told() {
        let log = std::env::temp_dir().join(format!("mman-log-{}", std::process::id()));
        fs::write(
            &log,
            "debug\nssh: Could not resolve hostname nope: Name or service not known\n\n",
        )
        .unwrap();
        let status = std::process::Command::new("false").status().unwrap();
        assert_eq!(
            failure_text(&log, status),
            "Could not resolve hostname nope: Name or service not known"
        );
        fs::remove_file(&log).unwrap();
        assert!(failure_text(&log, status).starts_with("ssh failed"));
    }
}
