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

//! The sandbox end to end: `PluginManager` starts real `.wasm` modules (written as WAT here, so no
//! wasm target is needed) and what they ask of the host is judged by the real read loop. Built
//! only with the `wasm` feature; `wasm_unavailable.rs` covers the build without it.
#![cfg(feature = "wasm")]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use plugin_host::{LogMessage, PluginManager, PluginState, Triggers};

fn scratch_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mman-wasm-it-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A module that writes each of `lines` (plus a newline) to its output, then reads its input until
/// it ends, so it stays alive to be answered.
fn emitter(lines: &[&str]) -> String {
    let mut data = Vec::new();
    let mut writes = String::new();
    for line in lines {
        let bytes = format!("{line}\n");
        writes += &format!(
            "(i32.store (i32.const 0) (i32.const {})) (i32.store (i32.const 4) (i32.const {}))\n\
             (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 8)))\n",
            1024 + data.len(),
            bytes.len()
        );
        data.extend(bytes.bytes());
    }
    let escaped: String = data.iter().map(|byte| format!("\\{byte:02x}")).collect();
    format!(
        r#"(module
            (import "wasi_snapshot_preview1" "fd_read" (func $fd_read (param i32 i32 i32 i32) (result i32)))
            (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 1024) "{escaped}")
            (func (export "_start")
                {writes}
                (i32.store (i32.const 16) (i32.const 2048))
                (i32.store (i32.const 20) (i32.const 256))
                (loop $again
                    (drop (call $fd_read (i32.const 0) (i32.const 16) (i32.const 1) (i32.const 24)))
                    (br_if $again (i32.load (i32.const 24))))))"#
    )
}

fn module(dir: &Path, name: &str, wat: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, wat::parse_str(wat).expect("valid WAT")).unwrap();
    path
}

fn request(id: u32, method: &str, params: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#)
}

/// Polls the manager until a log line satisfies `wanted`, or five seconds pass.
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

fn start(
    runtime: &tokio::runtime::Runtime,
    manager: &mut PluginManager,
    dir: &Path,
    module: &Path,
    args: &[String],
) {
    manager
        .spawn(
            runtime.handle(),
            "wasmer",
            module.to_str().unwrap(),
            args,
            Triggers::default(),
            dir,
        )
        .expect("the module starts");
}

#[test]
fn a_sandboxed_plugin_cannot_delete_a_file_even_inside_the_browsed_folder() {
    let dir = scratch_dir("delete");
    let victim = dir.join("victim.txt");
    std::fs::write(&victim, b"keep me").unwrap();
    let delete = request(
        1,
        "delete",
        &format!(r#"{{"path":{:?}}}"#, victim.to_str().unwrap()),
    );
    let wasm = module(&dir, "p.wasm", &emitter(&[&delete]));

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    start(&runtime, &mut manager, &dir, &wasm, &[]);

    let logs = wait_for_log(&mut manager, |m| m.message.contains("request refused"))
        .unwrap_or_else(|seen| panic!("no refusal was logged; saw {seen:?}"));
    assert!(
        logs.iter().any(|m| m.message.contains("delete files")),
        "{logs:?}"
    );
    assert_eq!(std::fs::read(&victim).unwrap(), b"keep me");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn reading_outside_the_browsed_folder_is_refused_while_a_command_can_still_be_registered() {
    let dir = scratch_dir("outside");
    let register = request(1, "register_command", r#"{"name":"wasmx"}"#);
    let read_etc = request(2, "read_dir", r#"{"path":"/etc"}"#);
    let wasm = module(&dir, "p.wasm", &emitter(&[&register, &read_etc]));

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    start(&runtime, &mut manager, &dir, &wasm, &[]);

    let logs = wait_for_log(&mut manager, |m| m.message.contains("/etc"))
        .unwrap_or_else(|seen| panic!("no refusal of /etc was logged; saw {seen:?}"));
    assert!(
        logs.iter()
            .any(|m| m.message.contains("outside the browsed folder")),
        "{logs:?}"
    );
    // The registration came first on the same line stream, so it has been handled by now.
    assert!(manager.dispatch_command("wasmx", Vec::new(), &dir, &[]));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_symlink_in_the_browsed_folder_does_not_open_the_way_out() {
    let dir = scratch_dir("link");
    let browsed = dir.join("browsed");
    let outside = dir.join("outside");
    std::fs::create_dir_all(&browsed).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret"), b"s").unwrap();
    std::os::unix::fs::symlink(&outside, browsed.join("link")).unwrap();
    let through = browsed.join("link").join("secret");
    let read = request(
        1,
        "read_file",
        &format!(r#"{{"path":{:?},"len":10}}"#, through.to_str().unwrap()),
    );
    let wasm = module(&dir, "p.wasm", &emitter(&[&read]));

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    start(&runtime, &mut manager, &browsed, &wasm, &[]);

    assert!(
        wait_for_log(&mut manager, |m| m
            .message
            .contains("outside the browsed folder"))
        .is_ok(),
        "the read through the link was not refused"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_module_that_hogs_the_cpu_is_ended_and_the_reason_is_reported() {
    let dir = scratch_dir("hog");
    let wasm = module(
        &dir,
        "p.wasm",
        r#"(module (memory (export "memory") 1) (func (export "_start") (loop $l (br $l))))"#,
    );

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    start(&runtime, &mut manager, &dir, &wasm, &[]);

    let logs = wait_for_log(&mut manager, |m| {
        m.message.contains("without reading its input")
    })
    .unwrap_or_else(|seen| panic!("the hog was never ended; saw {seen:?}"));
    assert!(logs.iter().any(|m| m.plugin == "wasmer"));
    let summary = manager
        .summaries()
        .into_iter()
        .find(|p| p.name == "wasmer")
        .unwrap();
    assert!(
        matches!(summary.state, PluginState::Crashed(_)),
        "{:?}",
        summary.state
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn arguments_given_to_a_module_are_ignored_with_a_warning() {
    let dir = scratch_dir("args");
    let wasm = module(&dir, "p.wasm", &emitter(&[]));

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    start(&runtime, &mut manager, &dir, &wasm, &["--fast".to_string()]);

    let logs = manager.poll();
    assert!(
        logs.iter().any(|m| m.message.contains("no arguments")),
        "{logs:?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_that_is_not_a_module_fails_to_start_with_a_reason() {
    let dir = scratch_dir("garbage");
    let path = dir.join("p.wasm");
    std::fs::write(&path, b"this is not WebAssembly").unwrap();

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    let error = manager
        .spawn(
            runtime.handle(),
            "bad",
            path.to_str().unwrap(),
            &[],
            Triggers::default(),
            &dir,
        )
        .expect_err("garbage must not start");
    assert!(
        error.to_string().contains("WebAssembly plugin 'bad'"),
        "{error}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
