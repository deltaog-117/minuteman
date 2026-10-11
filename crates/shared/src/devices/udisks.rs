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

//! Mounts, unmounts and powers off devices by running `udisksctl`, the command line to the
//! system's disk service. That service does the privileged part for an ordinary user, so no root
//! and no `mount` command are involved; and without the program installed, listing devices still
//! works and only changing them does not.
//!
//! Each call waits for the program, so it must run off the interface thread. A call is given
//! `TIMEOUT` and killed after it, because a request for a password with nobody to answer it would
//! otherwise wait for ever.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(25);
const POLL_EVERY: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Mount,
    Unmount,
    /// Cuts the power to the whole disk, once nothing on it is mounted: what makes it safe to
    /// pull out.
    PowerOff,
}

impl Action {
    fn subcommand(self) -> &'static str {
        match self {
            Action::Mount => "mount",
            Action::Unmount => "unmount",
            Action::PowerOff => "power-off",
        }
    }
}

/// What a successful call did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    /// Where a mount landed, for `Action::Mount`.
    pub mounted_at: Option<PathBuf>,
}

/// Runs `action` on the block device called `node` (`sdb1`, or `sdb` for `PowerOff`). The error
/// is one line a person can read.
pub fn run(action: Action, node: &str) -> Result<Done, String> {
    let device = format!("/dev/{node}");
    let mut child = Command::new("udisksctl")
        .args([action.subcommand(), "-b", &device, "--no-user-interaction"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                "udisksctl is not installed, so devices can be listed but not changed".to_string()
            }
            _ => format!("cannot run udisksctl: {e}"),
        })?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("udisksctl {} timed out", action.subcommand()));
            }
            Ok(None) => std::thread::sleep(POLL_EVERY),
            Err(e) => return Err(format!("cannot wait for udisksctl: {e}")),
        }
    };
    let (mut out, mut err) = (String::new(), String::new());
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut out);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut err);
    }
    if status.success() {
        return Ok(Done {
            mounted_at: parse_mounted(&out),
        });
    }
    Err(reason(&err, &out))
}

/// The folder in `Mounted /dev/sdb1 at /run/media/me/STICK.`
pub fn parse_mounted(output: &str) -> Option<PathBuf> {
    let line = output.lines().find(|l| l.starts_with("Mounted "))?;
    let (_, path) = line.rsplit_once(" at ")?;
    let path = path.trim().trim_end_matches('.').trim_matches(['\'', '"']);
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// The last line of what the program said, with its machinery stripped off: `Error unmounting
/// /dev/sdb1: GDBus.Error:org.freedesktop.UDisks2.Error.DeviceBusy: target is busy` reads as
/// `target is busy`.
fn reason(stderr: &str, stdout: &str) -> String {
    let text = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    let line = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("failed");
    let line = line.rsplit_once("Error:").map_or(line, |(_, rest)| rest);
    let line = line.split_once(": ").map_or(line, |(code, rest)| {
        if code.contains('.') && !code.contains(' ') {
            rest
        } else {
            line
        }
    });
    line.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mount_point_is_read_from_the_programs_answer() {
        assert_eq!(
            parse_mounted("Mounted /dev/sdb1 at /run/media/me/STICK.\n"),
            Some(PathBuf::from("/run/media/me/STICK"))
        );
        assert_eq!(
            parse_mounted("Mounted /dev/sdb1 at '/run/media/me/MY STICK'.\n"),
            Some(PathBuf::from("/run/media/me/MY STICK"))
        );
        assert_eq!(parse_mounted("Unmounted /dev/sdb1.\n"), None);
        assert_eq!(parse_mounted(""), None);
    }

    #[test]
    fn a_failure_reads_as_one_plain_line() {
        assert_eq!(
            reason(
                "Error unmounting /dev/sdb1: GDBus.Error:org.freedesktop.UDisks2.Error.DeviceBusy: target is busy\n",
                ""
            ),
            "target is busy"
        );
        assert_eq!(
            reason("", "Object /dev/sdz does not exist"),
            "Object /dev/sdz does not exist"
        );
        assert_eq!(reason("", ""), "failed");
    }
}
