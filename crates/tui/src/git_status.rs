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

//! The status bar's git segment: which branch the browsed directory is on, how far it is from its
//! upstream, how many files are staged, modified or untracked, and the state of each path (so the
//! selected file, or a folder holding changes, can say so).
//!
//! The answer comes from the `git` binary — `git status --porcelain=v2 --branch -z` — run on the
//! blocking pool, one job at a time, the way `live_refresh` re-lists a directory. Three choices
//! keep that safe to leave running while the user works in the same repository:
//!
//! - `--no-optional-locks` stops a status refresh from taking git's index lock, which would make
//!   a `git commit` typed in a mini-shell at the wrong instant fail with "index.lock exists";
//! - a job is cancelled when the browsed directory leaves the repository, and killed after
//!   `TIMEOUT` so a huge repository on a slow disk cannot pile up processes;
//! - whether a directory is in a repository at all is decided in-process by looking for `.git`,
//!   so browsing outside one never starts a process.
//!
//! A missing `git`, an unreadable repository or a timeout simply means no segment (or the last
//! good one, if there was one): the bar is decoration and must never get in the way. The parser
//! is a pure function over bytes so it can be tested without running git.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// Time between the end of one status run and the start of the next. Git's own work is the cost,
/// so this is counted from when a run finishes: a slow repository gets a proportionally lazier
/// refresh instead of back-to-back runs.
const REFRESH_AFTER: Duration = Duration::from_secs(3);

/// A status run still going after this is killed and reported as failed.
const TIMEOUT: Duration = Duration::from_secs(10);

/// How often a running job checks whether git finished or it was cancelled.
const POLL_EVERY: Duration = Duration::from_millis(15);

/// Output beyond this is never read, so git blocks on a full pipe and the run times out rather
/// than the process filling memory.
const MAX_OUTPUT: u64 = 32 * 1024 * 1024;

/// Paths kept for per-file lookups. Counts stay exact past it; only the lookup stops growing.
const MAX_TRACKED: usize = 100_000;

/// Where `HEAD` points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    Branch(String),
    /// Not on a branch; holds the commit's short id.
    Detached(String),
}

/// One path's state, from git's two-letter `XY` code (X the index, Y the work tree).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Conflicted,
    Untracked,
    /// Changed in the index only.
    Staged,
    /// Changed in the work tree only.
    Modified,
    /// Staged, and changed again since.
    StagedModified,
}

impl FileState {
    /// The single state that stands for two, for a folder holding both: a conflict outranks
    /// everything, an untracked file only shows when nothing tracked has changed, and different
    /// tracked changes together read as staged-and-modified.
    pub fn merge(self, other: Self) -> Self {
        use FileState::{Conflicted, StagedModified, Untracked};
        match (self, other) {
            (Conflicted, _) | (_, Conflicted) => Conflicted,
            (Untracked, tracked) | (tracked, Untracked) => tracked,
            (a, b) if a == b => a,
            _ => StagedModified,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FileState::Conflicted => "conflict",
            FileState::Untracked => "untracked",
            FileState::Staged => "staged",
            FileState::Modified => "modified",
            FileState::StagedModified => "staged+modified",
        }
    }
}

/// How many files are in each state. A file changed in both the index and the work tree counts
/// in both, the way a shell prompt shows `+1 ~1` for it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub staged: u32,
    pub modified: u32,
    pub untracked: u32,
    pub conflicted: u32,
}

/// What `git status` said, before it is tied to a repository root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub head: Head,
    /// `None` when the branch has no upstream.
    pub ahead_behind: Option<(u32, u32)>,
    pub counts: Counts,
    /// Root-relative path and state, in git's order.
    pub entries: Vec<(PathBuf, FileState)>,
}

fn field_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn state_from_xy(xy: &[u8]) -> Option<(FileState, bool, bool)> {
    let (&x, &y) = (xy.first()?, xy.get(1)?);
    let (staged, modified) = (x != b'.', y != b'.');
    let state = match (staged, modified) {
        (true, true) => FileState::StagedModified,
        (true, false) => FileState::Staged,
        (false, true) => FileState::Modified,
        (false, false) => return None,
    };
    Some((state, staged, modified))
}

fn relative_path(bytes: &[u8]) -> PathBuf {
    // An untracked folder is listed with a trailing slash; the lookup keys have none.
    let trimmed = bytes.strip_suffix(b"/").unwrap_or(bytes);
    PathBuf::from(OsStr::from_bytes(trimmed))
}

/// Parses the output of `git status --porcelain=v2 --branch -z`. Anything it does not recognise
/// is skipped, so a newer git that adds a record type cannot make this fail.
pub fn parse(output: &[u8]) -> Parsed {
    let mut branch: Option<String> = None;
    let mut oid = String::new();
    let mut ahead_behind = None;
    let mut counts = Counts::default();
    let mut entries = Vec::new();

    let mut records = output.split(|&b| b == 0);
    while let Some(record) = records.next() {
        match record.first() {
            Some(b'#') => {
                let text = field_text(record);
                let mut words = text.split(' ').skip(1);
                match (words.next(), words.next(), words.next()) {
                    (Some("branch.head"), Some(name), _) => branch = Some(name.to_string()),
                    (Some("branch.oid"), Some(id), _) => oid = id.to_string(),
                    (Some("branch.ab"), Some(ahead), Some(behind)) => {
                        let number = |s: &str| s.get(1..).and_then(|n| n.parse::<u32>().ok());
                        ahead_behind = number(ahead).zip(number(behind));
                    }
                    _ => {}
                }
            }
            // `1 XY sub mH mI mW hH hI path`
            Some(b'1') => {
                let fields: Vec<&[u8]> = record.splitn(9, |&b| b == b' ').collect();
                if let Some((state, staged, modified)) =
                    fields.get(1).and_then(|xy| state_from_xy(xy))
                    && let Some(path) = fields.get(8)
                {
                    counts.staged += u32::from(staged);
                    counts.modified += u32::from(modified);
                    entries.push((relative_path(path), state));
                }
            }
            // `2 XY sub mH mI mW hH hI Xscore path`, then the original path as its own record.
            Some(b'2') => {
                let _original_path = records.next();
                let fields: Vec<&[u8]> = record.splitn(10, |&b| b == b' ').collect();
                if let Some((state, staged, modified)) =
                    fields.get(1).and_then(|xy| state_from_xy(xy))
                    && let Some(path) = fields.get(9)
                {
                    counts.staged += u32::from(staged);
                    counts.modified += u32::from(modified);
                    entries.push((relative_path(path), state));
                }
            }
            // `u XY sub m1 m2 m3 mW h1 h2 h3 path`
            Some(b'u') => {
                let fields: Vec<&[u8]> = record.splitn(11, |&b| b == b' ').collect();
                if let Some(path) = fields.get(10) {
                    counts.conflicted += 1;
                    entries.push((relative_path(path), FileState::Conflicted));
                }
            }
            // `? path`
            Some(b'?') if record.get(1) == Some(&b' ') => {
                counts.untracked += 1;
                entries.push((relative_path(&record[2..]), FileState::Untracked));
            }
            _ => {}
        }
    }

    let head = match branch.as_deref() {
        Some("(detached)") | None => Head::Detached(oid.chars().take(7).collect()),
        Some(name) => Head::Branch(name.to_string()),
    };
    Parsed {
        head,
        ahead_behind,
        counts,
        entries,
    }
}

/// The state of a repository as of the last status run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub root: PathBuf,
    pub head: Head,
    pub ahead_behind: Option<(u32, u32)>,
    pub counts: Counts,
    /// Absolute path to state, for every changed path and — merged — every folder above one.
    states: HashMap<PathBuf, FileState>,
}

impl Repo {
    pub fn new(root: PathBuf, parsed: Parsed) -> Self {
        let mut states: HashMap<PathBuf, FileState> = HashMap::new();
        for (relative, state) in parsed.entries.into_iter().take(MAX_TRACKED) {
            let mut path = root.join(relative);
            let mut record = |path: &Path| {
                states
                    .entry(path.to_path_buf())
                    .and_modify(|seen| *seen = seen.merge(state))
                    .or_insert(state);
            };
            record(&path);
            // A folder shows the state of what it holds, so a change deep down is visible from
            // the directory listing above it. The root itself has no row to show it on.
            while path.pop() && path != root && path.starts_with(&root) {
                record(&path);
            }
        }
        Self {
            root,
            head: parsed.head,
            ahead_behind: parsed.ahead_behind,
            counts: parsed.counts,
            states,
        }
    }

    /// The state of `path` (an absolute path in the same spelling the browser uses), or `None`
    /// when it is clean, ignored or outside the repository.
    pub fn state_of(&self, path: &Path) -> Option<FileState> {
        self.states.get(path).copied()
    }
}

/// The nearest ancestor of `dir` (or `dir` itself) holding a `.git` entry — a folder for an
/// ordinary repository, a file for a linked worktree or submodule.
fn find_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(Path::to_path_buf)
}

/// Runs `git status` in `root`, returning its output, or `None` if git is missing, failed, timed
/// out, or `cancel` was set.
fn run_status(root: &Path, cancel: &AtomicBool) -> Option<Vec<u8>> {
    let mut child = Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(root)
        .args([
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=normal",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    // Read on its own thread: a status listing bigger than the pipe buffer would otherwise block
    // git on a full pipe while this loop waits for it to exit.
    let reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        stdout
            .take(MAX_OUTPUT)
            .read_to_end(&mut buffer)
            .map(|_| buffer)
    });

    let started = Instant::now();
    let status: Option<ExitStatus> = loop {
        // Checked before `try_wait` so a cancel that arrives with the answer still wins.
        if cancel.load(Ordering::Relaxed) || started.elapsed() >= TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => std::thread::sleep(POLL_EVERY),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    // Killed or finished, the pipe is closed, so the reader ends either way.
    let output = reader.join().ok()?.ok()?;
    status.filter(ExitStatus::success).map(|_| output)
}

/// What one background job found.
enum Answer {
    NotARepo,
    /// Git could not answer this time; keep showing the last good result.
    Failed,
    Repo(Box<Repo>),
}

fn read_repo(dir: &Path, cancel: &AtomicBool) -> Answer {
    let Some(root) = find_root(dir) else {
        return Answer::NotARepo;
    };
    match run_status(&root, cancel) {
        Some(output) => Answer::Repo(Box::new(Repo::new(root, parse(&output)))),
        None => Answer::Failed,
    }
}

struct Job {
    cancel: Arc<AtomicBool>,
    rx: UnboundedReceiver<Answer>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub struct GitStatus {
    handle: tokio::runtime::Handle,
    /// `[git_status]` in `config.toml`; off means no process is ever started.
    enabled: bool,
    /// The directory the current state and job are about.
    dir: Option<PathBuf>,
    repo: Option<Repo>,
    job: Option<Job>,
    /// Earliest moment the next job may start; `None` means at once.
    next_at: Option<Instant>,
}

impl GitStatus {
    pub fn new(handle: tokio::runtime::Handle, enabled: bool) -> Self {
        Self {
            handle,
            enabled,
            dir: None,
            repo: None,
            job: None,
            next_at: None,
        }
    }

    /// Call once per render tick with the browsed directory. Picks up a finished job, and starts
    /// the next when the directory changed or `REFRESH_AFTER` has passed since the last one.
    pub fn poll(&mut self, dir: &Path, now: Instant) {
        if !self.enabled {
            return;
        }
        if self.dir.as_deref() != Some(dir) {
            self.dir = Some(dir.to_path_buf());
            // A status describes the whole repository, so moving within it keeps what is on
            // screen; leaving it drops the branch straight away rather than after the next job.
            if self
                .repo
                .as_ref()
                .is_some_and(|r| !dir.starts_with(&r.root))
            {
                self.repo = None;
            }
            // The job in flight was for the old directory; dropping it cancels it.
            self.job = None;
            self.next_at = None;
        }
        if let Some(job) = self.job.as_mut() {
            match job.rx.try_recv() {
                Ok(answer) => self.apply(answer),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {}
            }
            self.job = None;
            self.next_at = Some(now + REFRESH_AFTER);
        }
        if self.next_at.is_none_or(|at| now >= at) {
            self.job = Some(self.start(dir.to_path_buf()));
        }
    }

    /// The repository the browsed directory is in, if it is in one and git answered.
    pub fn repo(&self) -> Option<&Repo> {
        self.repo.as_ref()
    }

    fn apply(&mut self, answer: Answer) {
        match answer {
            Answer::NotARepo => self.repo = None,
            Answer::Failed => {}
            Answer::Repo(repo) => self.repo = Some(*repo),
        }
    }

    fn start(&self, dir: PathBuf) -> Job {
        let (tx, rx) = unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_bg = Arc::clone(&cancel);
        self.handle.spawn_blocking(move || {
            let _ = tx.send(read_repo(&dir, &cancel_bg));
        });
        Job { cancel, rx }
    }
}

#[cfg(test)]
mod tests {
    //! The last group of tests runs the real `git`, so they need it installed (as the project's
    //! own development does).

    use super::*;
    use proptest::prelude::*;

    const ALL_STATES: [FileState; 5] = [
        FileState::Conflicted,
        FileState::Untracked,
        FileState::Staged,
        FileState::Modified,
        FileState::StagedModified,
    ];

    fn record(fields: &[&str]) -> Vec<u8> {
        let mut bytes = fields.join(" ").into_bytes();
        bytes.push(0);
        bytes
    }

    #[test]
    fn a_real_looking_status_parses_into_branch_counts_and_paths() {
        let mut out = Vec::new();
        out.extend(record(&["# branch.oid", "0123456789abcdef"]));
        out.extend(record(&["# branch.head", "main"]));
        out.extend(record(&["# branch.upstream", "origin/main"]));
        out.extend(record(&["# branch.ab", "+2", "-1"]));
        out.extend(record(&[
            "1",
            ".M",
            "N...",
            "100644",
            "100644",
            "100644",
            "a",
            "b",
            "src/lib.rs",
        ]));
        out.extend(record(&[
            "1",
            "A.",
            "N...",
            "000000",
            "100644",
            "100644",
            "a",
            "b",
            "new file.txt",
        ]));
        out.extend(record(&[
            "1", "MM", "N...", "100644", "100644", "100644", "a", "b", "both.rs",
        ]));
        out.extend(record(&[
            "2",
            "R.",
            "N...",
            "100644",
            "100644",
            "100644",
            "a",
            "b",
            "R100",
            "renamed.rs",
        ]));
        out.extend(record(&["old name.rs"]));
        out.extend(record(&[
            "u", "UU", "N...", "100644", "100644", "100644", "100644", "a", "b", "c", "clash.rs",
        ]));
        out.extend(record(&["?", "scratch/"]));

        let parsed = parse(&out);
        assert_eq!(parsed.head, Head::Branch("main".into()));
        assert_eq!(parsed.ahead_behind, Some((2, 1)));
        assert_eq!(
            parsed.counts,
            Counts {
                staged: 3,
                modified: 2,
                untracked: 1,
                conflicted: 1
            }
        );
        let lookup = |name: &str| {
            parsed
                .entries
                .iter()
                .find(|(path, _)| path == Path::new(name))
                .map(|(_, state)| *state)
        };
        assert_eq!(lookup("src/lib.rs"), Some(FileState::Modified));
        assert_eq!(lookup("new file.txt"), Some(FileState::Staged));
        assert_eq!(lookup("both.rs"), Some(FileState::StagedModified));
        assert_eq!(lookup("renamed.rs"), Some(FileState::Staged));
        assert_eq!(
            lookup("old name.rs"),
            None,
            "the rename's source is not a record of its own"
        );
        assert_eq!(lookup("clash.rs"), Some(FileState::Conflicted));
        assert_eq!(
            lookup("scratch"),
            Some(FileState::Untracked),
            "the trailing slash is dropped"
        );
    }

    #[test]
    fn a_detached_head_shows_the_short_commit_and_no_upstream_means_no_counts() {
        let mut out = Vec::new();
        out.extend(record(&["# branch.oid", "0123456789abcdef"]));
        out.extend(record(&["# branch.head", "(detached)"]));
        let parsed = parse(&out);
        assert_eq!(parsed.head, Head::Detached("0123456".into()));
        assert_eq!(parsed.ahead_behind, None);
    }

    #[test]
    fn empty_and_unrecognised_output_is_an_empty_status() {
        let parsed = parse(b"");
        assert_eq!(parsed.counts, Counts::default());
        assert!(parsed.entries.is_empty());
        assert!(
            parse(b"! ignored.log\0X something new\0")
                .entries
                .is_empty()
        );
    }

    #[test]
    fn merging_states_is_commutative_associative_and_idempotent() {
        for a in ALL_STATES {
            assert_eq!(a.merge(a), a);
            for b in ALL_STATES {
                assert_eq!(a.merge(b), b.merge(a), "{a:?} {b:?}");
                for c in ALL_STATES {
                    assert_eq!(
                        a.merge(b).merge(c),
                        a.merge(b.merge(c)),
                        "{a:?} {b:?} {c:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_conflict_outranks_all_and_untracked_yields_to_tracked_changes() {
        for other in ALL_STATES {
            assert_eq!(FileState::Conflicted.merge(other), FileState::Conflicted);
        }
        assert_eq!(
            FileState::Untracked.merge(FileState::Staged),
            FileState::Staged
        );
        assert_eq!(
            FileState::Staged.merge(FileState::Modified),
            FileState::StagedModified
        );
    }

    #[test]
    fn a_change_deep_down_shows_on_every_folder_above_it_but_not_the_root() {
        let root = PathBuf::from("/repo");
        let parsed = Parsed {
            head: Head::Branch("main".into()),
            ahead_behind: None,
            counts: Counts::default(),
            entries: vec![
                (PathBuf::from("a/b/c.rs"), FileState::Modified),
                (PathBuf::from("a/d.rs"), FileState::Staged),
            ],
        };
        let repo = Repo::new(root, parsed);
        assert_eq!(
            repo.state_of(Path::new("/repo/a/b/c.rs")),
            Some(FileState::Modified)
        );
        assert_eq!(
            repo.state_of(Path::new("/repo/a/b")),
            Some(FileState::Modified)
        );
        assert_eq!(
            repo.state_of(Path::new("/repo/a")),
            Some(FileState::StagedModified)
        );
        assert_eq!(repo.state_of(Path::new("/repo")), None);
        assert_eq!(repo.state_of(Path::new("/repo/other")), None);
    }

    fn arb_entry() -> impl Strategy<Value = (String, [u8; 2])> {
        // Names with spaces are the point: the path is the last field and must survive them.
        let xy = prop_oneof![
            Just(*b".M"),
            Just(*b"M."),
            Just(*b"MM"),
            Just(*b"A."),
            Just(*b"D."),
            Just(*b".D"),
        ];
        ("[a-z][a-z0-9 ._-]{0,12}[a-z0-9]", xy)
    }

    proptest! {
        /// Every ordinary record comes back with its path intact and the counts add up: one
        /// staged per non-`.` in X, one modified per non-`.` in Y.
        #[test]
        fn ordinary_records_round_trip_through_the_parser(
            entries in proptest::collection::vec(arb_entry(), 0..20),
            untracked in proptest::collection::vec("[a-z]{1,8}", 0..5),
        ) {
            let mut out = Vec::new();
            for (name, xy) in &entries {
                let xy = std::str::from_utf8(xy).unwrap();
                out.extend(record(&["1", xy, "N...", "100644", "100644", "100644", "a", "b", name]));
            }
            for name in &untracked {
                out.extend(record(&["?", name]));
            }

            let parsed = parse(&out);
            prop_assert_eq!(parsed.entries.len(), entries.len() + untracked.len());
            for ((name, _), (path, _)) in entries.iter().zip(&parsed.entries) {
                prop_assert_eq!(path, &PathBuf::from(name));
            }
            prop_assert_eq!(
                parsed.counts.staged as usize,
                entries.iter().filter(|(_, xy)| xy[0] != b'.').count()
            );
            prop_assert_eq!(
                parsed.counts.modified as usize,
                entries.iter().filter(|(_, xy)| xy[1] != b'.').count()
            );
            prop_assert_eq!(parsed.counts.untracked as usize, untracked.len());
        }

        /// Whatever bytes git (or a future git) sends, parsing returns instead of panicking.
        #[test]
        fn arbitrary_bytes_never_panic_the_parser(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
            let _ = parse(&bytes);
        }
    }

    // ---- against the real git ----------------------------------------------------------------

    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("minuteman-git-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git must be installed to run these tests");
        assert!(status.success(), "git {args:?} failed");
    }

    fn wait_for_repo(status: &mut GitStatus, dir: &Path, start: Instant) -> Option<Repo> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut now = start;
        loop {
            status.poll(dir, now);
            if status.job.is_none() {
                return status.repo().cloned();
            }
            assert!(Instant::now() < deadline, "git status never finished");
            std::thread::sleep(Duration::from_millis(10));
            now = Instant::now();
        }
    }

    #[test]
    fn the_real_git_reports_branch_and_the_state_of_each_path() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = scratch("real");
        git(&dir, &["init", "-q"]);
        std::fs::write(dir.join("tracked.txt"), "one").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/deep.txt"), "one").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "first"]);
        std::fs::write(dir.join("sub/deep.txt"), "two").unwrap();
        std::fs::write(dir.join("new file.txt"), "x").unwrap();

        let mut status = GitStatus::new(runtime.handle().clone(), true);
        let repo = wait_for_repo(&mut status, &dir, Instant::now()).expect("a repository");
        assert_eq!(repo.head, Head::Branch("main".into()));
        assert_eq!(repo.ahead_behind, None);
        assert_eq!(
            repo.state_of(&dir.join("sub/deep.txt")),
            Some(FileState::Modified)
        );
        assert_eq!(repo.state_of(&dir.join("sub")), Some(FileState::Modified));
        assert_eq!(
            repo.state_of(&dir.join("new file.txt")),
            Some(FileState::Untracked)
        );
        assert_eq!(repo.state_of(&dir.join("tracked.txt")), None);
        assert_eq!(
            repo.counts,
            Counts {
                modified: 1,
                untracked: 1,
                ..Counts::default()
            }
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_status_run_does_not_touch_the_index() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = scratch("lock");
        git(&dir, &["init", "-q"]);
        std::fs::write(dir.join("a"), "1").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "first"]);
        // Rewriting the same content leaves the index's cached stat data stale, which is exactly
        // when a plain `git status` refreshes it: it takes the index lock and rewrites the file.
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(dir.join("a"), "1").unwrap();
        let index = dir.join(".git/index");
        let before = std::fs::metadata(&index).unwrap().modified().unwrap();

        let mut status = GitStatus::new(runtime.handle().clone(), true);
        let repo = wait_for_repo(&mut status, &dir, Instant::now()).expect("a repository");
        assert_eq!(
            repo.state_of(&dir.join("a")),
            None,
            "same content, so clean"
        );
        let after = std::fs::metadata(&index).unwrap().modified().unwrap();
        assert_eq!(before, after, "reading status must leave the index alone");
        assert!(!dir.join(".git/index.lock").exists());

        // Control: without the flag the same situation does rewrite the index, so the assertion
        // above is not passing vacuously.
        git(&dir, &["status", "-s"]);
        let plain = std::fs::metadata(&index).unwrap().modified().unwrap();
        assert_ne!(
            before, plain,
            "a plain `git status` should have refreshed the index"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_directory_outside_any_repository_has_no_repo_and_leaving_one_clears_it() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let repo_dir = scratch("in");
        git(&repo_dir, &["init", "-q"]);
        let outside = scratch("out");

        let mut status = GitStatus::new(runtime.handle().clone(), true);
        assert!(wait_for_repo(&mut status, &repo_dir, Instant::now()).is_some());
        // The move itself must drop the branch, before any job has answered.
        status.poll(&outside, Instant::now());
        assert!(status.repo().is_none());
        assert!(wait_for_repo(&mut status, &outside, Instant::now()).is_none());
        std::fs::remove_dir_all(&repo_dir).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn moving_within_a_repository_keeps_what_is_shown() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = scratch("within");
        git(&dir, &["init", "-q"]);
        std::fs::create_dir(dir.join("sub")).unwrap();

        let mut status = GitStatus::new(runtime.handle().clone(), true);
        assert!(wait_for_repo(&mut status, &dir, Instant::now()).is_some());
        status.poll(&dir.join("sub"), Instant::now());
        assert!(
            status.repo().is_some(),
            "same repository, no reason to blank the bar"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn switched_off_it_never_starts_anything() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = scratch("off");
        git(&dir, &["init", "-q"]);
        let mut status = GitStatus::new(runtime.handle().clone(), false);
        status.poll(&dir, Instant::now());
        assert!(status.job.is_none());
        assert!(status.repo().is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_cancelled_run_gives_no_output() {
        let dir = scratch("cancelled");
        git(&dir, &["init", "-q"]);
        let cancel = AtomicBool::new(true);
        assert!(run_status(&dir, &cancel).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
