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

//! What a plugin costs, measured against the real fixture and a real `python3`. These are
//! benchmarks, not tests of behaviour, so they are `#[ignore]`d and run by hand:
//!
//! `cargo test --release -p plugin_host --test plugin_cost -- --ignored --nocapture --test-threads=1`
//!
//! There is no stored baseline: a run prints its numbers, and the ones worth keeping are written
//! into `DIARY.md`. Numbers from a debug build mean little, so run it with `--release`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use plugin_host::{PluginManager, Triggers};

const FIXTURE: &str = env!("CARGO_BIN_EXE_test-fixture-plugin");
const PYTHON_EXAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/plugins/dir_stats.py"
);

/// Spawns per measurement. A median of this many drops the odd slow start on a busy machine.
const SPAWNS: usize = 25;
/// Requests per measurement of a round trip.
const CALLS: usize = 2000;

fn scratch_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("minuteman-plugin-cost-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the benchmark's scratch directory");
    dir
}

/// Polls without sleeping until a log containing `needle` arrives, so the wait adds nothing a
/// plugin's own latency would not. `answer` runs between polls (for `run_action`, which the render
/// thread has to answer).
fn spin_until_log(
    manager: &mut PluginManager,
    needle: &str,
    mut answer: impl FnMut(&mut PluginManager),
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        answer(manager);
        if manager.poll().iter().any(|m| m.message.contains(needle)) {
            return;
        }
        std::hint::spin_loop();
    }
    panic!("no log containing {needle:?} within 30 s");
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

/// From the call to `spawn` until the plugin has logged its first line, which both fixtures do
/// only once `init` has been read.
fn time_to_first_log(command: &str, args: &[String], ready: &str) -> Duration {
    let dir = scratch_dir();
    let runtime = tokio::runtime::Runtime::new().expect("start a runtime");
    let samples = (0..SPAWNS)
        .map(|_| {
            let mut manager = PluginManager::new();
            let start = Instant::now();
            manager
                .spawn(
                    runtime.handle(),
                    "bench",
                    command,
                    args,
                    Triggers::default(),
                    &dir,
                )
                .expect("spawn the plugin");
            spin_until_log(&mut manager, ready, |_| {});
            start.elapsed()
        })
        .collect();
    median(samples)
}

/// The mean time of one `method` request, answered by the host, over `CALLS` back-to-back calls.
fn round_trip(method: &str) -> Duration {
    let dir = scratch_dir();
    let runtime = tokio::runtime::Runtime::new().expect("start a runtime");
    let mut manager = PluginManager::new();
    manager.declare_actions(["paste".to_string()]);
    manager
        .spawn(
            runtime.handle(),
            "bench",
            FIXTURE,
            &[format!("--bench={method}:{CALLS}")],
            Triggers::default(),
            &dir,
        )
        .expect("spawn the fixture");
    spin_until_log(&mut manager, "ready", |_| {});
    let start = Instant::now();
    spin_until_log(&mut manager, &format!("bench {method} done"), |manager| {
        if let Some(request) = manager.take_action() {
            request.ran();
        }
    });
    start.elapsed() / CALLS as u32
}

fn has_python() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok()
}

#[test]
#[ignore = "benchmark: run by hand with --release, see the module note"]
fn spawn_to_first_log() {
    let fixture = time_to_first_log(FIXTURE, &[], "ready");
    println!("spawn to first log, Rust fixture: {fixture:?} (median of {SPAWNS})");
    if has_python() {
        let python = time_to_first_log("python3", &[PYTHON_EXAMPLE.to_string()], "ready");
        println!("spawn to first log, python3 example: {python:?} (median of {SPAWNS})");
    } else {
        println!("python3 is not installed; skipped");
    }
}

#[test]
#[ignore = "benchmark: run by hand with --release, see the module note"]
fn request_round_trips() {
    for method in ["list_actions", "get_state", "run_action"] {
        println!(
            "{method} round trip: {:?} (mean of {CALLS})",
            round_trip(method)
        );
    }
}

/// A user with no plugin configured must pay nothing. The tick calls these on an empty manager
/// every 100 ms, so this shows they are a few instructions, and that no thread or process exists.
#[test]
#[ignore = "benchmark: run by hand with --release, see the module note"]
fn an_empty_manager_costs_nothing_per_tick() {
    let mut manager = PluginManager::new();
    assert!(manager.is_empty());
    let calls = 1_000_000u32;
    let start = Instant::now();
    for _ in 0..calls {
        std::hint::black_box(manager.poll());
        std::hint::black_box(manager.take_action());
        std::hint::black_box(manager.is_empty());
    }
    println!(
        "empty manager, poll + take_action + is_empty: {:?} per tick",
        start.elapsed() / calls
    );
}
