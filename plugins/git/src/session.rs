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

//! The plugin's state and how it reacts to each event, with no clock, process or output of its
//! own: every call is given the time and returns the effects the driver should carry out. That
//! keeps the rules that matter (when to ask git again, what to throw away when the folder leaves
//! the repository, what is worth sending) testable without running git.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use plugin_api::{SetView, Slot};

use crate::status::{Answer, Repo};
use crate::view;

/// Time between the end of one status run and the start of the next. Git's own work is the cost,
/// so this is counted from when a run finishes: a slow repository gets a proportionally lazier
/// refresh instead of back-to-back runs.
pub const REFRESH_AFTER: Duration = Duration::from_secs(3);

/// What the plugin is told.
pub enum Command {
    /// The host speaks a protocol version this plugin was not built for.
    Incompatible { host: u32 },
    /// The browsed folder at start, from `init`.
    Init { cwd: PathBuf },
    /// The reply to `get_state`: where the browser is and what its cursor is on.
    State {
        cwd: PathBuf,
        cursor: Option<PathBuf>,
    },
    /// The browsed folder changed.
    DirChanged(PathBuf),
    /// The cursor moved (`None` for an empty folder).
    CursorChanged(Option<PathBuf>),
    /// A status run ended.
    Answered { run: u64, answer: Answer },
}

/// What the driver should do.
pub enum Effect {
    /// Ask the host where the browser is.
    AskState,
    /// Run git for `dir` on another thread, stopping early once `cancel` is set, and report back
    /// with [`Command::Answered`] carrying `run`.
    Run {
        run: u64,
        dir: PathBuf,
        cancel: Arc<AtomicBool>,
    },
    /// Send this to the host.
    Show(SetView),
    /// Tell the host something is wrong, then stop.
    Fatal(String),
}

struct InFlight {
    run: u64,
    cancel: Arc<AtomicBool>,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
pub struct Session {
    /// The folder the state and the run in flight are about.
    dir: Option<PathBuf>,
    cursor: Option<PathBuf>,
    repo: Option<Repo>,
    running: Option<InFlight>,
    runs_started: u64,
    /// Earliest moment the next run may start; `None` means at once.
    next_at: Option<Instant>,
    /// What the host holds in each slot now, so only a difference is sent.
    sent: HashMap<Slot, SetView>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle(&mut self, command: Command, now: Instant) -> Vec<Effect> {
        let mut effects = Vec::new();
        match command {
            Command::Incompatible { host } => {
                effects.push(Effect::Fatal(format!(
                    "the host speaks plugin protocol {host}, this plugin was built for {}",
                    plugin_api::API_VERSION
                )));
                return effects;
            }
            Command::Init { cwd } => {
                self.move_to(cwd);
                effects.push(Effect::AskState);
            }
            Command::State { cwd, cursor } => {
                self.move_to(cwd);
                self.cursor = cursor;
            }
            Command::DirChanged(dir) => self.move_to(dir),
            Command::CursorChanged(cursor) => self.cursor = cursor,
            Command::Answered { run, answer } => {
                if self.running.as_ref().is_some_and(|r| r.run == run) {
                    self.running = None;
                    self.next_at = Some(now + REFRESH_AFTER);
                    match answer {
                        Answer::NotARepo => self.repo = None,
                        // Git could not answer this time; keep showing the last good result.
                        Answer::Failed => {}
                        Answer::Repo(repo) => self.repo = Some(*repo),
                    }
                }
            }
        }
        self.settle(now, &mut effects);
        effects
    }

    /// Called when [`Session::deadline`] has passed.
    pub fn tick(&mut self, now: Instant) -> Vec<Effect> {
        let mut effects = Vec::new();
        self.settle(now, &mut effects);
        effects
    }

    /// When the next run is due, or `None` if nothing is waiting on the clock.
    pub fn deadline(&self) -> Option<Instant> {
        if self.running.is_some() || self.dir.is_none() {
            return None;
        }
        self.next_at
    }

    fn move_to(&mut self, dir: PathBuf) {
        if self.dir.as_deref() == Some(dir.as_path()) {
            return;
        }
        // A status describes the whole repository, so moving within it keeps what is on screen;
        // leaving it drops the branch straight away rather than after the next run.
        if self
            .repo
            .as_ref()
            .is_some_and(|repo| !dir.starts_with(&repo.root))
        {
            self.repo = None;
        }
        // The run in flight was for the old folder; dropping it cancels it.
        self.running = None;
        self.next_at = None;
        self.dir = Some(dir);
    }

    fn settle(&mut self, now: Instant, effects: &mut Vec<Effect>) {
        if let Some(dir) = &self.dir
            && self.running.is_none()
            && self.next_at.is_none_or(|at| now >= at)
        {
            self.runs_started += 1;
            let cancel = Arc::new(AtomicBool::new(false));
            effects.push(Effect::Run {
                run: self.runs_started,
                dir: dir.clone(),
                cancel: Arc::clone(&cancel),
            });
            self.running = Some(InFlight {
                run: self.runs_started,
                cancel,
            });
        }
        let wanted = self.wanted();
        for (slot, set) in wanted {
            let unchanged = self.sent.get(&slot) == Some(&set);
            // The host starts with every slot empty, so clearing one it never held says nothing.
            let nothing_to_clear = set.view.is_none() && !self.sent.contains_key(&slot);
            if unchanged || nothing_to_clear {
                continue;
            }
            self.sent.insert(slot, set.clone());
            effects.push(Effect::Show(set));
        }
    }

    /// What each slot should hold now.
    fn wanted(&self) -> [(Slot, SetView); 3] {
        let empty = |slot| SetView { slot, view: None };
        match (&self.repo, &self.dir) {
            (Some(repo), Some(dir)) => [
                (Slot::StatusRight, view::repository(repo)),
                (Slot::StatusEntry, view::entry(repo, self.cursor.as_deref())),
                (Slot::EntryBadge, view::badges(repo, dir)),
            ],
            _ => [
                (Slot::StatusRight, empty(Slot::StatusRight)),
                (Slot::StatusEntry, empty(Slot::StatusEntry)),
                (Slot::EntryBadge, empty(Slot::EntryBadge)),
            ],
        }
    }

    #[cfg(test)]
    fn dir(&self) -> Option<&std::path::Path> {
        self.dir.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::{Counts, FileState, Head, Parsed};

    fn repo(root: &str) -> Repo {
        Repo::new(
            PathBuf::from(root),
            Parsed {
                head: Head::Branch("main".into()),
                ahead_behind: None,
                counts: Counts::default(),
                entries: vec![(PathBuf::from("a.txt"), FileState::Modified)],
            },
        )
    }

    fn runs(effects: &[Effect]) -> Vec<(u64, PathBuf)> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Run { run, dir, .. } => Some((*run, dir.clone())),
                _ => None,
            })
            .collect()
    }

    fn shown(effects: &[Effect]) -> Vec<(Slot, bool)> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Show(set) => Some((set.slot, set.view.is_some())),
                _ => None,
            })
            .collect()
    }

    fn answered(run: u64, repo: Repo) -> Command {
        Command::Answered {
            run,
            answer: Answer::Repo(Box::new(repo)),
        }
    }

    #[test]
    fn init_asks_where_the_browser_is_and_starts_a_run_for_the_folder() {
        let mut session = Session::new();
        let effects = session.handle(Command::Init { cwd: "/r".into() }, Instant::now());
        assert!(effects.iter().any(|e| matches!(e, Effect::AskState)));
        assert_eq!(runs(&effects), [(1, PathBuf::from("/r"))]);
    }

    #[test]
    fn an_answer_fills_every_slot_once_and_a_repeat_sends_nothing() {
        let (mut session, now) = (Session::new(), Instant::now());
        session.handle(Command::Init { cwd: "/r".into() }, now);
        session.handle(
            Command::State {
                cwd: "/r".into(),
                cursor: Some("/r/a.txt".into()),
            },
            now,
        );
        let effects = session.handle(answered(1, repo("/r")), now);
        assert_eq!(
            shown(&effects),
            [
                (Slot::StatusRight, true),
                (Slot::StatusEntry, true),
                (Slot::EntryBadge, true)
            ]
        );
        // The next run's identical answer changes nothing on screen.
        let later = now + REFRESH_AFTER;
        let effects = session.tick(later);
        assert_eq!(runs(&effects).len(), 1);
        let effects = session.handle(answered(2, repo("/r")), later);
        assert!(shown(&effects).is_empty());
    }

    #[test]
    fn the_next_run_waits_three_seconds_from_the_end_of_the_last() {
        let (mut session, now) = (Session::new(), Instant::now());
        session.handle(Command::Init { cwd: "/r".into() }, now);
        let end = now + Duration::from_secs(7);
        session.handle(answered(1, repo("/r")), end);
        assert_eq!(session.deadline(), Some(end + REFRESH_AFTER));
        assert!(runs(&session.tick(end + REFRESH_AFTER - Duration::from_millis(1))).is_empty());
        assert_eq!(runs(&session.tick(end + REFRESH_AFTER)).len(), 1);
        assert_eq!(session.deadline(), None, "a run in flight needs no timer");
    }

    #[test]
    fn moving_the_cursor_changes_only_the_entry_slot() {
        let (mut session, now) = (Session::new(), Instant::now());
        session.handle(Command::Init { cwd: "/r".into() }, now);
        session.handle(answered(1, repo("/r")), now);
        let effects = session.handle(Command::CursorChanged(Some("/r/a.txt".into())), now);
        assert_eq!(shown(&effects), [(Slot::StatusEntry, true)]);
        let effects = session.handle(Command::CursorChanged(Some("/r/clean".into())), now);
        assert_eq!(shown(&effects), [(Slot::StatusEntry, false)]);
    }

    #[test]
    fn leaving_the_repository_empties_every_slot_before_any_run_answers() {
        let (mut session, now) = (Session::new(), Instant::now());
        session.handle(Command::Init { cwd: "/r".into() }, now);
        session.handle(Command::CursorChanged(Some("/r/a.txt".into())), now);
        session.handle(answered(1, repo("/r")), now);
        let effects = session.handle(Command::DirChanged("/elsewhere".into()), now);
        assert_eq!(
            shown(&effects),
            [
                (Slot::StatusRight, false),
                (Slot::StatusEntry, false),
                (Slot::EntryBadge, false)
            ]
        );
        assert_eq!(runs(&effects), [(2, PathBuf::from("/elsewhere"))]);
    }

    #[test]
    fn moving_within_the_repository_keeps_the_status_and_re_marks_the_new_folder() {
        let (mut session, now) = (Session::new(), Instant::now());
        session.handle(Command::Init { cwd: "/r".into() }, now);
        session.handle(answered(1, repo("/r")), now);
        let effects = session.handle(Command::DirChanged("/r/sub".into()), now);
        // Nothing in /r/sub changed, so the marks of /r are taken away; the branch stays.
        assert_eq!(shown(&effects), [(Slot::EntryBadge, false)]);
        assert_eq!(session.dir(), Some(std::path::Path::new("/r/sub")));
    }

    #[test]
    fn the_marks_follow_the_folder_not_the_repository() {
        let (mut session, now) = (Session::new(), Instant::now());
        session.handle(Command::Init { cwd: "/r".into() }, now);
        session.handle(answered(1, repo("/r")), now);
        session.handle(Command::DirChanged("/r/sub".into()), now);
        let effects = session.handle(Command::DirChanged("/r".into()), now);
        assert_eq!(shown(&effects), [(Slot::EntryBadge, true)]);
    }

    #[test]
    fn the_run_for_a_folder_left_behind_is_cancelled_and_its_answer_ignored() {
        let (mut session, now) = (Session::new(), Instant::now());
        let effects = session.handle(Command::Init { cwd: "/a".into() }, now);
        let cancel = effects
            .iter()
            .find_map(|e| match e {
                Effect::Run { cancel, .. } => Some(Arc::clone(cancel)),
                _ => None,
            })
            .expect("a run started");
        session.handle(Command::DirChanged("/b".into()), now);
        assert!(cancel.load(Ordering::Relaxed));
        let effects = session.handle(answered(1, repo("/a")), now);
        assert!(shown(&effects).is_empty(), "a stale answer shows nothing");
    }

    #[test]
    fn a_failed_run_keeps_the_last_good_answer_and_not_a_repo_clears_it() {
        let (mut session, now) = (Session::new(), Instant::now());
        session.handle(Command::Init { cwd: "/r".into() }, now);
        session.handle(Command::CursorChanged(Some("/r/a.txt".into())), now);
        session.handle(answered(1, repo("/r")), now);
        let t = now + REFRESH_AFTER;
        session.tick(t);
        let effects = session.handle(
            Command::Answered {
                run: 2,
                answer: Answer::Failed,
            },
            t,
        );
        assert!(shown(&effects).is_empty());
        let t = t + REFRESH_AFTER;
        session.tick(t);
        let effects = session.handle(
            Command::Answered {
                run: 3,
                answer: Answer::NotARepo,
            },
            t,
        );
        assert_eq!(
            shown(&effects),
            [
                (Slot::StatusRight, false),
                (Slot::StatusEntry, false),
                (Slot::EntryBadge, false)
            ]
        );
    }

    #[test]
    fn nothing_is_sent_to_empty_slots_that_were_never_filled() {
        let (mut session, now) = (Session::new(), Instant::now());
        session.handle(
            Command::Init {
                cwd: "/plain".into(),
            },
            now,
        );
        let effects = session.handle(
            Command::Answered {
                run: 1,
                answer: Answer::NotARepo,
            },
            now,
        );
        assert!(shown(&effects).is_empty());
    }

    #[test]
    fn a_host_of_another_version_stops_the_plugin() {
        let effects = Session::new().handle(Command::Incompatible { host: 99 }, Instant::now());
        assert!(matches!(effects.as_slice(), [Effect::Fatal(_)]));
    }
}
