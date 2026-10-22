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

//! The Rust example module (`examples/plugins/wasm_stats`) compiled for real and run in the real
//! sandbox, so the example cannot drift from the protocol. It needs the `wasm32-wasip1` rust
//! target, which a machine without it lacks: the test then says so and passes, because a missing
//! target is a fact about the machine, not about the code. Built only with the `wasm` feature.
#![cfg(feature = "wasm")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use plugin_host::{LogMessage, PluginManager, Triggers};

const TARGET: &str = "wasm32-wasip1";

fn has_target() -> bool {
    let Ok(out) = Command::new("rustc")
        .args(["--print", "target-libdir", "--target", TARGET])
        .output()
    else {
        return false;
    };
    let libdir = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    out.status.success()
        && std::fs::read_dir(libdir).is_ok_and(|mut files| {
            files.any(|file| {
                file.is_ok_and(|file| file.file_name().to_string_lossy().starts_with("libstd-"))
            })
        })
}

/// Builds the example into `out` and returns the module.
fn build_example(out: &Path) -> PathBuf {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/wasm_stats");
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "--release", "--target", TARGET])
        .current_dir(&crate_dir)
        .env("CARGO_TARGET_DIR", out)
        .status()
        .expect("cargo runs");
    assert!(status.success(), "the example module failed to build");
    out.join(TARGET).join("release").join("wasm_stats.wasm")
}

fn wait_for_log(
    manager: &mut PluginManager,
    wanted: impl Fn(&LogMessage) -> bool,
) -> Result<Vec<LogMessage>, Vec<LogMessage>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        seen.extend(manager.poll());
        if seen.iter().any(&wanted) {
            return Ok(seen);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(seen)
}

#[test]
fn the_rust_example_counts_the_browsed_folder_and_is_refused_when_it_tries_to_write() {
    if !has_target() {
        eprintln!(
            "skipped: the {TARGET} rust target is not installed (rustup target add {TARGET})"
        );
        return;
    }
    let dir = std::env::temp_dir().join(format!("mman-wasm-example-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let browsed = dir.join("browsed");
    std::fs::create_dir_all(browsed.join("sub")).unwrap();
    std::fs::write(browsed.join("a.txt"), b"12345").unwrap();
    std::fs::write(browsed.join("b.txt"), b"123").unwrap();
    let module = build_example(&dir.join("build"));

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    manager
        .spawn(
            runtime.handle(),
            "wasm-stats",
            module.to_str().unwrap(),
            &[],
            Triggers {
                commands: vec!["wstats".into(), "wtouch".into()],
                ..Triggers::default()
            },
            &browsed,
        )
        .expect("the example module starts");
    wait_for_log(&mut manager, |m| m.message == "wasm-stats ready")
        .unwrap_or_else(|seen| panic!("no init reply (the API handshake?); saw {seen:?}"));

    assert!(manager.dispatch_command("wstats", Vec::new(), &browsed, &[]));
    wait_for_log(&mut manager, |m| m.message == "2 files, 1 folders, 8 bytes")
        .unwrap_or_else(|seen| panic!("no summary; saw {seen:?}"));

    assert!(manager.dispatch_command("wtouch", vec!["new.txt".into()], &browsed, &[]));
    wait_for_log(&mut manager, |m| m.message.contains("the sandbox said no"))
        .unwrap_or_else(|seen| panic!("the refusal was not reported; saw {seen:?}"));
    assert!(!browsed.join("new.txt").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
