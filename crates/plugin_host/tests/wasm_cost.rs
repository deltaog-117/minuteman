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

//! What a sandboxed plugin costs: the time from `spawn` to its first log line, and the mean time
//! of one request it makes and the host answers. Benchmarks, not tests of behaviour, so they are
//! `#[ignore]`d and run by hand, release build only (numbers from a debug build mean little):
//!
//! `cargo test --release -p plugin_host --features wasm --test wasm_cost -- --ignored --nocapture --test-threads=1`
//!
//! As with `plugin_cost`, no baseline is stored: the numbers worth keeping go into `DIARY.md`.
#![cfg(feature = "wasm")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use plugin_host::{PluginManager, Triggers};

const SPAWNS: usize = 25;
const CALLS: u32 = 2000;

const READY: &str =
    r#"{"jsonrpc":"2.0","method":"log","params":{"level":"info","message":"ready"}}"#;
const DONE: &str = r#"{"jsonrpc":"2.0","method":"log","params":{"level":"info","message":"done"}}"#;
const STATE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"get_state","params":{}}"#;

/// Logs "ready", then asks for the state `CALLS` times (reading once between asks, so it is at
/// most one answer behind), then logs "done".
fn module() -> String {
    let lines = [READY, STATE, DONE].map(|line| format!("{line}\n"));
    let mut at = 1024;
    let spans: Vec<(usize, usize)> = lines
        .iter()
        .map(|line| {
            let span = (at, line.len());
            at += line.len();
            span
        })
        .collect();
    let escaped: String = lines
        .concat()
        .bytes()
        .map(|byte| format!("\\{byte:02x}"))
        .collect();
    let write = |(ptr, len): (usize, usize)| {
        format!(
            "(i32.store (i32.const 0) (i32.const {ptr})) (i32.store (i32.const 4) (i32.const {len}))\n\
             (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 8)))\n"
        )
    };
    let (ready, state, done) = (write(spans[0]), write(spans[1]), write(spans[2]));
    format!(
        r#"(module
            (import "wasi_snapshot_preview1" "fd_read" (func $fd_read (param i32 i32 i32 i32) (result i32)))
            (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 1024) "{escaped}")
            (func (export "_start") (local $n i32)
                (i32.store (i32.const 16) (i32.const 4096))
                (i32.store (i32.const 20) (i32.const 4096))
                {ready}
                (local.set $n (i32.const {CALLS}))
                (loop $ask
                    {state}
                    (drop (call $fd_read (i32.const 0) (i32.const 16) (i32.const 1) (i32.const 24)))
                    (local.set $n (i32.sub (local.get $n) (i32.const 1)))
                    (br_if $ask (local.get $n)))
                {done}))"#
    )
}

fn scratch() -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("mman-wasm-cost-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("make the scratch folder");
    let path = dir.join("bench.wasm");
    std::fs::write(&path, wat::parse_str(module()).expect("valid WAT")).expect("write the module");
    (dir, path)
}

fn spin_until_log(manager: &mut PluginManager, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if manager.poll().iter().any(|m| m.message == needle) {
            return;
        }
        std::hint::spin_loop();
    }
    panic!("no log {needle:?} within 30 s");
}

#[test]
#[ignore = "benchmark: run by hand in a release build"]
fn start_and_round_trip() {
    let (dir, path) = scratch();
    let runtime = tokio::runtime::Runtime::new().expect("start a runtime");

    let mut starts: Vec<Duration> = (0..SPAWNS)
        .map(|_| {
            let mut manager = PluginManager::new();
            let began = Instant::now();
            manager
                .spawn(
                    runtime.handle(),
                    "bench",
                    path.to_str().unwrap(),
                    &[],
                    Triggers::default(),
                    &dir,
                )
                .expect("spawn the module");
            spin_until_log(&mut manager, "ready");
            began.elapsed()
        })
        .collect();
    starts.sort_unstable();
    println!(
        "wasm start to first log, median of {SPAWNS}: {:?}",
        starts[SPAWNS / 2]
    );

    let mut manager = PluginManager::new();
    manager
        .spawn(
            runtime.handle(),
            "bench",
            path.to_str().unwrap(),
            &[],
            Triggers::default(),
            &dir,
        )
        .expect("spawn the module");
    spin_until_log(&mut manager, "ready");
    let began = Instant::now();
    spin_until_log(&mut manager, "done");
    println!(
        "wasm get_state round trip, mean of {CALLS}: {:?}",
        began.elapsed() / CALLS
    );
}
