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

//! One machine's SFTP session and every filesystem operation on it.
//!
//! A [`Link`] connects lazily, on its first use, and keeps the session for the next call. If the
//! connection dies, the next call connects again; a read-only call is also retried once at once,
//! since asking twice cannot do harm, while a call that changes something is not, because the
//! first attempt may already have happened.

use std::future::{self, Future};
use std::io;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Poll;
use std::time::{Duration, SystemTime};

use futures_core::Stream;
use openssh_sftp_client::file::TokioCompatFile;
use openssh_sftp_client::metadata::{MetaData, MetaDataBuilder, Permissions};
use openssh_sftp_client::openssh::{ControlPersist, KnownHosts, SessionBuilder};
use openssh_sftp_client::{Error, Sftp, SftpOptions, UnixTimeStamp};
use shared::{DirEntryInfo, FileKind, Metadata, ReadSeek, ScanEntry, VfsError, WriteSeek};

use crate::error::{error_chain, io_error, is_transport, lost, to_vfs};
use crate::executor::Executor;
use crate::stream::{RemoteReader, RemoteWriter};
use crate::target::{Remote, Target};

/// How long `ssh` may spend reaching a host before the attempt is given up.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A silent peer is noticed after about three of these, so a dropped network fails a call instead
/// of hanging it.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// How long the `ssh` control process lives with no one using it. While the application runs, its
/// SFTP session is in use, so this only matters when the application dies without closing the
/// session: the process then goes away by itself instead of holding a connection open for good.
const IDLE_BEFORE_CLOSE: NonZeroUsize = NonZeroUsize::new(10).expect("10 is not zero");

/// What to do about a host key `ssh` has not seen before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostKeys {
    /// Refuse a host that is not already in `known_hosts`, so the first connection is made, and
    /// its key checked, by hand with `ssh`. The default, because trusting an unknown key
    /// silently is what a man-in-the-middle needs.
    #[default]
    Strict,
    /// Remember a new host's key and trust it from then on.
    AcceptNew,
}

/// How a connection is made; the defaults are what an interactive user wants.
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub host_keys: HostKeys,
    /// A `known_hosts` file other than the user's own.
    pub known_hosts_file: Option<PathBuf>,
    /// A private key to offer, besides what `ssh_config` and the agent supply.
    pub identity: Option<PathBuf>,
    pub connect_timeout: Duration,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self {
            host_keys: HostKeys::default(),
            known_hosts_file: None,
            identity: None,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        }
    }
}

/// Whether a failed call may be repeated.
#[derive(Debug, Clone, Copy)]
enum Retry {
    /// Only reads: asking again changes nothing.
    Safe,
    Never,
}

pub(crate) struct Link {
    target: Target,
    options: ConnectOptions,
    executor: Arc<Executor>,
    // Held across a connect, so callers arriving meanwhile wait for it instead of each starting
    // their own.
    session: Mutex<Option<Arc<Sftp>>>,
}

/// Where `ssh` keeps its control socket. A Unix socket's path is limited to about 100 bytes and
/// the `openssh` crate defaults to somewhere under `$HOME`, which a long home folder can overflow,
/// so use a short, private one: the per-user runtime folder, else `/tmp`.
fn control_directory() -> PathBuf {
    short_private_directory(std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
}

fn short_private_directory(runtime: Option<PathBuf>) -> PathBuf {
    // The socket's name adds about 45 bytes (`/.ssh-connectionXXXXXX/master.XXXXXXXXXXXXXXXX`).
    const ROOM: usize = 60;
    runtime
        .filter(|dir| dir.is_absolute() && dir.as_os_str().len() <= ROOM)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

async fn connect(target: &Target, options: &ConnectOptions) -> Result<Sftp, Error> {
    let mut builder = SessionBuilder::default();
    builder
        .control_directory(control_directory())
        .control_persist(ControlPersist::IdleFor(IDLE_BEFORE_CLOSE))
        .known_hosts_check(match options.host_keys {
            HostKeys::Strict => KnownHosts::Strict,
            HostKeys::AcceptNew => KnownHosts::Add,
        })
        .connect_timeout(options.connect_timeout)
        .server_alive_interval(KEEP_ALIVE);
    if let Some(file) = &options.known_hosts_file {
        builder.user_known_hosts_file(file);
    }
    if let Some(identity) = &options.identity {
        builder.keyfile(identity);
    }
    let session = builder.connect(target.destination()).await?;
    Sftp::from_session(session, SftpOptions::new()).await
}

fn describe(meta: &MetaData) -> Metadata {
    let kind = match meta.file_type() {
        Some(t) if t.is_dir() => FileKind::Dir,
        Some(t) if t.is_symlink() => FileKind::Symlink,
        Some(t) if t.is_file() => FileKind::File,
        Some(_) => FileKind::Other,
        None => FileKind::File,
    };
    let type_bits = match kind {
        FileKind::Dir => 0o040_000,
        FileKind::Symlink => 0o120_000,
        FileKind::File => 0o100_000,
        FileKind::Other => 0,
    };
    Metadata {
        kind,
        len: meta.len().unwrap_or(0),
        // SFTP v3 reports neither blocks, link counts nor inodes; a caller leaves those out.
        allocated: None,
        mode: meta
            .permissions()
            .map(|p| type_bits | (p.as_raw().bits() & 0o7777)),
        uid: meta.uid(),
        gid: meta.gid(),
        nlink: None,
        modified: meta.modified().map(UnixTimeStamp::as_system_time),
        accessed: meta.accessed().map(UnixTimeStamp::as_system_time),
        created: None,
        device: None,
        inode: None,
    }
}

/// One folder entry and what following its link, if it is one, leads to.
struct Listed {
    name: String,
    itself: MetaData,
    followed: Option<MetaData>,
}

/// Everything in `path`, as `list_dir` and `scan_dir` need it, in as few round trips as SFTP
/// allows. `None` when `path` is not a folder.
async fn read_folder(
    sftp: Arc<Sftp>,
    path: PathBuf,
    follow: bool,
) -> Result<Option<Vec<Listed>>, Error> {
    let mut fs = sftp.fs();
    match fs.metadata(&path).await {
        Ok(meta) if meta.file_type().is_some_and(|t| t.is_dir()) => {}
        Ok(_) => return Ok(None),
        Err(Error::SftpError(openssh_sftp_client::error::SftpErrorKind::NoSuchFile, _)) => {
            return Ok(None);
        }
        Err(other) => return Err(other),
    }
    let mut stream = pin!(fs.open_dir(&path).await?.read_dir());
    let mut listed = Vec::new();
    while let Some(entry) = future::poll_fn(|cx| match stream.as_mut().poll_next(cx) {
        Poll::Ready(item) => Poll::Ready(item),
        Poll::Pending => Poll::Pending,
    })
    .await
    {
        let entry = entry?;
        let name = entry.filename().to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        let itself = entry.metadata();
        let is_link = itself.file_type().is_some_and(|t| t.is_symlink());
        listed.push(Listed {
            name,
            itself,
            followed: None,
        });
        if follow && is_link {
            // A link that leads nowhere lists as a plain, empty entry, as on the local disk.
            let last = listed.len() - 1;
            let target = path.join(&listed[last].name);
            listed[last].followed = fs.metadata(&target).await.ok();
        }
    }
    Ok(Some(listed))
}

fn display_kind(meta: &MetaData) -> bool {
    meta.file_type().is_some_and(|t| t.is_dir())
}

fn sort_for_display(entries: &mut [DirEntryInfo]) {
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
}

impl Link {
    pub(crate) fn new(target: Target, options: ConnectOptions, executor: Arc<Executor>) -> Self {
        Self {
            target,
            options,
            executor,
            session: Mutex::new(None),
        }
    }

    fn session(&self, shown: &Path) -> Result<Arc<Sftp>, VfsError> {
        let mut slot = self.session.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(live) = slot.as_ref() {
            return Ok(Arc::clone(live));
        }
        let target = self.target.clone();
        let options = self.options.clone();
        let made = self
            .executor
            .run(async move { connect(&target, &options).await })
            .map_err(|gone| lost(shown, gone))?;
        match made {
            Ok(sftp) => {
                let live = Arc::new(sftp);
                *slot = Some(Arc::clone(&live));
                Ok(live)
            }
            Err(error) => Err(io_error(
                shown,
                io::ErrorKind::NotConnected,
                format!("cannot connect to {}: {}", self.target, error_chain(&error)),
            )),
        }
    }

    /// Forgets `dead` so the next call connects afresh, unless a newer session already replaced it.
    fn forget(&self, dead: &Arc<Sftp>) {
        let mut slot = self.session.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.as_ref().is_some_and(|live| Arc::ptr_eq(live, dead)) {
            *slot = None;
        }
    }

    fn call<T, F, Fut>(&self, shown: &Path, retry: Retry, op: F) -> Result<T, VfsError>
    where
        T: Send + 'static,
        F: Fn(Arc<Sftp>) -> Fut,
        Fut: Future<Output = Result<T, Error>> + Send + 'static,
    {
        let mut attempts = match retry {
            Retry::Safe => 2,
            Retry::Never => 1,
        };
        loop {
            let session = self.session(shown)?;
            let outcome = self
                .executor
                .run(op(Arc::clone(&session)))
                .map_err(|gone| lost(shown, gone))?;
            match outcome {
                Ok(value) => return Ok(value),
                Err(error) if is_transport(&error) => {
                    self.forget(&session);
                    attempts -= 1;
                    if attempts == 0 {
                        return Err(to_vfs(shown, &error));
                    }
                }
                Err(error) => return Err(to_vfs(shown, &error)),
            }
        }
    }

    /// An SFTP server answers a plain "failure" to a create that hits something already there, so
    /// ask what is there before calling it anything else.
    fn name_clash(&self, remote: &Remote, error: VfsError) -> VfsError {
        let generic =
            matches!(&error, VfsError::Io { source, .. } if source.kind() == io::ErrorKind::Other);
        if generic && self.symlink_metadata(remote).is_ok() {
            VfsError::AlreadyExists(remote.to_path_buf())
        } else {
            error
        }
    }

    pub(crate) fn list_dir(&self, remote: &Remote) -> Result<Vec<DirEntryInfo>, VfsError> {
        let shown = remote.to_path_buf();
        let path = remote.path().to_path_buf();
        let listed = self
            .call(&shown, Retry::Safe, |sftp| {
                read_folder(sftp, path.clone(), true)
            })?
            .ok_or_else(|| VfsError::NotADirectory(shown.clone()))?;
        let mut entries: Vec<DirEntryInfo> = listed
            .into_iter()
            .map(|item| {
                // What a link leads to is what is shown, as on the local disk.
                let seen = if item.itself.file_type().is_some_and(|t| t.is_symlink()) {
                    item.followed
                } else {
                    Some(item.itself)
                };
                let described = seen.as_ref().map(describe);
                DirEntryInfo {
                    path: remote.at(remote.path().join(&item.name)).to_path_buf(),
                    name: item.name,
                    is_dir: seen.as_ref().is_some_and(display_kind),
                    size: described.as_ref().map_or(0, |m| m.len),
                    modified: described.as_ref().and_then(|m| m.modified),
                    mode: described.and_then(|m| m.mode),
                }
            })
            .collect();
        sort_for_display(&mut entries);
        Ok(entries)
    }

    pub(crate) fn scan_dir(&self, remote: &Remote) -> Result<Vec<ScanEntry>, VfsError> {
        let shown = remote.to_path_buf();
        let path = remote.path().to_path_buf();
        let listed = self
            .call(&shown, Retry::Safe, |sftp| {
                read_folder(sftp, path.clone(), false)
            })?
            .ok_or_else(|| VfsError::NotADirectory(shown.clone()))?;
        Ok(listed
            .into_iter()
            .map(|item| ScanEntry {
                path: remote.at(remote.path().join(&item.name)).to_path_buf(),
                name: item.name,
                metadata: Some(describe(&item.itself)),
            })
            .collect())
    }

    pub(crate) fn metadata(&self, remote: &Remote) -> Result<Metadata, VfsError> {
        let path = remote.path().to_path_buf();
        self.call(&remote.to_path_buf(), Retry::Safe, |sftp| {
            let path = path.clone();
            async move { sftp.fs().metadata(&path).await }
        })
        .map(|meta| describe(&meta))
    }

    pub(crate) fn symlink_metadata(&self, remote: &Remote) -> Result<Metadata, VfsError> {
        let path = remote.path().to_path_buf();
        self.call(&remote.to_path_buf(), Retry::Safe, |sftp| {
            let path = path.clone();
            async move { sftp.fs().symlink_metadata(&path).await }
        })
        .map(|meta| describe(&meta))
    }

    pub(crate) fn read_link(&self, remote: &Remote) -> Result<PathBuf, VfsError> {
        let path = remote.path().to_path_buf();
        self.call(&remote.to_path_buf(), Retry::Safe, |sftp| {
            let path = path.clone();
            async move { sftp.fs().read_link(&path).await }
        })
    }

    pub(crate) fn open_read(&self, remote: &Remote) -> Result<Box<dyn ReadSeek>, VfsError> {
        let shown = remote.to_path_buf();
        let path = remote.path().to_path_buf();
        let (file, len) = self
            .call(&shown, Retry::Safe, |sftp| {
                let path = path.clone();
                async move {
                    let meta = sftp.fs().metadata(&path).await?;
                    if meta.file_type().is_some_and(|t| t.is_dir()) {
                        return Ok(None);
                    }
                    let file = sftp.options().read(true).open(&path).await?;
                    Ok(Some((
                        Box::pin(TokioCompatFile::new(file)),
                        meta.len().unwrap_or(0),
                    )))
                }
            })?
            .ok_or_else(|| {
                io_error(&shown, io::ErrorKind::InvalidInput, "is a directory".into())
            })?;
        Ok(RemoteReader::new(Arc::clone(&self.executor), file, len).boxed())
    }

    pub(crate) fn create_write(&self, remote: &Remote) -> Result<Box<dyn WriteSeek>, VfsError> {
        let path = remote.path().to_path_buf();
        let file = self
            .call(&remote.to_path_buf(), Retry::Never, |sftp| {
                let path = path.clone();
                async move {
                    let file = sftp
                        .options()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .await?;
                    Ok(Box::pin(TokioCompatFile::new(file)))
                }
            })
            .map_err(|error| self.name_clash(remote, error))?;
        Ok(RemoteWriter::new(Arc::clone(&self.executor), file).boxed())
    }

    pub(crate) fn create_symlink(&self, target: &Path, link: &Remote) -> Result<(), VfsError> {
        let (target, path) = (target.to_path_buf(), link.path().to_path_buf());
        self.call(&link.to_path_buf(), Retry::Never, |sftp| {
            let (target, path) = (target.clone(), path.clone());
            async move { sftp.fs().symlink(&target, &path).await }
        })
        .map_err(|error| self.name_clash(link, error))
    }

    pub(crate) fn set_mode(&self, remote: &Remote, mode: u32) -> Result<(), VfsError> {
        let path = remote.path().to_path_buf();
        // `From<u16>` keeps the twelve permission bits, which is all `set_mode` promises.
        let permissions = Permissions::from((mode & 0o7777) as u16);
        self.call(&remote.to_path_buf(), Retry::Never, |sftp| {
            let path = path.clone();
            async move { sftp.fs().set_permissions(&path, permissions).await }
        })
    }

    pub(crate) fn replace(&self, src: &Remote, dst: &Remote) -> Result<(), VfsError> {
        if self
            .symlink_metadata(dst)
            .is_ok_and(|m| m.kind == FileKind::Dir)
        {
            return Err(io_error(
                &dst.to_path_buf(),
                io::ErrorKind::IsADirectory,
                "is a directory".into(),
            ));
        }
        let (from, to) = (src.path().to_path_buf(), dst.path().to_path_buf());
        self.call(&dst.to_path_buf(), Retry::Never, |sftp| {
            let (from, to) = (from.clone(), to.clone());
            async move {
                let mut fs = sftp.fs();
                // With the `posix-rename` extension this replaces `to` in one step. Without it a
                // rename refuses an occupied name, so the old file is removed first and may be
                // briefly missing, which `Vfs::replace` allows.
                match fs.rename(&from, &to).await {
                    Err(Error::SftpError(..)) => {
                        let _ = fs.remove_file(&to).await;
                        fs.rename(&from, &to).await
                    }
                    other => other,
                }
            }
        })
    }

    pub(crate) fn create_dir(&self, remote: &Remote) -> Result<(), VfsError> {
        let path = remote.path().to_path_buf();
        self.call(&remote.to_path_buf(), Retry::Never, |sftp| {
            let path = path.clone();
            async move { sftp.fs().create_dir(&path).await }
        })
        .map_err(|error| self.name_clash(remote, error))
    }

    pub(crate) fn create_file(&self, remote: &Remote) -> Result<(), VfsError> {
        // Dropping the writer closes the new, empty file.
        self.create_write(remote).map(drop)
    }

    pub(crate) fn touch(&self, remote: &Remote) -> Result<(), VfsError> {
        if self.symlink_metadata(remote).is_err() {
            return match self.create_file(remote) {
                Err(VfsError::AlreadyExists(_)) => self.touch_existing(remote),
                other => other,
            };
        }
        self.touch_existing(remote)
    }

    fn touch_existing(&self, remote: &Remote) -> Result<(), VfsError> {
        let path = remote.path().to_path_buf();
        let now = SystemTime::now();
        self.call(&remote.to_path_buf(), Retry::Never, |sftp| {
            let path = path.clone();
            async move {
                let stamp = UnixTimeStamp::new(now)
                    .map_err(|error| Error::IOError(io::Error::other(error)))?;
                let meta = MetaDataBuilder::new().time(stamp, stamp).create();
                sftp.fs().set_metadata(&path, meta).await
            }
        })
    }

    pub(crate) fn copy_file(&self, src: &Remote, dst: &Remote) -> Result<(), VfsError> {
        if self.symlink_metadata(dst).is_ok() {
            return Err(VfsError::AlreadyExists(dst.to_path_buf()));
        }
        let (from, to) = (src.path().to_path_buf(), dst.path().to_path_buf());
        self.call(&src.to_path_buf(), Retry::Never, |sftp| {
            let (from, to) = (from.clone(), to.clone());
            async move {
                let mut fs = sftp.fs();
                let source_meta = fs.metadata(&from).await?;
                let mut reader = Box::pin(TokioCompatFile::new(
                    sftp.options().read(true).open(&from).await?,
                ));
                let mut writer = Box::pin(TokioCompatFile::new(
                    sftp.options()
                        .write(true)
                        .create_new(true)
                        .open(&to)
                        .await?,
                ));
                let copied = async {
                    tokio::io::copy(&mut reader, &mut writer).await?;
                    tokio::io::AsyncWriteExt::shutdown(&mut writer).await
                }
                .await;
                if let Err(error) = copied {
                    // A half-written copy would otherwise look like the real file.
                    let _ = fs.remove_file(&to).await;
                    return Err(Error::from(error));
                }
                if let Some(permissions) = source_meta.permissions() {
                    fs.set_permissions(&to, permissions).await?;
                }
                Ok(())
            }
        })
        .map_err(|error| self.name_clash(dst, error))
    }

    pub(crate) fn rename(&self, src: &Remote, dst: &Remote) -> Result<(), VfsError> {
        // The `posix-rename` extension would replace `dst`, so say so before asking.
        if self.symlink_metadata(dst).is_ok() {
            return Err(VfsError::AlreadyExists(dst.to_path_buf()));
        }
        let (from, to) = (src.path().to_path_buf(), dst.path().to_path_buf());
        self.call(&src.to_path_buf(), Retry::Never, |sftp| {
            let (from, to) = (from.clone(), to.clone());
            async move { sftp.fs().rename(&from, &to).await }
        })
    }

    pub(crate) fn remove_file(&self, remote: &Remote) -> Result<(), VfsError> {
        let path = remote.path().to_path_buf();
        self.call(&remote.to_path_buf(), Retry::Never, |sftp| {
            let path = path.clone();
            async move { sftp.fs().remove_file(&path).await }
        })
    }

    pub(crate) fn remove_dir_all(&self, remote: &Remote) -> Result<(), VfsError> {
        let shown = remote.to_path_buf();
        // Like `std::fs::remove_dir_all`: a folder goes with its contents, a link goes without
        // what it points at, and anything else is not a folder.
        match self.symlink_metadata(remote)?.kind {
            FileKind::Dir => {}
            FileKind::Symlink => return self.remove_file(remote),
            FileKind::File | FileKind::Other => return Err(VfsError::NotADirectory(shown)),
        }
        let path = remote.path().to_path_buf();
        self.call(&shown, Retry::Never, |sftp| {
            let path = path.clone();
            async move { remove_tree(sftp, path).await }
        })
    }

    /// The login's home folder on this machine, which is where an `ssh://host` with no path opens.
    pub(crate) fn home(&self) -> Result<PathBuf, VfsError> {
        let shown = PathBuf::from(format!("ssh://{}", self.target));
        self.call(&shown, Retry::Safe, |sftp| async move {
            sftp.fs().canonicalize(".").await
        })
    }
}

/// Deletes the folder `root` and everything under it without following a link out of it.
async fn remove_tree(sftp: Arc<Sftp>, root: PathBuf) -> Result<(), Error> {
    let mut fs = sftp.fs();
    // Parents are found before their children, so removing the folders in reverse empties each
    // one before it is removed itself.
    let mut folders = vec![root];
    let mut next = 0;
    while next < folders.len() {
        let folder = folders[next].clone();
        next += 1;
        let Some(entries) = read_folder(Arc::clone(&sftp), folder.clone(), false).await? else {
            continue;
        };
        for item in entries {
            let child = folder.join(&item.name);
            if item.itself.file_type().is_some_and(|t| t.is_dir()) {
                folders.push(child);
            } else {
                fs.remove_file(&child).await?;
            }
        }
    }
    for folder in folders.into_iter().rev() {
        fs.remove_dir(&folder).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_runtime_folder_is_used_and_a_long_or_missing_one_is_not() {
        let run = PathBuf::from("/run/user/1000");
        assert_eq!(short_private_directory(Some(run.clone())), run);
        assert_eq!(short_private_directory(None), PathBuf::from("/tmp"));
        let long = PathBuf::from(format!("/{}", "x".repeat(80)));
        assert_eq!(short_private_directory(Some(long)), PathBuf::from("/tmp"));
        assert_eq!(
            short_private_directory(Some(PathBuf::from("relative"))),
            PathBuf::from("/tmp")
        );
    }
}
