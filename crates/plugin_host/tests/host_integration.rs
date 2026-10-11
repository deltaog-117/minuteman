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

//! End-to-end coverage across a real process boundary: `PluginManager` spawns the reference
//! fixture (`tests/fixtures/plugin.rs`, built as `test-fixture-plugin`), fires a key event at it
//! exactly the way `tui` will, and the fixture calls back into `read_dir`/`create_file` — proving
//! the whole round trip (spawn, event out, request in, `file_ops` executed, reply out) works
//! against a genuinely separate program talking nothing but the line-delimited JSON protocol.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::KeyCode;
use plugin_host::{
    ActionDispatch, EventKind, JobOutcome, PluginManager, PluginState, Restart, StateSnapshot,
    Triggers, UiRequest,
};

fn scratch_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "minuteman-plugins-test-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const FIXTURE: &str = env!("CARGO_BIN_EXE_test-fixture-plugin");

#[test]
fn a_key_event_reaches_the_plugin_and_its_file_op_reaches_the_real_filesystem() {
    let dir = scratch_dir("roundtrip");
    std::fs::write(dir.join("a.txt"), b"hi").unwrap();
    std::fs::write(dir.join("b.txt"), b"hi").unwrap();
    let target = dir.join("a.txt");

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    manager
        .spawn(
            runtime.handle(),
            "fixture",
            FIXTURE,
            &[],
            Triggers {
                keys: vec![KeyCode::Char('b')],
                commands: Vec::new(),
                ..Triggers::default()
            },
            &dir,
        )
        .unwrap();

    assert!(manager.dispatch_key(KeyCode::Char('b'), &dir, std::slice::from_ref(&target)));

    let touched = dir.join("a.txt.touched-by-plugin");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut messages = Vec::new();
    while Instant::now() < deadline && !touched.exists() {
        messages.extend(manager.poll());
        std::thread::sleep(Duration::from_millis(20));
    }
    messages.extend(manager.poll());

    assert!(
        touched.exists(),
        "plugin-issued create_file never reached the real filesystem; log messages so far: \
         {messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.message.contains("entries: 2")),
        "expected a log reporting the two-entry directory listing; got: {messages:?}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_key_with_no_bound_plugin_is_not_dispatched() {
    let dir = scratch_dir("unbound");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    manager
        .spawn(
            runtime.handle(),
            "fixture",
            FIXTURE,
            &[],
            Triggers {
                keys: vec![KeyCode::Char('b')],
                commands: Vec::new(),
                ..Triggers::default()
            },
            &dir,
        )
        .unwrap();

    assert!(!manager.dispatch_key(KeyCode::Char('z'), &dir, &[]));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_missing_command_fails_to_spawn_without_panicking() {
    let dir = scratch_dir("missing-command");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();

    let result = manager.spawn(
        runtime.handle(),
        "broken",
        "this-binary-does-not-exist-anywhere",
        &[],
        Triggers::default(),
        &dir,
    );

    assert!(result.is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The Python example from `examples/plugins`, run by a real interpreter: nothing in it shares
/// code with this crate, so these tests prove the protocol is enough for a plugin written outside
/// Rust. They are skipped, with a note on stderr, on a machine with no `python3`.
const PYTHON_EXAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/plugins/dir_stats.py"
);

fn has_python() -> bool {
    let found = std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());
    if !found {
        eprintln!("skipping: python3 is not installed");
    }
    found
}

fn spawn_python_example(
    runtime: &tokio::runtime::Runtime,
    manager: &mut PluginManager,
    dir: &std::path::Path,
) {
    manager
        .spawn(
            runtime.handle(),
            "dir-stats",
            "python3",
            &[PYTHON_EXAMPLE.to_string()],
            Triggers {
                keys: vec![KeyCode::Char('i'), KeyCode::Char('I')],
                commands: vec!["stats".to_string()],
                ..Triggers::default()
            },
            dir,
        )
        .unwrap();
}

/// Polls until a log line contains `needle`, or panics with everything seen after five seconds.
#[track_caller]
fn wait_for_log(manager: &mut PluginManager, needle: &str) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        seen.extend(manager.poll().into_iter().map(|m| m.message));
        if seen.iter().any(|m| m.contains(needle)) {
            return seen;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no log containing {needle:?}; saw {seen:?}");
}

#[test]
fn a_python_plugin_answers_each_of_its_keys_and_names_the_one_pressed() {
    if !has_python() {
        return;
    }
    let dir = scratch_dir("python-keys");
    std::fs::write(dir.join("a.txt"), b"12345").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_python_example(&runtime, &mut manager, &dir);
    wait_for_log(&mut manager, "ready");

    assert!(manager.dispatch_key(KeyCode::Char('i'), &dir, &[]));
    wait_for_log(&mut manager, "[i] 1 files, 1 folders, 5 bytes");
    assert!(manager.dispatch_key(KeyCode::Char('I'), &dir, &[]));
    wait_for_log(&mut manager, "[I] 1 files, 1 folders, 5 bytes");
    assert!(!manager.dispatch_key(KeyCode::Char('j'), &dir, &[]));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_declared_colon_command_and_plugin_by_name_reach_the_python_plugin() {
    if !has_python() {
        return;
    }
    let dir = scratch_dir("python-commands");
    std::fs::write(dir.join("a.txt"), b"12345").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_python_example(&runtime, &mut manager, &dir);
    wait_for_log(&mut manager, "ready");

    assert!(manager.dispatch_command("stats", Vec::new(), &dir, &[]));
    wait_for_log(&mut manager, "1 files, 0 folders, 5 bytes");
    assert!(!manager.dispatch_command("undeclared", Vec::new(), &dir, &[]));

    assert!(manager.dispatch_named(
        "dir-stats",
        vec!["touch".into(), "new.txt".into()],
        &dir,
        &[]
    ));
    wait_for_log(&mut manager, "created new.txt");
    assert!(dir.join("new.txt").exists());
    assert!(!manager.dispatch_named("no-such-plugin", Vec::new(), &dir, &[]));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn get_state_returns_what_the_host_last_published() {
    let dir = scratch_dir("get-state");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    manager
        .spawn(
            runtime.handle(),
            "fixture",
            FIXTURE,
            &[],
            Triggers {
                keys: vec![KeyCode::Char('b')],
                commands: Vec::new(),
                ..Triggers::default()
            },
            &dir,
        )
        .unwrap();
    manager.publish_state(StateSnapshot {
        cwd: dir.clone(),
        cursor: Some(dir.join("picked.txt")),
        ..StateSnapshot::default()
    });

    assert!(manager.dispatch_key(KeyCode::Char('b'), &dir, &[]));
    let seen = wait_for_log(&mut manager, "cursor: ");
    assert!(
        seen.iter()
            .any(|m| m == &format!("cursor: {}", dir.join("picked.txt").display())),
        "the plugin did not see the published cursor; saw {seen:?}"
    );

    manager.publish_state(StateSnapshot {
        cwd: dir.clone(),
        cursor: None,
        ..StateSnapshot::default()
    });
    assert!(manager.dispatch_key(KeyCode::Char('b'), &dir, &[]));
    wait_for_log(&mut manager, "cursor: none");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn before_anything_is_published_get_state_still_names_the_start_directory() {
    let dir = scratch_dir("get-state-early");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    manager
        .spawn(
            runtime.handle(),
            "fixture",
            FIXTURE,
            &[],
            Triggers {
                keys: vec![KeyCode::Char('b')],
                commands: Vec::new(),
                ..Triggers::default()
            },
            &dir,
        )
        .unwrap();

    assert!(manager.dispatch_key(KeyCode::Char('b'), &dir, &[]));
    wait_for_log(&mut manager, "cursor: none");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_python_plugin_reads_the_browser_state_with_get_state() {
    if !has_python() {
        return;
    }
    let dir = scratch_dir("python-state");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_python_example(&runtime, &mut manager, &dir);
    wait_for_log(&mut manager, "ready");
    manager.publish_state(StateSnapshot {
        cwd: dir.clone(),
        cursor: Some(dir.join("a.txt")),
        marks: vec![dir.join("a.txt"), dir.join("b.txt")],
        sort: "size".into(),
        view: "grid".into(),
        ..StateSnapshot::default()
    });

    assert!(manager.dispatch_named("dir-stats", vec!["state".into()], &dir, &[]));
    let seen = wait_for_log(&mut manager, "state: ");
    let line = seen.iter().find(|m| m.starts_with("state: ")).unwrap();
    assert!(
        line.contains("cursor=a.txt")
            && line.contains("marks=2")
            && line.contains("sort=size")
            && line.contains("view=grid"),
        "unexpected state line: {line}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

fn spawn_listener(
    runtime: &tokio::runtime::Runtime,
    manager: &mut PluginManager,
    dir: &std::path::Path,
    events: Vec<EventKind>,
) {
    manager
        .spawn(
            runtime.handle(),
            "fixture",
            FIXTURE,
            &[],
            Triggers {
                events,
                ..Triggers::default()
            },
            dir,
        )
        .unwrap();
}

fn state_at(cwd: &std::path::Path, cursor: Option<&str>, marks: &[&str]) -> StateSnapshot {
    StateSnapshot {
        cwd: cwd.to_path_buf(),
        cursor: cursor.map(|name| cwd.join(name)),
        marks: marks.iter().map(|name| cwd.join(name)).collect(),
        ..StateSnapshot::default()
    }
}

#[test]
fn a_subscribed_plugin_hears_the_cursor_and_directory_move() {
    let dir = scratch_dir("events-move");
    let elsewhere = dir.join("sub");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_listener(
        &runtime,
        &mut manager,
        &dir,
        vec![EventKind::DirChanged, EventKind::SelectionChanged],
    );
    wait_for_log(&mut manager, "ready");

    // The first snapshot is the browser arriving, not a change.
    manager.publish_state(state_at(&dir, Some("a"), &[]));
    manager.publish_state(state_at(&dir, Some("b"), &[]));
    wait_for_log(
        &mut manager,
        &format!(
            "selection_changed: {} -> {}",
            dir.join("a").display(),
            dir.join("b").display()
        ),
    );

    manager.publish_state(state_at(&elsewhere, Some("c"), &[]));
    wait_for_log(
        &mut manager,
        &format!("dir_changed: {} -> {}", dir.display(), elsewhere.display()),
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_first_published_state_fires_no_event() {
    let dir = scratch_dir("events-first");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_listener(
        &runtime,
        &mut manager,
        &dir,
        vec![EventKind::DirChanged, EventKind::SelectionChanged],
    );
    wait_for_log(&mut manager, "ready");

    manager.publish_state(state_at(&dir, Some("a"), &["a"]));
    // A later change is heard, and nothing from the first publish arrived before it.
    manager.publish_state(state_at(&dir, Some("b"), &["a"]));
    let seen = wait_for_log(&mut manager, "selection_changed");
    assert_eq!(
        seen.iter()
            .filter(|m| m.starts_with("selection_changed"))
            .count(),
        1,
        "saw {seen:?}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_plugin_that_did_not_subscribe_hears_nothing_and_a_change_of_marks_alone_is_heard() {
    let dir = scratch_dir("events-optin");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut quiet = PluginManager::new();
    spawn_listener(&runtime, &mut quiet, &dir, Vec::new());
    wait_for_log(&mut quiet, "ready");
    quiet.publish_state(state_at(&dir, Some("a"), &[]));
    quiet.publish_state(state_at(&dir, Some("b"), &["b"]));
    quiet.notify_job_finished("copy", JobOutcome::Completed);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        quiet.poll().is_empty(),
        "an unsubscribed plugin got an event"
    );

    let mut manager = PluginManager::new();
    spawn_listener(
        &runtime,
        &mut manager,
        &dir,
        vec![EventKind::SelectionChanged],
    );
    wait_for_log(&mut manager, "ready");
    manager.publish_state(state_at(&dir, Some("a"), &[]));
    manager.publish_state(state_at(&dir, Some("a"), &["a"]));
    wait_for_log(
        &mut manager,
        &format!(
            "selection_changed: {} -> {}",
            dir.join("a").display(),
            dir.join("a").display()
        ),
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_subscribed_plugin_hears_each_job_that_finishes() {
    let dir = scratch_dir("events-jobs");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_listener(&runtime, &mut manager, &dir, vec![EventKind::JobFinished]);
    wait_for_log(&mut manager, "ready");

    manager.notify_job_finished("copy", JobOutcome::Completed);
    manager.notify_job_finished("delete", JobOutcome::Failed);
    let mut seen = wait_for_log(&mut manager, "job_finished: delete failed");
    seen.extend(manager.poll().into_iter().map(|m| m.message));

    let jobs: Vec<_> = seen
        .iter()
        .filter(|m| m.starts_with("job_finished"))
        .collect();
    assert_eq!(
        jobs,
        [
            "job_finished: copy completed",
            "job_finished: delete failed"
        ]
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_plugin_that_never_reads_cannot_stall_the_host_however_much_changes() {
    let dir = scratch_dir("events-stuck");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    // `sleep` never reads its stdin, so once the pipe is full every write waits.
    manager
        .spawn(
            runtime.handle(),
            "stuck",
            "sleep",
            &["30".to_string()],
            Triggers {
                events: vec![EventKind::SelectionChanged],
                ..Triggers::default()
            },
            &dir,
        )
        .unwrap();
    let long_name = "x".repeat(2000);
    manager.publish_state(state_at(&dir, Some("first"), &[]));

    let started = Instant::now();
    for i in 0..20_000 {
        manager.publish_state(state_at(&dir, Some(&format!("{long_name}{i}")), &[]));
    }

    assert!(
        started.elapsed() < Duration::from_secs(5),
        "publishing blocked on a plugin that was not reading"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_python_plugin_reacts_to_directory_changes_and_finished_jobs() {
    if !has_python() {
        return;
    }
    let dir = scratch_dir("python-events");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    manager
        .spawn(
            runtime.handle(),
            "dir-stats",
            "python3",
            &[PYTHON_EXAMPLE.to_string()],
            Triggers {
                events: vec![EventKind::DirChanged, EventKind::JobFinished],
                ..Triggers::default()
            },
            &dir,
        )
        .unwrap();
    wait_for_log(&mut manager, "ready");

    manager.publish_state(state_at(&dir, None, &[]));
    manager.publish_state(state_at(&dir.join("photos"), None, &[]));
    wait_for_log(&mut manager, "entered photos");
    manager.notify_job_finished("move", JobOutcome::Cancelled);
    wait_for_log(&mut manager, "move cancelled");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// The pid a fixture logged when it started, from the first "ready pid=N" in `seen`.
fn pid_in(seen: &[String]) -> u32 {
    seen.iter()
        .find_map(|m| {
            m.strip_prefix("ready pid=")
                .and_then(|pid| pid.parse().ok())
        })
        .unwrap_or_else(|| panic!("no 'ready pid=' in {seen:?}"))
}

fn spawn_fixture_with(
    runtime: &tokio::runtime::Runtime,
    manager: &mut PluginManager,
    dir: &std::path::Path,
    flag: &str,
    triggers: Triggers,
) {
    manager
        .spawn(
            runtime.handle(),
            "fixture",
            FIXTURE,
            &[flag.to_string()],
            triggers,
            dir,
        )
        .unwrap();
}

fn on_key_b() -> Triggers {
    Triggers {
        keys: vec![KeyCode::Char('b')],
        ..Triggers::default()
    }
}

/// Polls until the only plugin is in a state `done` accepts, or panics with what it was.
fn wait_for_state(manager: &PluginManager, done: impl Fn(&PluginState) -> bool) -> PluginState {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = manager.summaries().remove(0).state;
        if done(&state) {
            return state;
        }
        assert!(Instant::now() < deadline, "plugin stayed {state:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn is_gone(pid: u32) -> bool {
    !std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .is_ok_and(|out| out.status.success())
}

#[test]
fn a_plugin_that_finished_is_started_again_by_its_next_key() {
    let dir = scratch_dir("respawn-key");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_with(
        &runtime,
        &mut manager,
        &dir,
        "--exit-after-key=0",
        on_key_b(),
    );
    let first = pid_in(&wait_for_log(&mut manager, "ready pid="));

    assert!(manager.dispatch_key(KeyCode::Char('b'), &dir, &[]));
    wait_for_log(&mut manager, "cursor: ");
    wait_for_state(&manager, |s| *s == PluginState::Finished);

    assert!(manager.dispatch_key(KeyCode::Char('b'), &dir, &[]));
    let again = wait_for_log(&mut manager, "cursor: ");
    assert_ne!(pid_in(&again), first, "the same process answered twice");
    assert_eq!(manager.summaries()[0].restarts, 1);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_plugin_that_crashes_straight_away_gets_three_tries_and_then_the_host_stops() {
    let dir = scratch_dir("respawn-loop");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_with(&runtime, &mut manager, &dir, "--exit-on-init=1", on_key_b());

    for _ in 0..3 {
        wait_for_state(&manager, |s| matches!(s, PluginState::Crashed(_)));
        assert!(manager.dispatch_key(KeyCode::Char('b'), &dir, &[]));
    }

    let seen = wait_for_log(&mut manager, "stopped after 3 crashes in a row");
    assert!(
        seen.iter()
            .any(|m| m.contains("`:plugins restart fixture`")),
        "the message should say how to start it again; saw {seen:?}"
    );
    let info = manager.summaries().remove(0);
    assert!(matches!(info.state, PluginState::Stopped(_)), "{info:?}");
    assert_eq!(
        info.restarts, 2,
        "three runs in all: the first and two more"
    );

    // A stopped plugin stays stopped, and each key press says so rather than doing nothing.
    assert!(manager.dispatch_key(KeyCode::Char('b'), &dir, &[]));
    wait_for_log(&mut manager, "stopped after 3 crashes in a row");
    assert_eq!(manager.summaries()[0].restarts, 2);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn restart_gives_a_stopped_plugin_another_go_and_clears_its_crashes() {
    let dir = scratch_dir("respawn-restart-stopped");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_with(&runtime, &mut manager, &dir, "--exit-on-init=1", on_key_b());
    for _ in 0..3 {
        wait_for_state(&manager, |s| matches!(s, PluginState::Crashed(_)));
        manager.dispatch_key(KeyCode::Char('b'), &dir, &[]);
    }
    wait_for_state(&manager, |s| matches!(s, PluginState::Stopped(_)));

    assert_eq!(manager.restart("fixture"), Restart::Restarted);

    // The new process crashes too, but the count began again, so the host is not stopped yet.
    let state = wait_for_state(&manager, |s| !matches!(s, PluginState::Running));
    assert!(matches!(state, PluginState::Crashed(_)), "{state:?}");
    assert_eq!(manager.restart("no-such-plugin"), Restart::NotFound);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn restart_stops_a_plugin_that_is_still_running_and_says_nothing_about_it() {
    let dir = scratch_dir("respawn-restart-running");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_listener(&runtime, &mut manager, &dir, Vec::new());
    let first = pid_in(&wait_for_log(&mut manager, "ready pid="));

    assert_eq!(manager.restart("fixture"), Restart::Restarted);

    let second = pid_in(&wait_for_log(&mut manager, "ready pid="));
    assert_ne!(first, second);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !is_gone(first) {
        assert!(Instant::now() < deadline, "the old process is still alive");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        manager
            .poll()
            .iter()
            .all(|m| !m.message.starts_with("exited")),
        "stopping a plugin on purpose is not a crash"
    );
    assert_eq!(manager.summaries()[0].state, PluginState::Running);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_plugin_that_only_listens_is_started_again_by_the_next_event() {
    let dir = scratch_dir("respawn-event");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_with(
        &runtime,
        &mut manager,
        &dir,
        "--exit-after-event=0",
        Triggers {
            events: vec![EventKind::DirChanged],
            ..Triggers::default()
        },
    );
    let first = pid_in(&wait_for_log(&mut manager, "ready pid="));
    let (a, b, c) = (dir.join("a"), dir.join("b"), dir.join("c"));
    manager.publish_state(state_at(&a, None, &[]));
    manager.publish_state(state_at(&b, None, &[]));
    wait_for_log(
        &mut manager,
        &format!("dir_changed: {} -> {}", a.display(), b.display()),
    );
    wait_for_state(&manager, |s| *s == PluginState::Finished);

    manager.publish_state(state_at(&c, None, &[]));

    let seen = wait_for_log(
        &mut manager,
        &format!("dir_changed: {} -> {}", b.display(), c.display()),
    );
    assert!(
        seen.iter().any(|m| m.starts_with("ready pid=")),
        "the event should have started it again first; saw {seen:?}"
    );
    assert_ne!(pid_in(&seen), first);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_plugin_with_a_key_is_not_started_again_by_an_event() {
    let dir = scratch_dir("respawn-event-keyed");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_with(
        &runtime,
        &mut manager,
        &dir,
        "--exit-after-event=0",
        Triggers {
            keys: vec![KeyCode::Char('b')],
            events: vec![EventKind::DirChanged],
            ..Triggers::default()
        },
    );
    wait_for_log(&mut manager, "ready pid=");
    manager.publish_state(state_at(&dir.join("a"), None, &[]));
    manager.publish_state(state_at(&dir.join("b"), None, &[]));
    wait_for_state(&manager, |s| *s == PluginState::Finished);

    manager.publish_state(state_at(&dir.join("c"), None, &[]));
    std::thread::sleep(Duration::from_millis(300));

    assert_eq!(manager.summaries()[0].restarts, 0);
    assert!(
        manager
            .poll()
            .iter()
            .all(|m| !m.message.starts_with("ready"))
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

fn spawn_fixture_args(
    runtime: &tokio::runtime::Runtime,
    manager: &mut PluginManager,
    name: &str,
    dir: &std::path::Path,
    args: &[&str],
    triggers: Triggers,
) {
    let args: Vec<String> = args.iter().map(ToString::to_string).collect();
    manager
        .spawn(runtime.handle(), name, FIXTURE, &args, triggers, dir)
        .unwrap();
}

#[test]
fn a_plugin_registers_a_command_while_running_and_the_command_then_reaches_it() {
    let dir = scratch_dir("register-run");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "fixture",
        &dir,
        &["--register=blame"],
        Triggers::default(),
    );
    assert!(
        !manager.dispatch_command("blame", Vec::new(), &dir, &[]),
        "nothing owns the word before the plugin asks"
    );
    wait_for_log(&mut manager, "register blame: ok");

    assert!(manager.dispatch_command("blame", vec!["a.rs".into(), "b".into()], &dir, &[]));
    wait_for_log(&mut manager, "command: blame a.rs b");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_built_in_word_and_a_word_another_plugin_holds_are_refused_with_a_reason() {
    let dir = scratch_dir("register-refused");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    manager.reserve_commands(["cd".to_string()]);
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "first",
        &dir,
        &["--register=shared"],
        Triggers::default(),
    );
    wait_for_log(&mut manager, "register shared: ok");
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "second",
        &dir,
        &["--register=cd", "--register=shared", "--register=two words"],
        Triggers::default(),
    );

    let seen = wait_for_log(&mut manager, "register two words:");
    let said = |word: &str| {
        seen.iter()
            .find(|m| m.starts_with(&format!("register {word}:")))
            .unwrap_or_else(|| panic!("no answer for {word} in {seen:?}"))
            .clone()
    };
    assert!(said("cd").contains("built-in command"), "{}", said("cd"));
    assert!(
        said("shared").contains("belongs to plugin 'first'"),
        "{}",
        said("shared")
    );
    assert!(
        said("two words").contains("letters, digits"),
        "{}",
        said("two words")
    );

    // The word stays with the plugin that had it.
    assert!(manager.dispatch_command("shared", Vec::new(), &dir, &[]));
    wait_for_log(&mut manager, "command: shared");
    assert!(!manager.dispatch_command("cd", Vec::new(), &dir, &[]));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_config_word_and_a_registered_word_answer_alike_and_config_words_cannot_be_given_up() {
    let dir = scratch_dir("register-config");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "fixture",
        &dir,
        &["--register=live", "--unregister-on-key=fixed"],
        Triggers {
            keys: vec![KeyCode::Char('b')],
            commands: vec!["fixed".to_string()],
            ..Triggers::default()
        },
    );
    wait_for_log(&mut manager, "register live: ok");

    assert!(manager.dispatch_command("fixed", Vec::new(), &dir, &[]));
    wait_for_log(&mut manager, "command: fixed");
    assert!(manager.dispatch_command("live", Vec::new(), &dir, &[]));
    wait_for_log(&mut manager, "command: live");

    assert!(manager.dispatch_key(KeyCode::Char('b'), &dir, &[]));
    let seen = wait_for_log(&mut manager, "unregister fixed:");
    assert!(
        seen.iter()
            .any(|m| m.contains("declared in the config file")),
        "{seen:?}"
    );
    assert!(manager.dispatch_command("fixed", Vec::new(), &dir, &[]));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_unregistered_word_stops_reaching_the_plugin() {
    let dir = scratch_dir("register-unregister");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "fixture",
        &dir,
        &["--register=live", "--unregister-on-key=live"],
        on_key_b(),
    );
    wait_for_log(&mut manager, "register live: ok");
    assert!(manager.dispatch_command("live", Vec::new(), &dir, &[]));
    wait_for_log(&mut manager, "command: live");

    assert!(manager.dispatch_key(KeyCode::Char('b'), &dir, &[]));
    wait_for_log(&mut manager, "unregister live: ok");

    assert!(!manager.dispatch_command("live", Vec::new(), &dir, &[]));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_registered_command_still_starts_a_plugin_that_has_exited() {
    let dir = scratch_dir("register-respawn");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "fixture",
        &dir,
        &["--register=blame", "--exit-after-key=0"],
        on_key_b(),
    );
    wait_for_log(&mut manager, "register blame: ok");
    assert!(manager.dispatch_key(KeyCode::Char('b'), &dir, &[]));
    wait_for_state(&manager, |s| *s == PluginState::Finished);

    assert!(
        manager.dispatch_command("blame", vec!["x".into()], &dir, &[]),
        "the word must outlive the process that registered it"
    );

    // The fresh process registers again at `init` (the same word, so it is accepted) and the
    // command that started it is delivered after.
    let seen = wait_for_log(&mut manager, "command: blame x");
    assert!(
        seen.iter().any(|m| m == "register blame: ok"),
        "the fresh process should have registered first; saw {seen:?}"
    );
    assert_eq!(manager.summaries()[0].restarts, 1);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn restart_forgets_what_the_plugin_registered_while_running_and_keeps_its_config_words() {
    let dir = scratch_dir("register-restart");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    // The plugin registers `live` at `init` and the config declares `fixed`; a restart wipes
    // what was registered while running, and the fresh process registers it again.
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "fixture",
        &dir,
        &["--register=live"],
        Triggers {
            commands: vec!["fixed".to_string()],
            ..Triggers::default()
        },
    );
    wait_for_log(&mut manager, "register live: ok");
    assert!(manager.dispatch_command("live", Vec::new(), &dir, &[]));
    wait_for_log(&mut manager, "command: live");

    assert_eq!(manager.restart("fixture"), Restart::Restarted);

    // Config words are never gone; the runtime one is back once the new process asks for it.
    assert!(manager.dispatch_command("fixed", Vec::new(), &dir, &[]));
    wait_for_log(&mut manager, "register live: ok");
    assert!(manager.dispatch_command("live", Vec::new(), &dir, &[]));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_python_plugin_registers_its_own_command_with_no_config() {
    if !has_python() {
        return;
    }
    let dir = scratch_dir("python-register");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    manager
        .spawn(
            runtime.handle(),
            "dir-stats",
            "python3",
            &[PYTHON_EXAMPLE.to_string()],
            Triggers::default(),
            &dir,
        )
        .unwrap();
    wait_for_log(&mut manager, "registered :state");
    manager.publish_state(state_at(&dir, Some("a.txt"), &[]));

    assert!(manager.dispatch_command("state", Vec::new(), &dir, &[]));
    wait_for_log(&mut manager, "state: cursor=a.txt");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// Waits for the plugin's next `run_action` to reach the render thread's side.
#[track_caller]
fn wait_for_action(manager: &mut PluginManager) -> plugin_host::ActionRequest {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(request) = manager.take_action() {
            return request;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no run_action arrived");
}

#[test]
fn a_run_action_waits_for_the_interface_and_hears_whether_it_ran() {
    let dir = scratch_dir("run-action");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "fixture",
        &dir,
        &["--run-action=paste", "--run-action=sort"],
        Triggers::default(),
    );

    let paste = wait_for_action(&mut manager);
    assert_eq!((paste.plugin(), paste.name()), ("fixture", "paste"));
    // The plugin is blocked on the reply, so its second request cannot have been sent yet.
    std::thread::sleep(Duration::from_millis(100));
    assert!(manager.take_action().is_none());
    paste.ran();
    wait_for_log(&mut manager, "run paste: ok");

    wait_for_action(&mut manager).refuse("a prompt is open");
    wait_for_log(
        &mut manager,
        "run sort: cannot run 'sort': a prompt is open",
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn list_actions_answers_with_the_declared_names_without_the_interface() {
    let dir = scratch_dir("list-actions");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    manager.declare_actions(["paste".to_string(), "sort".to_string()]);
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "fixture",
        &dir,
        &["--list-actions"],
        Triggers::default(),
    );

    wait_for_log(&mut manager, "actions: paste,sort");
    assert!(manager.take_action().is_none());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_run_action_the_interface_drops_is_answered_with_an_error() {
    let dir = scratch_dir("run-action-dropped");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "fixture",
        &dir,
        &["--run-action=quit"],
        Triggers::default(),
    );

    drop(wait_for_action(&mut manager));
    wait_for_log(
        &mut manager,
        "run quit: the interface closed before running it",
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_python_plugin_runs_an_action_by_name_and_lists_them() {
    if !has_python() {
        return;
    }
    let dir = scratch_dir("python-actions");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    manager.declare_actions(["paste".to_string(), "sort".to_string()]);
    spawn_python_example(&runtime, &mut manager, &dir);
    wait_for_log(&mut manager, "ready");

    assert!(manager.dispatch_named("dir-stats", vec!["actions".into()], &dir, &[]));
    wait_for_log(&mut manager, "actions: paste sort");

    assert!(manager.dispatch_named("dir-stats", vec!["run".into(), "sort".into()], &dir, &[]));
    let request = wait_for_action(&mut manager);
    assert_eq!((request.plugin(), request.name()), ("dir-stats", "sort"));
    request.ran();
    wait_for_log(&mut manager, "ran sort");

    assert!(manager.dispatch_named("dir-stats", vec!["run".into(), "paste".into()], &dir, &[]));
    wait_for_action(&mut manager).refuse("a job is running");
    wait_for_log(&mut manager, "cannot run 'paste': a job is running");

    std::fs::remove_dir_all(&dir).unwrap();
}

fn provides_find() -> Triggers {
    Triggers {
        actions: vec!["find".to_string()],
        ..Triggers::default()
    }
}

/// The next picker request waiting for the render thread.
fn wait_for_ui(manager: &mut PluginManager) -> UiRequest {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(request) = manager.take_ui() {
            return request;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("nothing arrived for the interface");
}

#[test]
fn an_action_reaches_the_plugin_that_provides_it_and_no_other() {
    let dir = scratch_dir("action-dispatch");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    assert_eq!(
        manager.dispatch_action("find", &dir, &[]),
        ActionDispatch::NoProvider,
        "with no plugins, the interface handles it"
    );
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "fixture",
        &dir,
        &[],
        provides_find(),
    );

    assert_eq!(
        manager.dispatch_action("sort", &dir, &[]),
        ActionDispatch::NoProvider
    );
    let mut state = StateSnapshot::default();
    state.cwd.clone_from(&dir);
    state.show_hidden = true;
    manager.publish_state(state);
    assert_eq!(
        manager.dispatch_action("find", &dir, &[]),
        ActionDispatch::Sent
    );
    wait_for_log(&mut manager, "action: find hidden=true");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_provider_that_cannot_run_is_reported_not_silently_skipped() {
    let dir = scratch_dir("action-unavailable");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "fixture",
        &dir,
        &["--exit-on-init=1"],
        provides_find(),
    );
    // It crashes straight away every time, until the host stops starting it.
    let mut last = ActionDispatch::Sent;
    for _ in 0..6 {
        std::thread::sleep(Duration::from_millis(150));
        last = manager.dispatch_action("find", &dir, &[]);
    }
    assert_eq!(last, ActionDispatch::Unavailable);
    let said = manager.poll();
    assert!(
        said.iter().any(|m| m.message.contains("restart fixture")),
        "the user is told how to start it again: {said:?}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_plugin_opens_a_picker_through_the_interface_and_hears_what_is_typed() {
    let dir = scratch_dir("picker");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "fixture",
        &dir,
        &["--picker-on-action"],
        provides_find(),
    );
    assert_eq!(
        manager.dispatch_action("find", &dir, &[]),
        ActionDispatch::Sent
    );

    let UiRequest::ShowPicker(request) = wait_for_ui(&mut manager) else {
        panic!("the first thing is the request to open the picker");
    };
    assert_eq!(request.plugin(), "fixture");
    assert_eq!(request.spec().picker, 1);
    assert_eq!(request.spec().title, "fixture");
    assert!(!request.spec().preview);
    request.opened();
    wait_for_log(&mut manager, "picker: ok");

    let UiRequest::Items { plugin, update } = wait_for_ui(&mut manager) else {
        panic!("then what the picker lists");
    };
    assert_eq!(plugin, "fixture");
    assert_eq!(
        (update.picker, update.matches, update.items.len()),
        (1, 1, 1)
    );
    assert_eq!(update.items[0].rel, "a.txt");

    manager.send_picker_query("fixture", 1, 4, "ma");
    wait_for_log(&mut manager, "picker_query: 1 4 ma");
    manager.send_picker_closed("fixture", 1);
    wait_for_log(&mut manager, "picker_closed: 1");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_refused_picker_tells_the_plugin_why() {
    let dir = scratch_dir("picker-refused");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "fixture",
        &dir,
        &["--picker-on-action"],
        provides_find(),
    );
    manager.dispatch_action("find", &dir, &[]);
    let UiRequest::ShowPicker(request) = wait_for_ui(&mut manager) else {
        panic!("a picker was asked for");
    };
    request.refuse("another picker is open");
    wait_for_log(
        &mut manager,
        "picker: cannot open the picker: another picker is open",
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_walk_comes_back_in_batches_through_the_hosts_own_paths_and_ends() {
    let dir = scratch_dir("walk");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.txt"), b"").unwrap();
    std::fs::write(dir.join("sub").join("b.txt"), b"").unwrap();
    std::fs::write(dir.join(".hidden"), b"").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "fixture",
        &dir,
        &["--picker-on-action"],
        provides_find(),
    );
    manager.dispatch_action("find", &dir, &[]);
    let UiRequest::ShowPicker(request) = wait_for_ui(&mut manager) else {
        panic!("a picker was asked for");
    };
    request.opened();

    // Two folders are listed, so two batches: the root's two visible entries, then `sub`'s one.
    let seen = wait_for_log(&mut manager, "walk_done: 7 done truncated=false");
    assert!(seen.contains(&"walk: ok".to_string()), "{seen:?}");
    assert!(seen.contains(&"walk_batch: 7 2".to_string()), "{seen:?}");
    assert!(seen.contains(&"walk_batch: 7 1".to_string()), "{seen:?}");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_stopped_plugin_is_no_longer_running() {
    let dir = scratch_dir("running");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    spawn_fixture_args(
        &runtime,
        &mut manager,
        "fixture",
        &dir,
        &[],
        provides_find(),
    );
    wait_for_log(&mut manager, "ready");
    assert!(manager.is_running("fixture"));
    assert!(!manager.is_running("nobody"));
    manager.restart("fixture");
    drop(manager);

    std::fs::remove_dir_all(&dir).unwrap();
}

fn serves_archives() -> Triggers {
    Triggers {
        actions: vec![plugin_api::PROVIDES_VFS_ARCHIVE.to_string()],
        ..Triggers::default()
    }
}

fn list_request(path: &str) -> plugin_api::VfsRequest {
    plugin_api::VfsRequest::List { path: path.into() }
}

#[test]
fn a_request_for_the_archive_paths_is_answered_by_the_plugin_that_provides_them() {
    use plugin_api::{Epoch, Peer, PeerError, VfsFault};

    let dir = scratch_dir("serve-answer");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    let link = manager.serve_link();
    assert_eq!(
        link.call(&list_request("/x/a.zip!"), None),
        Err(PeerError::Unavailable),
        "nothing answers before a plugin is started"
    );

    spawn_fixture_with(
        &runtime,
        &mut manager,
        &dir,
        "--serve-vfs",
        serves_archives(),
    );
    wait_for_log(&mut manager, "ready pid=");
    assert!(manager.serves_archives());

    let answer = link.call(&list_request("/x/a.zip!"), None).unwrap();
    assert_eq!(answer.epoch, Epoch(1));
    assert_eq!(answer.value["op"], "list");

    assert_eq!(
        link.call(&list_request("/x/missing.zip!"), None),
        Err(PeerError::Fault(VfsFault::NotFound)),
        "the plugin's reason comes back as it was given"
    );
}

#[test]
fn a_plugin_that_does_not_provide_the_archive_paths_is_not_asked_for_them() {
    use plugin_api::{Peer, PeerError};

    let dir = scratch_dir("serve-not-provided");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    let link = manager.serve_link();
    spawn_fixture_with(
        &runtime,
        &mut manager,
        &dir,
        "--serve-vfs",
        Triggers::default(),
    );
    wait_for_log(&mut manager, "ready pid=");
    assert_eq!(
        link.call(&list_request("/x/a.zip!"), None),
        Err(PeerError::Unavailable)
    );
}

#[test]
fn a_plugin_that_dies_in_the_middle_of_a_request_ends_the_call_and_comes_back() {
    use plugin_api::{Epoch, Peer, PeerError, VfsRequest};

    let dir = scratch_dir("serve-dies");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    let link = manager.serve_link();
    spawn_fixture_with(
        &runtime,
        &mut manager,
        &dir,
        "--serve-vfs",
        serves_archives(),
    );
    wait_for_log(&mut manager, "ready pid=");
    let first = link.call(&list_request("/x/a.zip!"), None).unwrap();

    assert_eq!(
        link.call(&list_request("/x/die.zip!"), None),
        Err(PeerError::Stopped),
        "the call ends with the process instead of waiting out its timeout"
    );

    // The next tick of the interface starts it again, with no key or command to wake it.
    let deadline = Instant::now() + Duration::from_secs(5);
    let second = loop {
        manager.poll();
        if let Ok(answer) = link.call(&list_request("/x/a.zip!"), None) {
            break answer;
        }
        assert!(
            Instant::now() < deadline,
            "the plugin was not started again"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_ne!(
        second.value["pid"], first.value["pid"],
        "a new process answered"
    );
    assert_eq!(second.epoch, Epoch(first.epoch.0 + 1));

    // A file opened in the first life must not be asked of the second.
    let read = VfsRequest::Read {
        handle: 1,
        offset: 0,
        len: 1,
    };
    assert_eq!(link.call(&read, Some(first.epoch)), Err(PeerError::Stale));
}
