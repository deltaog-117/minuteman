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

//! What a plugin that packs and unpacks archives exchanges with the host. The interface keeps the
//! forms, the prompts, the busy state and the trash; it names the work (the `archive` event, sent
//! to the plugin that `provides = ["archive"]`) and the plugin does it on the local disk, telling
//! the host about each entry it reaches (`archive_progress`) and how the whole request ended
//! (`archive_done`). A request in flight is stopped with an `archive_cancel` event.
//!
//! This module also holds the few types the interface and the plugin both have to name, so neither
//! needs the other: the archive [`Format`]s and their names, the compression [`Level`], the
//! [`Conflict`] policy and the typed [`Password`].
//!
//! A plugin may be wrong or hostile and its text ends up in the user's terminal, so
//! [`ArchiveProgress::checked`] and [`ArchiveDone::checked`] clean what the host passes on.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::{Zeroize, Zeroizing};

use crate::view::is_unsafe;

/// The event name an archiving plugin gets when it is given work.
pub const EVENT_ARCHIVE: &str = "archive";
/// The event name an archiving plugin gets when the work in flight should stop.
pub const EVENT_ARCHIVE_CANCEL: &str = "archive_cancel";
/// The notification that says which entry a request has reached: `archive_progress`.
pub const METHOD_ARCHIVE_PROGRESS: &str = "archive_progress";
/// The notification that says how a request ended: `archive_done`.
pub const METHOD_ARCHIVE_DONE: &str = "archive_done";
/// What a plugin lists under `provides` to be the one that packs and unpacks archives.
pub const PROVIDES_ARCHIVE: &str = "archive";

/// The most characters a [`Password`] holds. The buffer is reserved at this size up front, so it
/// never reallocates and leaves an old, unwiped copy of the password behind as it grows.
pub const MAX_PASSWORD_CHARS: usize = 256;
/// Most characters of a progress name or a failure message; longer text is cut.
pub const MAX_ARCHIVE_TEXT_CHARS: usize = 1024;
/// Most jobs one request holds, and so most indices a [`ArchiveDone`] lists as finished.
pub const MAX_ARCHIVE_JOBS: usize = 100_000;

/// The archive formats that can be read and written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Zip,
    Tar,
    TarGz,
}

impl Format {
    /// Which format `path` is, going by its name alone (`.zip`, `.jar`, `.tar`, `.tar.gz`,
    /// `.tgz`), the same rule the preview pane uses.
    pub fn of(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?.to_lowercase();
        if name.ends_with(".zip") || name.ends_with(".jar") {
            Some(Self::Zip)
        } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Some(Self::TarGz)
        } else if name.ends_with(".tar") {
            Some(Self::Tar)
        } else {
            None
        }
    }

    /// `path`'s file name without its archive extension (`photos.tar.gz` gives `photos`), or
    /// `None` if it is not an archive or nothing is left of the name.
    pub fn stem_of(path: &Path) -> Option<String> {
        let name = path.file_name()?.to_str()?;
        let lower = name.to_lowercase();
        let extension = [".tar.gz", ".tgz", ".tar", ".zip", ".jar"]
            .into_iter()
            .find(|extension| lower.ends_with(extension))?;
        // Lower-casing can change a name's byte length, so cut the original by characters.
        let kept = name.chars().count() - extension.chars().count();
        let stem: String = name.chars().take(kept).collect();
        (!stem.is_empty()).then_some(stem)
    }

    /// The file-name ending an archive of this format is given.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Tar => "tar",
            Self::TarGz => "tar.gz",
        }
    }
}

/// How hard a compression works to make an archive small. Ignored for a plain `tar`, which is not
/// compressed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// The quickest, for a big tree or a slow disk.
    Fast,
    #[default]
    Normal,
    /// The smallest, at the cost of time.
    Best,
}

/// What to do when a file a request would create already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Conflict {
    /// Fail, leaving both untouched.
    Abort,
    /// Leave what is there and carry on.
    Skip,
    /// Replace what is there.
    Overwrite,
}

/// A password for an encrypted archive. It is wiped from memory when dropped, never shown by
/// `Debug`, and capped at [`MAX_PASSWORD_CHARS`] characters.
///
/// It travels to the plugin inside the request that needs it, as one JSON string on the pipe to a
/// process the user's own session started. The line it is read from, and the text the parser makes
/// on the way, are not wiped: only this type is.
#[derive(Clone)]
pub struct Password(Zeroizing<String>);

impl Default for Password {
    fn default() -> Self {
        // Four bytes is the most one `char` takes in UTF-8.
        Self(Zeroizing::new(String::with_capacity(
            MAX_PASSWORD_CHARS * 4,
        )))
    }
}

impl Password {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `c`, unless it is a control character or the password is already as long as allowed.
    pub fn push(&mut self, c: char) {
        if !c.is_control() && self.char_count() < MAX_PASSWORD_CHARS {
            self.0.push(c);
        }
    }

    pub fn pop(&mut self) {
        self.0.pop();
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn char_count(&self) -> usize {
        self.0.chars().count()
    }

    /// The password's text, for the archive library that needs it.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A password from text that came off the wire, or `None` if it is longer than allowed or
    /// holds a control character (which [`Password::push`] would never have let in).
    fn from_wire(text: &str) -> Option<Self> {
        let mut password = Self::new();
        for c in text.chars() {
            if c.is_control() || password.char_count() >= MAX_PASSWORD_CHARS {
                return None;
            }
            password.0.push(c);
        }
        Some(password)
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(..)")
    }
}

impl PartialEq for Password {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_bytes() == other.0.as_bytes()
    }
}

impl Eq for Password {}

impl Serialize for Password {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

struct PasswordVisitor;

impl Visitor<'_> for PasswordVisitor {
    type Value = Password;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a password of at most {MAX_PASSWORD_CHARS} characters with no control character"
        )
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Password, E> {
        Password::from_wire(text).ok_or_else(|| E::custom("not a valid password"))
    }

    fn visit_string<E: de::Error>(self, mut text: String) -> Result<Password, E> {
        let password = Password::from_wire(&text);
        // The parser's own copy is ours to wipe.
        text.zeroize();
        password.ok_or_else(|| E::custom("not a valid password"))
    }
}

impl<'de> Deserialize<'de> for Password {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_string(PasswordVisitor)
    }
}

/// One archive to write: `sources` packed into `out`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompressJob {
    pub sources: Vec<PathBuf>,
    pub out: PathBuf,
}

/// One archive to read: `archive` unpacked into the folder `dest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractJob {
    pub archive: PathBuf,
    pub dest: PathBuf,
}

/// What a request asks for. Its jobs run in order and stop at the first failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ArchiveWork {
    /// Pack each job's sources into its archive, in `format`. A `password` seals a zip.
    Compress {
        jobs: Vec<CompressJob>,
        format: Format,
        level: Level,
        policy: Conflict,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<Password>,
    },
    /// Unpack each job's archive. Without a `password`, the request ends with
    /// [`ArchiveStatus::PasswordNeeded`] before writing anything if one of them is encrypted; with
    /// one, every encrypted archive is checked against it first.
    Extract {
        jobs: Vec<ExtractJob>,
        policy: Conflict,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<Password>,
    },
}

impl ArchiveWork {
    /// How many jobs the request holds.
    pub fn job_count(&self) -> usize {
        match self {
            Self::Compress { jobs, .. } => jobs.len(),
            Self::Extract { jobs, .. } => jobs.len(),
        }
    }
}

/// The parameters of an `archive` event: work to do, under a number the plugin's progress and
/// result carry back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveRequest {
    pub job: u64,
    pub work: ArchiveWork,
}

/// The parameters of an `archive_cancel` event: stop the request numbered `job`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveCancel {
    pub job: u64,
}

/// The parameters of `archive_progress`: the request `job` has reached the entry called `path`,
/// the `done`th of the request. A plugin may skip entries to keep the pipe light, so `done` is the
/// running total, not a count of messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveProgress {
    pub job: u64,
    pub done: u64,
    pub path: String,
}

impl ArchiveProgress {
    /// This progress with control and direction-changing characters removed and long text cut.
    pub fn checked(mut self) -> Self {
        cleaned(&mut self.path);
        self
    }
}

/// How a request ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArchiveStatus {
    /// Every job ran to its end (a job whose archive was left alone by `Skip` counts).
    Completed,
    /// Stopped by an `archive_cancel`; what was written before stays.
    Cancelled,
    /// An extract found `archive` encrypted and had no password. Nothing was written.
    PasswordNeeded { archive: PathBuf },
    /// The password was refused by `archive`. Nothing was written.
    WrongPassword { archive: PathBuf },
    /// A job failed; `message` says why, in words for the status line.
    Failed { message: String },
}

/// The parameters of `archive_done`: the request `job` has ended. `finished` lists the indices of
/// the jobs that completed and wrote something, so the interface can trash their originals even
/// when a later job failed or the request was cancelled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveDone {
    pub job: u64,
    #[serde(default)]
    pub finished: Vec<usize>,
    pub status: ArchiveStatus,
}

/// Why a result was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveError {
    TooManyJobs,
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyJobs => write!(f, "a request holds at most {MAX_ARCHIVE_JOBS} jobs"),
        }
    }
}

impl std::error::Error for ArchiveError {}

fn cleaned(text: &mut String) {
    text.retain(|c| !is_unsafe(c));
    if let Some((cut, _)) = text.char_indices().nth(MAX_ARCHIVE_TEXT_CHARS) {
        text.truncate(cut);
    }
}

impl ArchiveDone {
    /// This result with a failure message cleaned of control and direction-changing characters
    /// and cut short, or why it cannot be accepted.
    ///
    /// # Errors
    ///
    /// More than [`MAX_ARCHIVE_JOBS`] finished indices.
    pub fn checked(mut self) -> Result<Self, ArchiveError> {
        if self.finished.len() > MAX_ARCHIVE_JOBS {
            return Err(ArchiveError::TooManyJobs);
        }
        if let ArchiveStatus::Failed { message } = &mut self.status {
            cleaned(message);
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;

    fn done(status: ArchiveStatus) -> ArchiveDone {
        ArchiveDone {
            job: 1,
            finished: vec![0],
            status,
        }
    }

    #[test]
    fn a_name_is_an_archive_by_its_ending_alone() {
        let of = |name: &str| Format::of(Path::new(name));
        assert_eq!(of("a.zip"), Some(Format::Zip));
        assert_eq!(of("A.JAR"), Some(Format::Zip));
        assert_eq!(of("a.tar"), Some(Format::Tar));
        assert_eq!(of("a.tar.gz"), Some(Format::TarGz));
        assert_eq!(of("a.TGZ"), Some(Format::TarGz));
        assert_eq!(of("a.gz"), None);
        assert_eq!(of("zip"), None);
    }

    #[test]
    fn the_stem_drops_the_archive_ending_and_nothing_else() {
        let stem = |name: &str| Format::stem_of(Path::new(name));
        assert_eq!(stem("photos.tar.gz").as_deref(), Some("photos"));
        assert_eq!(stem("a.b.zip").as_deref(), Some("a.b"));
        assert_eq!(stem("İ.ZIP").as_deref(), Some("İ"));
        assert_eq!(stem(".zip"), None);
        assert_eq!(stem("notes.txt"), None);
    }

    #[test]
    fn a_format_names_its_extension() {
        assert_eq!(Format::Zip.extension(), "zip");
        assert_eq!(Format::Tar.extension(), "tar");
        assert_eq!(Format::TarGz.extension(), "tar.gz");
    }

    #[test]
    fn the_wire_names_are_stable() {
        assert_eq!(serde_json::to_string(&Format::TarGz).unwrap(), "\"tar_gz\"");
        assert_eq!(serde_json::to_string(&Level::Best).unwrap(), "\"best\"");
        assert_eq!(
            serde_json::to_string(&Conflict::Overwrite).unwrap(),
            "\"overwrite\""
        );
    }

    #[test]
    fn a_password_is_never_shown_by_debug() {
        let mut password = Password::new();
        "hunter2".chars().for_each(|c| password.push(c));
        assert_eq!(format!("{password:?}"), "Password(..)");
        let work = ArchiveWork::Extract {
            jobs: Vec::new(),
            policy: Conflict::Abort,
            password: Some(password),
        };
        assert!(!format!("{work:?}").contains("hunter2"));
    }

    #[test]
    fn a_password_ignores_control_characters_and_stops_at_its_limit() {
        let mut password = Password::new();
        password.push('\n');
        password.push('\u{1b}');
        assert!(password.is_empty());
        for _ in 0..MAX_PASSWORD_CHARS + 10 {
            password.push('é');
        }
        assert_eq!(password.char_count(), MAX_PASSWORD_CHARS);
    }

    #[test]
    fn a_password_crosses_the_wire_as_a_plain_string() {
        let work = ArchiveWork::Extract {
            jobs: vec![ExtractJob {
                archive: "/a.zip".into(),
                dest: "/a".into(),
            }],
            policy: Conflict::Skip,
            password: Password::from_wire("pä ss"),
        };
        let value = serde_json::to_value(&work).unwrap();
        assert_eq!(value["password"], json!("pä ss"));
        assert_eq!(value["op"], json!("extract"));
        let back: ArchiveWork = serde_json::from_value(value).unwrap();
        assert_eq!(back, work);
    }

    #[test]
    fn a_request_without_a_password_leaves_the_field_out() {
        let work = ArchiveWork::Compress {
            jobs: Vec::new(),
            format: Format::Zip,
            level: Level::Normal,
            policy: Conflict::Abort,
            password: None,
        };
        let value = serde_json::to_value(&work).unwrap();
        assert!(value.get("password").is_none());
        assert_eq!(serde_json::from_value::<ArchiveWork>(value).unwrap(), work);
    }

    #[test]
    fn a_password_with_a_control_character_or_too_long_is_refused() {
        assert!(serde_json::from_value::<Password>(json!("a\nb")).is_err());
        let long = "x".repeat(MAX_PASSWORD_CHARS + 1);
        assert!(serde_json::from_value::<Password>(json!(long)).is_err());
        let longest = "x".repeat(MAX_PASSWORD_CHARS);
        assert!(serde_json::from_value::<Password>(json!(longest)).is_ok());
    }

    #[test]
    fn a_result_names_how_it_ended() {
        let line = serde_json::to_string(&done(ArchiveStatus::WrongPassword {
            archive: "/a.zip".into(),
        }))
        .unwrap();
        assert_eq!(
            line,
            r#"{"job":1,"finished":[0],"status":{"kind":"wrong_password","archive":"/a.zip"}}"#
        );
        let read: ArchiveDone =
            serde_json::from_str(r#"{"job":2,"status":{"kind":"completed"}}"#).unwrap();
        assert_eq!(read.finished, Vec::<usize>::new());
    }

    #[test]
    fn too_many_finished_jobs_are_refused_and_the_limit_itself_is_not() {
        let with = |n: usize| ArchiveDone {
            job: 1,
            finished: vec![0; n],
            status: ArchiveStatus::Completed,
        };
        assert!(with(MAX_ARCHIVE_JOBS).checked().is_ok());
        assert_eq!(
            with(MAX_ARCHIVE_JOBS + 1).checked(),
            Err(ArchiveError::TooManyJobs)
        );
    }

    proptest! {
        #[test]
        fn nothing_unsafe_or_over_long_survives_checking(
            path in ".{0,1200}",
            message in ".{0,1200}",
        ) {
            let progress = ArchiveProgress { job: 1, done: 1, path }.checked();
            prop_assert!(progress.path.chars().count() <= MAX_ARCHIVE_TEXT_CHARS);
            prop_assert!(!progress.path.chars().any(is_unsafe));
            let result = done(ArchiveStatus::Failed { message }).checked().unwrap();
            let ArchiveStatus::Failed { message } = result.status else {
                unreachable!("checking never changes the kind")
            };
            prop_assert!(message.chars().count() <= MAX_ARCHIVE_TEXT_CHARS);
            prop_assert!(!message.chars().any(is_unsafe));
        }

        #[test]
        fn checking_twice_changes_nothing(message in ".{0,60}") {
            let once = done(ArchiveStatus::Failed { message }).checked().unwrap();
            prop_assert_eq!(once.clone().checked().unwrap(), once);
        }

        #[test]
        fn any_password_the_type_accepts_round_trips(text in "[^\\p{Cc}]{0,300}") {
            let mut password = Password::new();
            text.chars().for_each(|c| password.push(c));
            let value = serde_json::to_value(&password).unwrap();
            prop_assert_eq!(serde_json::from_value::<Password>(value).unwrap(), password);
        }
    }
}
