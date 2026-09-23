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
use plugins::PluginManager;

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
            Some(KeyCode::Char('b')),
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
            Some(KeyCode::Char('b')),
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
        None,
        &dir,
    );

    assert!(result.is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
