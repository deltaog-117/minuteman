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
//! Being told that a folder changed, so a view of it can be refreshed when something happens
//! instead of on a timer. See [`Vfs::watch`](crate::Vfs::watch).

use std::any::Any;
use std::path::Path;
use std::sync::Arc;

/// What a backend calls when the watched folder changes.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// Keeps a watch running; dropping it stops the watching.
pub struct WatchGuard(#[allow(dead_code)] Box<dyn Any + Send>);

impl WatchGuard {
    pub fn new(watcher: impl Any + Send) -> Self {
        Self(Box::new(watcher))
    }
}

/// Watches `path` (not what is below it) on this machine's disk with the platform's own
/// notification service. `None` when the service cannot be started or the folder cannot be
/// watched (it is gone, or the user's watch limit is spent), which the caller treats as "poll".
#[cfg(feature = "watch")]
pub(crate) fn watch_local(path: &Path, wake: Wake) -> Option<WatchGuard> {
    use notify::{RecursiveMode, Watcher};

    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        // Reading a folder is itself an access event, so waking on those would make every
        // listing trigger the next one. A failed event means events were lost: list again.
        if event.is_ok_and(|event| event.kind.is_access()) {
            return;
        }
        wake();
    })
    .ok()?;
    watcher.watch(path, RecursiveMode::NonRecursive).ok()?;
    Some(WatchGuard::new(watcher))
}

#[cfg(not(feature = "watch"))]
pub(crate) fn watch_local(_path: &Path, _wake: Wake) -> Option<WatchGuard> {
    None
}

#[cfg(test)]
mod tests {
    // Without the feature `LocalVfs::watch` declines, so there is nothing to test.
    #[cfg(feature = "watch")]
    mod local {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};

        use crate::{LocalVfs, Vfs};

        use super::super::*;

        fn counting_wake() -> (Wake, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            let wake: Wake = {
                let count = Arc::clone(&count);
                Arc::new(move || {
                    count.fetch_add(1, Ordering::SeqCst);
                })
            };
            (wake, count)
        }

        fn wait_for(count: &AtomicUsize, at_least: usize) -> bool {
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if count.load(Ordering::SeqCst) >= at_least {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            false
        }

        fn scratch(label: &str) -> std::path::PathBuf {
            let dir = std::env::temp_dir()
                .join(format!("minuteman-watch-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        #[test]
        fn a_new_file_in_a_watched_folder_wakes_the_caller() {
            let dir = scratch("create");
            let (wake, count) = counting_wake();
            let _guard = LocalVfs.watch(&dir, wake).expect("inotify should start");
            std::fs::write(dir.join("a.txt"), b"x").unwrap();
            assert!(wait_for(&count, 1));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn listing_the_watched_folder_does_not_wake_the_caller() {
            let dir = scratch("quiet");
            std::fs::write(dir.join("a.txt"), b"x").unwrap();
            let (wake, count) = counting_wake();
            let _guard = LocalVfs.watch(&dir, wake).expect("inotify should start");
            for _ in 0..5 {
                LocalVfs.list_dir(&dir).unwrap();
            }
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(count.load(Ordering::SeqCst), 0);
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn dropping_the_guard_stops_the_wakes() {
            let dir = scratch("drop");
            let (wake, count) = counting_wake();
            drop(LocalVfs.watch(&dir, wake).expect("inotify should start"));
            std::fs::write(dir.join("a.txt"), b"x").unwrap();
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(count.load(Ordering::SeqCst), 0);
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_missing_folder_cannot_be_watched() {
            let (wake, _) = counting_wake();
            assert!(
                LocalVfs
                    .watch(std::path::Path::new("/nonexistent/minuteman"), wake)
                    .is_none()
            );
        }
    }
}
