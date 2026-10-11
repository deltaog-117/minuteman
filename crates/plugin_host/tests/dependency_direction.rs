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

//! Which way the workspace's crates may depend on each other. `crates/` is the core and `plugins/`
//! holds the bundled plugins; the core must not name a plugin (so it can run, and be replaced,
//! without them), `plugin_api` must stay light enough for any plugin to depend on, and a plugin
//! reaches the program through `plugin_api` and `shared` only, never through the host.
//!
//! It reads `cargo metadata`, so it checks the manifests as they are rather than a list kept
//! beside them. The rules are a pure function over that JSON, tried on made-up workspaces below so
//! a rule that stopped firing would show up here.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

/// The path dependencies a `plugins/*` crate may have.
const PLUGIN_MAY_USE: [&str; 2] = ["plugin_api", "shared"];
/// The path dependencies `plugin_api` may have.
const API_MAY_USE: [&str; 1] = ["shared"];

fn path_deps(package: &Value) -> Vec<(String, PathBuf)> {
    package["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|dep| {
            let name = dep["name"].as_str()?.to_string();
            let path = dep["path"].as_str()?;
            Some((name, PathBuf::from(path)))
        })
        .collect()
}

/// Every rule the metadata breaks, one human-readable line each; empty when none.
fn violations(metadata: &Value) -> Vec<String> {
    let root = Path::new(metadata["workspace_root"].as_str().unwrap_or(""));
    let (core, bundled) = (root.join("crates"), root.join("plugins"));
    let mut found = Vec::new();

    for package in metadata["packages"].as_array().into_iter().flatten() {
        let name = package["name"].as_str().unwrap_or("?");
        let manifest = Path::new(package["manifest_path"].as_str().unwrap_or(""));
        let deps = path_deps(package);

        if manifest.starts_with(&core) {
            for (dep, path) in &deps {
                if path.starts_with(&bundled) {
                    found.push(format!(
                        "core crate `{name}` depends on bundled plugin `{dep}`"
                    ));
                }
            }
        }
        if name == "plugin_api" {
            for (dep, _) in deps
                .iter()
                .filter(|(dep, _)| !API_MAY_USE.contains(&dep.as_str()))
            {
                found.push(format!(
                    "`plugin_api` depends on `{dep}`, which is not light"
                ));
            }
        }
        if manifest.starts_with(&bundled) {
            for (dep, _) in deps
                .iter()
                .filter(|(dep, _)| !PLUGIN_MAY_USE.contains(&dep.as_str()))
            {
                found.push(format!(
                    "bundled plugin `{name}` depends on `{dep}`; a plugin may use only plugin_api and shared"
                ));
            }
        }
    }
    found
}

fn workspace_metadata() -> Value {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata prints JSON")
}

fn package(root: &str, dir: &str, name: &str, deps: &[(&str, &str)]) -> Value {
    json!({
        "name": name,
        "manifest_path": format!("{root}/{dir}/{name}/Cargo.toml"),
        "dependencies": deps
            .iter()
            .map(|(dep, dep_dir)| json!({ "name": dep, "path": format!("{root}/{dep_dir}/{dep}") }))
            .collect::<Vec<_>>(),
    })
}

fn workspace(packages: Vec<Value>) -> Value {
    json!({ "workspace_root": "/w", "packages": packages })
}

#[test]
fn this_workspace_keeps_its_dependencies_pointing_inward() {
    let found = violations(&workspace_metadata());
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn a_core_crate_naming_a_bundled_plugin_is_caught() {
    let found = violations(&workspace(vec![
        package("/w", "crates", "tui", &[("finder", "plugins")]),
        package("/w", "plugins", "finder", &[("plugin_api", "crates")]),
    ]));
    assert_eq!(
        found,
        ["core crate `tui` depends on bundled plugin `finder`"]
    );
}

#[test]
fn a_plugin_reaching_for_the_host_or_the_file_operations_is_caught() {
    let found = violations(&workspace(vec![package(
        "/w",
        "plugins",
        "finder",
        &[
            ("plugin_host", "crates"),
            ("file_ops", "crates"),
            ("shared", "crates"),
        ],
    )]));
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found
            .iter()
            .all(|line| line.contains("bundled plugin `finder`"))
    );
}

#[test]
fn plugin_api_growing_a_heavy_dependency_is_caught() {
    let found = violations(&workspace(vec![package(
        "/w",
        "crates",
        "plugin_api",
        &[("shared", "crates"), ("file_ops", "crates")],
    )]));
    assert_eq!(
        found,
        ["`plugin_api` depends on `file_ops`, which is not light"]
    );
}

#[test]
fn a_well_formed_workspace_reports_nothing() {
    let found = violations(&workspace(vec![
        package("/w", "crates", "tui", &[("plugin_host", "crates")]),
        package("/w", "crates", "plugin_api", &[("shared", "crates")]),
        package(
            "/w",
            "plugins",
            "finder",
            &[("plugin_api", "crates"), ("shared", "crates")],
        ),
    ]));
    assert!(found.is_empty(), "{found:?}");
}
