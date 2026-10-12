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

//! Turning what SFTP and `ssh` report into [`VfsError`]s the rest of the file manager understands.

use std::io;
use std::path::Path;

use openssh_sftp_client::Error;
use openssh_sftp_client::error::SftpErrorKind;
use shared::VfsError;

use crate::executor::Lost;

/// Whether `error` means the connection itself failed, as opposed to the server refusing one
/// request. After a transport failure the session is of no further use.
pub(crate) fn is_transport(error: &Error) -> bool {
    !matches!(error, Error::SftpError(..) | Error::UnsupportedExtension(_))
}

/// `shown` is the path as the user sees it (`ssh://host/...`), so a message names what they typed.
pub(crate) fn to_vfs(shown: &Path, error: &Error) -> VfsError {
    match error {
        Error::SftpError(SftpErrorKind::NoSuchFile, _) => VfsError::NotFound(shown.to_path_buf()),
        Error::SftpError(SftpErrorKind::PermDenied, _) => io_error(
            shown,
            io::ErrorKind::PermissionDenied,
            "permission denied".into(),
        ),
        Error::SftpError(SftpErrorKind::OpUnsupported, _) | Error::UnsupportedExtension(_) => {
            VfsError::Unsupported("an operation this SFTP server does not offer")
        }
        Error::SftpError(_, message) => {
            io_error(shown, io::ErrorKind::Other, message.get().0.into())
        }
        Error::IOError(source) => VfsError::Io {
            path: shown.to_path_buf(),
            source: io::Error::new(source.kind(), source.to_string()),
        },
        other => io_error(shown, io::ErrorKind::BrokenPipe, other.to_string()),
    }
}

/// An error and its causes on one line. `ssh`'s own complaint (a refused key, a bad socket path)
/// is usually the last cause, and the top-level text alone hides it. A cause already quoted by the
/// text before it is not repeated.
pub(crate) fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut next = error.source();
    while let Some(cause) = next {
        let said = cause.to_string();
        if !text.contains(&said) {
            text.push_str(": ");
            text.push_str(&said);
        }
        next = cause.source();
    }
    text
}

pub(crate) fn lost(shown: &Path, _: Lost) -> VfsError {
    io_error(
        shown,
        io::ErrorKind::BrokenPipe,
        "the SSH connection was shut down".into(),
    )
}

pub(crate) fn io_error(shown: &Path, kind: io::ErrorKind, message: String) -> VfsError {
    VfsError::Io {
        path: shown.to_path_buf(),
        source: io::Error::new(kind, message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dead_connection_is_a_transport_failure_and_a_refusal_is_not() {
        let broken = Error::IOError(io::Error::from(io::ErrorKind::BrokenPipe));
        assert!(is_transport(&broken));
        assert!(!is_transport(&Error::UnsupportedExtension(&"x")));
    }

    #[derive(Debug, thiserror::Error)]
    #[error("failed to connect to the remote host")]
    struct Outer(#[source] io::Error);

    #[test]
    fn a_chain_names_the_last_cause_without_repeating_itself() {
        let error = Outer(io::Error::other("Host key verification failed."));
        assert_eq!(
            error_chain(&error),
            "failed to connect to the remote host: Host key verification failed."
        );
        let same = Outer(io::Error::other("failed to connect to the remote host"));
        assert_eq!(error_chain(&same), "failed to connect to the remote host");
    }

    #[test]
    fn a_lost_executor_reads_as_a_broken_pipe() {
        match lost(Path::new("ssh://h/p"), Lost) {
            VfsError::Io { source, .. } => assert_eq!(source.kind(), io::ErrorKind::BrokenPipe),
            other => panic!("expected an io error, got {other:?}"),
        }
    }
}
