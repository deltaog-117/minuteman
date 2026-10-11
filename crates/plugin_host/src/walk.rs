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

//! The tree walks a plugin asks for (`walk`). The host walks, because the host knows how to read
//! every kind of path the browser can show (the local disk, a machine over ssh, the inside of an
//! archive) and how to leave out what the user has chosen not to see; the plugin gets the entries
//! in batches, nearest level first, and can stop the walk. One walk is one task on the blocking
//! pool, and it checks for a stop between folders.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use browser::finder::{Walked, walk};
use browser::search::Limits;
use shared::Vfs;
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedSender;

use crate::protocol::{HostEvent, TreeEntry, WalkEnd, WalkOutcome, WalkRequest};

/// The most entries one `walk_batch` line holds, so a folder of a hundred thousand files is not
/// one line the plugin must buffer whole.
const BATCH: usize = 1000;

/// The walks one plugin process has running, so it can stop them and so a process that ends
/// takes its walks with it.
#[derive(Debug, Default)]
pub(crate) struct Walks {
    running: Mutex<HashMap<u64, Arc<AtomicBool>>>,
}

impl Walks {
    fn running(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<AtomicBool>>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts `request` on `handle`'s blocking pool, writing its batches and its end as lines to
    /// `out`. Returns at once; fails only if a walk by that number is still running.
    pub(crate) fn start(
        self: &Arc<Self>,
        handle: &Handle,
        vfs: Arc<dyn Vfs>,
        request: WalkRequest,
        out: UnboundedSender<String>,
    ) -> Result<(), String> {
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut running = self.running();
            if running.contains_key(&request.walk) {
                return Err(format!("walk {} is already running", request.walk));
            }
            running.insert(request.walk, Arc::clone(&cancel));
        }
        let walks = Arc::clone(self);
        handle.spawn_blocking(move || {
            let WalkRequest {
                walk: number,
                root,
                show_hidden,
            } = request;
            let ended = walk(
                vfs.as_ref(),
                &root,
                show_hidden,
                Limits::default(),
                &cancel,
                |batch| {
                    let entries: Vec<TreeEntry> = batch
                        .into_iter()
                        .map(|candidate| TreeEntry {
                            path: candidate.path,
                            rel: candidate.rel,
                            is_dir: candidate.is_dir,
                        })
                        .collect();
                    for chunk in entries.chunks(BATCH) {
                        let event = HostEvent::walk_batch(number, chunk.to_vec());
                        // Nobody is reading any more: stop walking for them.
                        if out.send(event.to_line()).is_err() {
                            cancel.store(true, Ordering::Relaxed);
                        }
                    }
                },
            );
            walks.running().remove(&number);
            let end = match ended {
                Walked::Done { truncated } => WalkEnd {
                    outcome: WalkOutcome::Done,
                    truncated,
                },
                Walked::Cancelled => WalkEnd {
                    outcome: WalkOutcome::Cancelled,
                    truncated: false,
                },
            };
            let _ = out.send(HostEvent::walk_done(number, end).to_line());
        });
        Ok(())
    }

    /// Stops walk `number`, if it is still running; it still reports that it ended.
    pub(crate) fn cancel(&self, number: u64) {
        if let Some(flag) = self.running().get(&number) {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Stops every walk, for a process that has ended.
    pub(crate) fn cancel_all(&self) {
        for flag in self.running().values() {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::MemVfs;
    use std::path::PathBuf;
    use std::time::Duration;
    use tokio::sync::mpsc::unbounded_channel;

    fn tree() -> Arc<dyn Vfs> {
        let vfs = MemVfs::new();
        vfs.add_file("/root/a.txt", b"");
        vfs.add_file("/root/src/main.rs", b"");
        vfs.add_file("/root/.hidden", b"");
        Arc::new(vfs)
    }

    fn request(walk: u64, show_hidden: bool) -> WalkRequest {
        WalkRequest {
            walk,
            root: PathBuf::from("/root"),
            show_hidden,
        }
    }

    /// Every line the walk wrote, once it has ended.
    fn run(show_hidden: bool) -> Vec<serde_json::Value> {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (out, mut lines) = unbounded_channel();
        let walks = Arc::new(Walks::default());
        walks
            .start(runtime.handle(), tree(), request(5, show_hidden), out)
            .unwrap();
        let mut all = Vec::new();
        loop {
            let line = runtime
                .block_on(async {
                    tokio::time::timeout(Duration::from_secs(5), lines.recv()).await
                })
                .expect("the walk ended in time")
                .expect("the walk reported its end");
            let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
            let done = value["params"]["name"] == "walk_done";
            all.push(value);
            if done {
                return all;
            }
        }
    }

    #[test]
    fn a_walk_sends_the_entries_then_its_end() {
        let lines = run(false);
        let rels: Vec<&str> = lines
            .iter()
            .flat_map(|line| {
                line["params"]["walk"]["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
            })
            .map(|entry| entry["rel"].as_str().unwrap())
            .collect();
        assert_eq!(
            rels,
            ["src", "a.txt", "src/main.rs"],
            "a level in the order the Vfs lists it, then the next"
        );
        let end = &lines.last().unwrap()["params"]["walk"];
        assert_eq!(end["walk"], 5);
        assert_eq!(end["end"]["outcome"], "done");
        assert_eq!(end["end"]["truncated"], false);
    }

    #[test]
    fn hidden_entries_are_walked_only_when_asked_for() {
        let has_hidden = |lines: Vec<serde_json::Value>| {
            lines.iter().any(|line| {
                line["params"]["walk"]["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|entry| entry["rel"] == ".hidden")
            })
        };
        assert!(!has_hidden(run(false)));
        assert!(has_hidden(run(true)));
    }

    #[test]
    fn a_second_walk_under_a_running_number_is_refused() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let walks = Arc::new(Walks::default());
        walks.running().insert(9, Arc::new(AtomicBool::new(false)));
        let (out, _lines) = unbounded_channel();
        let refused = walks.start(runtime.handle(), tree(), request(9, false), out);
        assert_eq!(refused, Err("walk 9 is already running".to_string()));
    }

    #[test]
    fn a_cancelled_walk_still_says_it_ended() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let walks = Arc::new(Walks::default());
        let (out, mut lines) = unbounded_channel();
        walks
            .start(runtime.handle(), tree(), request(2, false), out)
            .unwrap();
        walks.cancel_all();
        let last = runtime.block_on(async {
            let mut last = None;
            while let Ok(Some(line)) =
                tokio::time::timeout(Duration::from_secs(5), lines.recv()).await
            {
                last = Some(line);
            }
            last
        });
        let value: serde_json::Value = serde_json::from_str(last.unwrap().trim()).unwrap();
        assert_eq!(value["params"]["name"], "walk_done");
    }
}
