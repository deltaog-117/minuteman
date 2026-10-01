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
//! Where a remote path lives: the host to connect to and the path on it, parsed from the text the
//! file manager already passes around as a `Path` (`ssh://[user@]host[:port]/path`).
//!
//! Every piece is checked when the value is built, so nothing that reaches the `ssh` program can
//! be mistaken for one of its options: a host or user may not start with `-`, and the connection
//! is always asked for as an `ssh://` URI.

use std::fmt;
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};

/// The schemes a path may start with. `sftp` is accepted because people type it, and written back
/// as `ssh`, so one location has one spelling.
const SCHEMES: [&str; 2] = ["ssh", "sftp"];
const CANONICAL_SCHEME: &str = "ssh";
const MAX_HOST_LEN: usize = 253;
const MAX_USER_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TargetError {
    #[error("the host name is empty")]
    EmptyHost,
    #[error(
        "the host name `{0}` is not valid (letters, digits, `.`, `-` and `_`, not starting with `-`)"
    )]
    BadHost(String),
    #[error(
        "the user name `{0}` is not valid (letters, digits, `.`, `-` and `_`, not starting with `-`)"
    )]
    BadUser(String),
    #[error("the port `{0}` is not a number from 1 to 65535")]
    BadPort(String),
    #[error("the path holds a NUL byte")]
    NulInPath,
}

/// A host name or address, as `ssh` would be given it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Host(String);

impl Host {
    /// Accepts a DNS name, an IPv4 address, an `~/.ssh/config` alias or an IPv6 address (without
    /// the brackets), and refuses anything that could be read as an option.
    pub fn try_new(text: &str) -> Result<Self, TargetError> {
        if text.is_empty() {
            return Err(TargetError::EmptyHost);
        }
        let is_name = text.len() <= MAX_HOST_LEN
            && !text.starts_with('-')
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
        let is_ipv6 = text.parse::<std::net::Ipv6Addr>().is_ok();
        if is_name || is_ipv6 {
            Ok(Self(text.to_owned()))
        } else {
            Err(TargetError::BadHost(text.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_ipv6(&self) -> bool {
        self.0.contains(':')
    }
}

/// A login name on the remote host.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct User(String);

impl User {
    pub fn try_new(text: &str) -> Result<Self, TargetError> {
        let valid = !text.is_empty()
            && text.len() <= MAX_USER_LEN
            && !text.starts_with('-')
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(TargetError::BadUser(text.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One machine to connect to. Two paths with the same target share one connection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Target {
    user: Option<User>,
    host: Host,
    port: Option<NonZeroU16>,
}

impl Target {
    pub fn new(user: Option<User>, host: Host, port: Option<NonZeroU16>) -> Self {
        Self { user, host, port }
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    /// The connection request in the one form that can never be taken for an option: an `ssh://`
    /// URI, which `ssh` and the `openssh` crate both understand.
    pub fn destination(&self) -> String {
        format!("{CANONICAL_SCHEME}://{self}")
    }
}

impl fmt::Display for Target {
    /// `[user@]host[:port]`, with an IPv6 address in brackets.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(user) = &self.user {
            write!(f, "{}@", user.as_str())?;
        }
        if self.host.is_ipv6() {
            write!(f, "[{}]", self.host.as_str())?;
        } else {
            f.write_str(self.host.as_str())?;
        }
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        Ok(())
    }
}

/// A place on a remote machine: a [`Target`] and an absolute path on it (`/` is the machine's
/// root, not the login's home).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Remote {
    target: Target,
    path: PathBuf,
}

/// What a path the file manager holds turns out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// An ordinary path on this machine's disk.
    Local,
    /// A path on a remote machine.
    Remote(Remote),
    /// A bare scheme with nothing after it, which is what stepping up out of a remote machine's
    /// root lands on. It names nothing.
    Beyond,
}

impl Remote {
    pub fn new(target: Target, path: PathBuf) -> Self {
        Self { target, path }
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    /// The path on the remote machine, absolute.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The same remote machine at another path on it.
    pub fn at(&self, path: PathBuf) -> Self {
        Self {
            target: self.target.clone(),
            path,
        }
    }

    /// The text form the rest of the file manager carries, `ssh://host/path`.
    pub fn to_path_buf(&self) -> PathBuf {
        let mut text = format!("{CANONICAL_SCHEME}://{}", self.target);
        text.push_str(&self.path.to_string_lossy());
        PathBuf::from(text)
    }

    /// Reads `path` as a location. Text that does not begin with a remote scheme is
    /// [`Location::Local`], so ordinary paths never pass through here as errors.
    ///
    /// # Errors
    ///
    /// A path that begins with a remote scheme but names no valid host, user or port.
    pub fn locate(path: &Path) -> Result<Location, TargetError> {
        let Some(text) = path.to_str() else {
            return Ok(Location::Local);
        };
        let Some((scheme, rest)) = text.split_once(':') else {
            return Ok(Location::Local);
        };
        if !SCHEMES.contains(&scheme) {
            return Ok(Location::Local);
        }
        let Some(rest) = rest.strip_prefix("//") else {
            // `ssh:` alone is what `Path::parent` leaves behind at a remote root.
            return Ok(if rest.is_empty() {
                Location::Beyond
            } else {
                Location::Local
            });
        };
        let rest = rest.trim_start_matches('/');
        if rest.is_empty() {
            return Ok(Location::Beyond);
        }
        let (authority, remote_path) = match rest.find('/') {
            Some(at) => rest.split_at(at),
            None => (rest, "/"),
        };
        if remote_path.contains('\0') {
            return Err(TargetError::NulInPath);
        }
        Ok(Location::Remote(Self {
            target: parse_authority(authority)?,
            path: PathBuf::from(remote_path),
        }))
    }
}

/// Whether `path` is on another machine (or is the empty "beyond" a remote root steps up to), as
/// opposed to this machine's disk. For code that runs a program or reads the disk itself and so
/// has to refuse such a path.
pub fn is_remote(path: &Path) -> bool {
    !matches!(Remote::locate(path), Ok(Location::Local))
}

fn parse_authority(authority: &str) -> Result<Target, TargetError> {
    let (user, host_port) = match authority.split_once('@') {
        Some((user, rest)) => (Some(User::try_new(user)?), rest),
        None => (None, authority),
    };
    let (host_text, port_text) = if let Some(bracketed) = host_port.strip_prefix('[') {
        let (host, after) = bracketed
            .split_once(']')
            .ok_or_else(|| TargetError::BadHost(host_port.to_owned()))?;
        match after.strip_prefix(':') {
            Some(port) => (host, Some(port)),
            None if after.is_empty() => (host, None),
            None => return Err(TargetError::BadHost(host_port.to_owned())),
        }
    } else {
        match host_port.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_port, None),
        }
    };
    let port = match port_text {
        Some(text) => Some(
            text.parse::<NonZeroU16>()
                .map_err(|_| TargetError::BadPort(text.to_owned()))?,
        ),
        None => None,
    };
    Ok(Target::new(user, Host::try_new(host_text)?, port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn remote(text: &str) -> Remote {
        match Remote::locate(Path::new(text)) {
            Ok(Location::Remote(remote)) => remote,
            other => panic!("{text} should be remote, got {other:?}"),
        }
    }

    #[test]
    fn a_plain_path_is_local() {
        for text in [
            "/home/me",
            "relative/dir",
            "",
            "C:/x",
            "ssh",
            "sshfs://x",
            "ssh:thing",
        ] {
            assert_eq!(
                Remote::locate(Path::new(text)),
                Ok(Location::Local),
                "{text}"
            );
        }
    }

    #[test]
    fn only_a_local_path_is_not_remote() {
        assert!(!is_remote(Path::new("/home/me")));
        assert!(is_remote(Path::new("ssh://box/x")));
        assert!(is_remote(Path::new("ssh:")));
        assert!(
            is_remote(Path::new("ssh://-bad/x")),
            "an invalid address is not local either"
        );
    }

    #[test]
    fn a_remote_path_is_split_into_target_and_path() {
        let r = remote("ssh://me@box.example:2222/var/log");
        assert_eq!(r.target().to_string(), "me@box.example:2222");
        assert_eq!(r.path(), Path::new("/var/log"));
        assert_eq!(r.target().destination(), "ssh://me@box.example:2222");
    }

    #[test]
    fn no_path_means_the_machine_root() {
        assert_eq!(remote("ssh://box").path(), Path::new("/"));
        assert_eq!(remote("ssh://box/").path(), Path::new("/"));
    }

    #[test]
    fn sftp_is_written_back_as_ssh() {
        assert_eq!(
            remote("sftp://box/etc").to_path_buf(),
            PathBuf::from("ssh://box/etc")
        );
    }

    #[test]
    fn an_ipv6_address_goes_in_brackets() {
        let r = remote("ssh://[::1]:22/tmp");
        assert_eq!(r.target().host().as_str(), "::1");
        assert_eq!(r.to_path_buf(), PathBuf::from("ssh://[::1]:22/tmp"));
    }

    #[test]
    fn a_bare_scheme_names_nothing() {
        for text in ["ssh:", "ssh://", "sftp:///"] {
            assert_eq!(Remote::locate(Path::new(text)), Ok(Location::Beyond));
        }
    }

    #[test]
    fn stepping_up_from_a_remote_root_lands_beyond() {
        let root = remote("ssh://box/").to_path_buf();
        let up = root.parent().expect("a root still has a parent path");
        assert_eq!(Remote::locate(up), Ok(Location::Beyond));
    }

    #[test]
    fn what_could_be_an_option_is_refused() {
        for text in [
            "ssh://-oProxyCommand=x/p",
            "ssh://-v@box/p",
            "ssh://me@-box/p",
            "ssh://bo x/p",
            "ssh://box:0/p",
            "ssh://box:70000/p",
            "ssh://box:http/p",
            "ssh://[::1/p",
            "ssh://me@/p",
            "ssh://@box/p",
        ] {
            assert!(Remote::locate(Path::new(text)).is_err(), "{text}");
        }
    }

    fn host_text() -> impl Strategy<Value = String> {
        "[a-z0-9][a-z0-9._-]{0,20}"
    }

    proptest! {
        #[test]
        fn a_valid_location_survives_a_round_trip(
            user in proptest::option::of("[a-z0-9][a-z0-9._-]{0,10}"),
            host in host_text(),
            port in proptest::option::of(1u16..),
            path in "/[a-zA-Z0-9 ._/-]{0,30}",
        ) {
            let target = Target::new(
                user.as_deref().map(|u| User::try_new(u).unwrap()),
                Host::try_new(&host).unwrap(),
                port.and_then(NonZeroU16::new),
            );
            let original = Remote::new(target, PathBuf::from(&path));
            let again = Remote::locate(&original.to_path_buf());
            prop_assert_eq!(again, Ok(Location::Remote(original)));
        }

        #[test]
        fn nothing_accepted_can_begin_with_a_dash(text in "\\PC{0,40}") {
            if let Ok(Location::Remote(r)) = Remote::locate(Path::new(&format!("ssh://{text}"))) {
                let destination = r.target().destination();
                prop_assert!(destination.starts_with("ssh://"));
                let after_scheme = &destination["ssh://".len()..];
                prop_assert!(!after_scheme.starts_with('-'));
                if let Some((user, _)) = after_scheme.split_once('@') {
                    prop_assert!(!user.starts_with('-'));
                }
            }
        }

        #[test]
        fn locating_never_panics(text in "\\PC{0,60}") {
            let _ = Remote::locate(Path::new(&text));
        }
    }
}
