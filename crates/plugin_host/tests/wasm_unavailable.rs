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

//! A build without the `wasm` feature must say how to get it, not fail in a way that looks like a
//! missing program. `wasm_sandbox.rs` covers the build with it.
#![cfg(not(feature = "wasm"))]

use plugin_host::{PluginManager, Triggers};

#[test]
fn a_wasm_plugin_in_a_build_without_the_feature_says_how_to_get_it() {
    let dir = std::env::temp_dir();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    let error = manager
        .spawn(
            runtime.handle(),
            "w",
            "anything.wasm",
            &[],
            Triggers::default(),
            &dir,
        )
        .expect_err("no runtime is built in");
    let message = error.to_string();
    assert!(message.contains("no WebAssembly support"), "{message}");
    assert!(message.contains("--features wasm"), "{message}");
}

#[test]
fn programs_are_unaffected_by_the_feature_being_off() {
    let dir = std::env::temp_dir();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut manager = PluginManager::new();
    let error = manager
        .spawn(
            runtime.handle(),
            "p",
            "no-such-program-minuteman",
            &[],
            Triggers::default(),
            &dir,
        )
        .expect_err("the program does not exist");
    assert!(
        error.to_string().contains("failed to start plugin"),
        "{error}"
    );
}

#[test]
fn checking_a_wasm_plugin_in_a_build_without_the_feature_says_how_to_get_it() {
    let result = plugin_host::probe::probe(
        "w",
        "anything.wasm",
        &[],
        &std::env::temp_dir(),
        std::time::Duration::from_millis(50),
    );
    let message = result.expect_err("no runtime is built in").to_string();
    assert!(message.contains("--features wasm"), "{message}");
}
