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

//! The daemon end to end, over a private session bus: the real `mman-portal` binary, a stand-in
//! for `mman` (a shell script that writes the answer it is told to), and a D-Bus client playing
//! the portal. Skipped, with a note, when `dbus-daemon` is not installed.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use zbus::zvariant::{ObjectPath, OwnedValue, Value};

const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.minuteman";
const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
const CHOOSER: &str = "org.freedesktop.impl.portal.FileChooser";

/// Stands in for `mman`: logs its arguments, then behaves as the contents of `mode` say.
const FAKE_MMAN: &str = r#"#!/bin/sh
printf '%s\n' "$@" > "$STUB_DIR/args"
out="$2"
case "$(cat "$STUB_DIR/mode")" in
  choose) printf '%s\0%s\0' "/data/a b#c.txt" "/data/é.txt" > "$out" ;;
  cancel) : > "$out" ;;
  closed) : ;;
  garbage) printf 'relative/path\0' > "$out" ;;
  hang) sleep 30 ;;
esac
"#;

struct Scene {
    dir: PathBuf,
    bus: Child,
    daemon: Child,
    connection: zbus::Connection,
}

impl Drop for Scene {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
        let _ = self.bus.kill();
        let _ = self.bus.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Scene {
    /// `None` when there is no `dbus-daemon` to run the test on.
    async fn start(name: &str) -> Option<Self> {
        let dir =
            std::env::temp_dir().join(format!("mman-portal-e2e-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir is writable");
        let stub = dir.join("mman");
        std::fs::write(&stub, FAKE_MMAN).expect("temp dir is writable");
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
            .expect("temp dir is writable");

        let mut bus = match Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .arg(format!("--address=unix:path={}", dir.join("bus").display()))
            .stdout(Stdio::piped())
            .spawn()
        {
            Ok(bus) => bus,
            Err(_) => {
                eprintln!("skipped: dbus-daemon is not installed");
                let _ = std::fs::remove_dir_all(&dir);
                return None;
            }
        };
        let mut address = String::new();
        BufReader::new(bus.stdout.take().expect("piped"))
            .read_line(&mut address)
            .expect("the bus prints its address");
        let address = address.trim().to_owned();

        let daemon = Command::new(env!("CARGO_BIN_EXE_mman-portal"))
            .env("DBUS_SESSION_BUS_ADDRESS", &address)
            .env("MINUTEMAN_PORTAL_TERMINAL_CMD", "env")
            .env("MINUTEMAN_PORTAL_MMAN", &stub)
            .env("STUB_DIR", &dir)
            .env("HOME", &dir)
            .stderr(Stdio::null())
            .spawn()
            .expect("the daemon starts");

        let connection = zbus::connection::Builder::address(address.as_str())
            .expect("a valid address")
            .build()
            .await
            .expect("connects to the private bus");
        let scene = Self {
            dir,
            bus,
            daemon,
            connection,
        };
        scene.wait_for_name().await;
        Some(scene)
    }

    async fn wait_for_name(&self) {
        for _ in 0..100 {
            let reply = self
                .connection
                .call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "NameHasOwner",
                    &(BUS_NAME,),
                )
                .await
                .expect("the bus answers");
            if reply.body().deserialize::<bool>().expect("a bool") {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the daemon never took {BUS_NAME}");
    }

    fn set_mode(&self, mode: &str) {
        std::fs::write(self.dir.join("mode"), mode).expect("temp dir is writable");
    }

    fn stub_args(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("args"))
            .expect("the stub ran")
            .lines()
            .map(str::to_owned)
            .collect()
    }

    async fn call(
        &self,
        method: &'static str,
        handle: &'static str,
        options: HashMap<&str, Value<'_>>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let reply = self
            .connection
            .call_method(
                Some(BUS_NAME),
                OBJECT_PATH,
                Some(CHOOSER),
                method,
                &(
                    ObjectPath::try_from(handle).expect("a path"),
                    "app.id",
                    "",
                    "Pick one",
                    options,
                ),
            )
            .await
            .expect("the daemon replies");
        reply.body().deserialize().expect("(ua{sv})")
    }
}

fn uris(results: &HashMap<String, OwnedValue>) -> Vec<String> {
    let Some(value) = results.get("uris") else {
        return Vec::new();
    };
    let Value::Array(array) = &**value else {
        panic!("uris is not an array: {value:?}");
    };
    array
        .iter()
        .map(|item| match item {
            Value::Str(text) => text.as_str().to_owned(),
            other => panic!("not a string: {other:?}"),
        })
        .collect()
}

fn bytes(path: &Path) -> Value<'static> {
    let mut bytes = path.as_os_str().as_encoded_bytes().to_vec();
    bytes.push(0);
    Value::new(bytes)
}

#[tokio::test]
async fn a_pick_comes_back_as_encoded_file_uris() {
    let Some(scene) = Scene::start("choose").await else {
        return;
    };
    scene.set_mode("choose");
    let options = HashMap::from([
        ("multiple", Value::from(true)),
        ("current_folder", bytes(Path::new("/data"))),
    ]);
    let (response, results) = scene
        .call(
            "OpenFile",
            "/org/freedesktop/portal/desktop/request/t/open",
            options,
        )
        .await;
    assert_eq!(response, 0);
    assert_eq!(
        uris(&results),
        ["file:///data/a%20b%23c.txt", "file:///data/%C3%A9.txt"]
    );
    let args = scene.stub_args();
    assert_eq!(args[0], "--chooser-file");
    assert_eq!(&args[2..], ["--pick", "files", "/data"]);
}

#[tokio::test]
async fn backing_out_and_closing_the_window_are_both_a_cancel() {
    let Some(scene) = Scene::start("cancel").await else {
        return;
    };
    for mode in ["cancel", "closed"] {
        scene.set_mode(mode);
        let (response, results) = scene
            .call(
                "OpenFile",
                "/org/freedesktop/portal/desktop/request/t/c",
                HashMap::new(),
            )
            .await;
        assert_eq!((response, uris(&results)), (1, vec![]), "{mode}");
    }
}

#[tokio::test]
async fn an_answer_that_is_not_one_is_a_failure_not_a_cancel() {
    let Some(scene) = Scene::start("garbage").await else {
        return;
    };
    scene.set_mode("garbage");
    let (response, _) = scene
        .call(
            "OpenFile",
            "/org/freedesktop/portal/desktop/request/t/g",
            HashMap::new(),
        )
        .await;
    assert_eq!(response, 2);
}

#[tokio::test]
async fn saving_passes_the_suggested_name_and_folder() {
    let Some(scene) = Scene::start("save").await else {
        return;
    };
    scene.set_mode("choose");
    let options = HashMap::from([
        ("current_name", Value::from("report.pdf")),
        ("current_folder", bytes(Path::new("/data"))),
    ]);
    let (response, _) = scene
        .call(
            "SaveFile",
            "/org/freedesktop/portal/desktop/request/t/save",
            options,
        )
        .await;
    assert_eq!(response, 0);
    assert_eq!(
        &scene.stub_args()[2..],
        ["--pick", "save", "--pick-name", "report.pdf", "/data"]
    );
}

#[tokio::test]
async fn saving_several_files_names_each_inside_the_chosen_folder() {
    let Some(scene) = Scene::start("savefiles").await else {
        return;
    };
    // The stand-in answers with two paths; the first is the folder.
    scene.set_mode("choose");
    let files = Value::new(vec![
        b"one.txt\0".to_vec(),
        b"two words.txt\0".to_vec(),
        b"../escape\0".to_vec(),
    ]);
    let (response, results) = scene
        .call(
            "SaveFiles",
            "/org/freedesktop/portal/desktop/request/t/many",
            HashMap::from([("files", files)]),
        )
        .await;
    assert_eq!(response, 0);
    assert_eq!(
        uris(&results),
        [
            "file:///data/a%20b%23c.txt/one.txt",
            "file:///data/a%20b%23c.txt/two%20words.txt"
        ]
    );
    assert_eq!(&scene.stub_args()[2..4], ["--pick", "directory"]);
}

#[tokio::test]
async fn closing_the_request_ends_a_pick_that_is_still_open() {
    let Some(scene) = Scene::start("close").await else {
        return;
    };
    scene.set_mode("hang");
    let handle = "/org/freedesktop/portal/desktop/request/t/hang";
    let pending = scene.call("OpenFile", handle, HashMap::new());
    let closing = async {
        // Close is only valid once the daemon has exported the request object.
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let closed = scene
                .connection
                .call_method(
                    Some(BUS_NAME),
                    handle,
                    Some("org.freedesktop.impl.portal.Request"),
                    "Close",
                    &(),
                )
                .await;
            if closed.is_ok() {
                return;
            }
        }
        panic!("the request object never appeared");
    };
    let ((response, _), ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(pending, closing)
    })
    .await
    .expect("Close ends the pick well before the stub's 30 seconds");
    assert_eq!(response, 1);
}
