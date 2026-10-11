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

//! The counts a plugin asks for (`tally`): how many files and folders are below a folder and what
//! they add up to. The host counts, for the reason it walks: it knows how to read every kind of
//! path the browser can show. One count is one task on the blocking pool that checks for a stop
//! as it goes, and it is answered with a `tally_done` event rather than a reply, so a count that
//! takes seconds never holds up the plugin's other requests or its `tally_cancel`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use shared::Vfs;
use shared::tally::{Ending, TALLY_LIMIT, tally_dir};
use tokio::runtime::Handle;
use tokio::sync::mpsc::UnboundedSender;

use crate::protocol::{HostEvent, TallyDone, TallyEnding, TallyRequest};

/// The counts one plugin process has running, so it can stop them and so a process that ends
/// takes its counts with it.
#[derive(Debug, Default)]
pub(crate) struct Tallies {
    running: Mutex<HashMap<u64, Arc<AtomicBool>>>,
}

impl Tallies {
    fn running(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<AtomicBool>>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts `request` on `handle`'s blocking pool, writing its end as a line to `out`. Returns
    /// at once; fails only if a count by that number is still running.
    pub(crate) fn start(
        self: &Arc<Self>,
        handle: &Handle,
        vfs: Arc<dyn Vfs>,
        request: TallyRequest,
        out: UnboundedSender<String>,
    ) -> Result<(), String> {
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut running = self.running();
            if running.contains_key(&request.tally) {
                return Err(format!("tally {} is already running", request.tally));
            }
            running.insert(request.tally, Arc::clone(&cancel));
        }
        let tallies = Arc::clone(self);
        handle.spawn_blocking(move || {
            let TallyRequest { tally, root } = request;
            let (found, ending) = tally_dir(vfs.as_ref(), &root, TALLY_LIMIT, &cancel);
            tallies.running().remove(&tally);
            let done = TallyDone {
                tally,
                files: found.files,
                dirs: found.dirs,
                bytes: found.bytes,
                unreadable: found.unreadable,
                ending: match ending {
                    Ending::Complete => TallyEnding::Complete,
                    Ending::Truncated => TallyEnding::Truncated,
                    Ending::Cancelled => TallyEnding::Cancelled,
                },
            };
            let _ = out.send(HostEvent::tally_done(done).to_line());
        });
        Ok(())
    }

    /// Stops count `number`, if it is still running; it still reports that it ended.
    pub(crate) fn cancel(&self, number: u64) {
        if let Some(flag) = self.running().get(&number) {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Stops every count, for a process that has ended.
    pub(crate) fn cancel_all(&self) {
        for flag in self.running().values() {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use shared::MemVfs;
    use tokio::sync::mpsc::unbounded_channel;

    use super::*;

    fn tree() -> Arc<dyn Vfs> {
        let vfs = MemVfs::new();
        vfs.add_file("/r/one", vec![0u8; 10]);
        vfs.add_file("/r/a/two", vec![0u8; 20]);
        Arc::new(vfs)
    }

    fn request(tally: u64) -> TallyRequest {
        TallyRequest {
            tally,
            root: PathBuf::from("/r"),
        }
    }

    fn done_event(line: &str) -> TallyDone {
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["params"]["name"], "tally_done");
        serde_json::from_value(value["params"]["tally"].clone()).unwrap()
    }

    #[test]
    fn a_count_ends_with_one_event_carrying_its_number_and_totals() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (out, mut lines) = unbounded_channel();
        let tallies = Arc::new(Tallies::default());
        tallies
            .start(runtime.handle(), tree(), request(9), out)
            .unwrap();
        let line = runtime
            .block_on(async { tokio::time::timeout(Duration::from_secs(5), lines.recv()).await })
            .expect("the count ended in time")
            .expect("and said so");
        let done = done_event(&line);
        assert_eq!(
            (done.tally, done.files, done.dirs, done.bytes, done.ending),
            (9, 2, 1, 30, TallyEnding::Complete)
        );
        assert!(
            tallies.running().is_empty(),
            "a finished count is forgotten"
        );
    }

    #[test]
    fn a_number_still_running_is_refused_and_a_stopped_count_still_reports() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (out, mut lines) = unbounded_channel();
        let tallies = Arc::new(Tallies::default());
        // Hold the number by hand so the refusal does not race the count finishing.
        tallies
            .running()
            .insert(4, Arc::new(AtomicBool::new(false)));
        assert!(
            tallies
                .start(runtime.handle(), tree(), request(4), out.clone())
                .is_err()
        );
        tallies.cancel(4);
        assert!(tallies.running()[&4].load(Ordering::Relaxed));
        tallies.running().clear();

        tallies
            .start(runtime.handle(), tree(), request(5), out)
            .unwrap();
        tallies.cancel(5);
        let line = runtime
            .block_on(async { tokio::time::timeout(Duration::from_secs(5), lines.recv()).await })
            .expect("it ended in time")
            .expect("and said so");
        // Whether the stop beat the count or not, the plugin hears about number 5 exactly once.
        assert_eq!(done_event(&line).tally, 5);
    }
}
