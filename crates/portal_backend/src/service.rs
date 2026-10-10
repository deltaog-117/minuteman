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

//! The D-Bus side: `org.freedesktop.impl.portal.FileChooser`, and the `Request` object each call
//! exports so the portal can withdraw it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use shared::chooser::Selection;
use tokio::sync::Notify;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{ObjectServer, interface};

use crate::request::{PickRequest, RawOptions, file_names};
use crate::session::{Launcher, Outcome};
use crate::uri::file_uri;

/// The `response` of the portal's reply.
const SUCCESS: u32 = 0;
const CANCELLED: u32 = 1;
const FAILED: u32 = 2;

type Results = HashMap<String, OwnedValue>;

pub struct FileChooser {
    launcher: Arc<Launcher>,
}

impl FileChooser {
    pub fn new(launcher: Launcher) -> Self {
        Self {
            launcher: Arc::new(launcher),
        }
    }

    /// Exports the `Request` object at `handle`, runs the pick, and turns its end into the reply.
    /// `uris` maps the chosen paths to what is sent back.
    async fn serve(
        &self,
        server: &ObjectServer,
        handle: ObjectPath<'_>,
        call: &str,
        request: PickRequest,
        uris: impl FnOnce(Selection) -> Vec<PathBuf>,
    ) -> (u32, Results) {
        let withdrawn = Arc::new(Notify::new());
        let exported = server
            .at(
                handle.clone(),
                Request {
                    withdrawn: Arc::clone(&withdrawn),
                },
            )
            .await;
        if !matches!(exported, Ok(true)) {
            log(format_args!(
                "{call}: could not export the request object at {handle}"
            ));
            return (FAILED, Results::new());
        }

        let started = Instant::now();
        let outcome = self.launcher.pick(&request, &withdrawn).await;
        let _ = server.remove::<Request, _>(handle).await;

        // Counts and durations only: the paths and the title are the user's, and a log is read
        // by others.
        match outcome {
            Ok(Outcome::Chosen(selection)) => {
                let paths = uris(selection);
                log(format_args!(
                    "{call}: {:?} chose {} path(s) in {:?}",
                    request.kind,
                    paths.len(),
                    started.elapsed()
                ));
                (SUCCESS, results_with_uris(&paths))
            }
            Ok(Outcome::Cancelled) => {
                log(format_args!(
                    "{call}: cancelled after {:?}",
                    started.elapsed()
                ));
                (CANCELLED, Results::new())
            }
            Err(error) => {
                log(format_args!("{call}: failed: {error}"));
                (FAILED, Results::new())
            }
        }
    }
}

#[interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooser {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    async fn open_file(
        &self,
        handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        let request = PickRequest::open(title, &raw_options(&options));
        self.serve(server, handle, "OpenFile", request, Selection::into_paths)
            .await
    }

    async fn save_file(
        &self,
        handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        let request = PickRequest::save(title, &raw_options(&options));
        self.serve(server, handle, "SaveFile", request, Selection::into_paths)
            .await
    }

    async fn save_files(
        &self,
        handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        let raw = raw_options(&options);
        let names = file_names(&raw.files);
        let request = PickRequest::save_into_folder(title, &raw);
        // The pick is a folder; the reply names each file inside it.
        self.serve(server, handle, "SaveFiles", request, move |folders| {
            let folder = folders.into_paths().into_iter().next().unwrap_or_default();
            names.iter().map(|name| folder.join(name)).collect()
        })
        .await
    }
}

/// `org.freedesktop.impl.portal.Request`: lets the portal withdraw a call that is still open.
struct Request {
    withdrawn: Arc<Notify>,
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    async fn close(&self) {
        // `notify_one` keeps the wake-up if the pick has not started waiting yet.
        self.withdrawn.notify_one();
    }
}

fn results_with_uris(paths: &[PathBuf]) -> Results {
    let uris: Vec<String> = paths.iter().map(|path| file_uri(path)).collect();
    let mut results = Results::new();
    if let Ok(value) = OwnedValue::try_from(Value::new(uris)) {
        results.insert("uris".into(), value);
    }
    results
}

fn raw_options(options: &HashMap<String, OwnedValue>) -> RawOptions {
    let get = |key: &str| options.get(key).map(|value| &**value);
    RawOptions {
        multiple: get("multiple").is_some_and(|value| matches!(value, Value::Bool(true))),
        directory: get("directory").is_some_and(|value| matches!(value, Value::Bool(true))),
        current_name: get("current_name").and_then(|value| match value {
            Value::Str(text) => Some(text.as_str().to_owned()),
            _ => None,
        }),
        current_folder: get("current_folder").and_then(bytes_of),
        current_file: get("current_file").and_then(bytes_of),
        files: get("files").map(byte_strings_of).unwrap_or_default(),
    }
}

/// An `ay`.
fn bytes_of(value: &Value<'_>) -> Option<Vec<u8>> {
    let Value::Array(array) = value else {
        return None;
    };
    array
        .iter()
        .map(|item| match item {
            Value::U8(byte) => Some(*byte),
            _ => None,
        })
        .collect()
}

/// An `aay`; an entry that is not a byte string is skipped.
fn byte_strings_of(value: &Value<'_>) -> Vec<Vec<u8>> {
    let Value::Array(array) = value else {
        return Vec::new();
    };
    array.iter().filter_map(bytes_of).collect()
}

fn log(message: std::fmt::Arguments<'_>) {
    eprintln!("mman-portal: {message}");
}
