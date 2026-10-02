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

//! Small pictures of many image files at once, for the filmstrip's strip: a bounded cache filled
//! on the blocking pool.
//!
//! `ImagePreview` shows one picture, resized to whatever pane it is in, through
//! `ratatui-image`'s `ThreadProtocol`. A strip needs a dozen at a time, all the same size, so each
//! is decoded *and encoded for its final cell size* on the pool and arrives as a finished
//! [`Protocol`] that the stateless `Image` widget draws with no work left for the render loop.
//! The size is the same for every slot, so the cache is keyed by path alone and is emptied
//! whenever the slot size changes (a resized terminal).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ratatui::layout::Size;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::{FilterType, Resize};
use shared::Vfs;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// The most thumbnails kept. A strip shows about a dozen, so this holds the screens either side
/// of it for scrolling back, while bounding what a folder of thousands of photos can cost.
pub const CAPACITY: usize = 64;

/// What the cache holds for one path.
pub enum Thumb {
    /// Being decoded.
    Loading,
    /// Ready to draw.
    Ready(Protocol),
    /// Not an image after all, or unreadable: the strip shows a placeholder.
    Failed,
}

struct Finished {
    path: PathBuf,
    size: Size,
    protocol: Option<Protocol>,
}

pub struct Thumbnails {
    picker: Picker,
    handle: tokio::runtime::Handle,
    vfs: Arc<dyn Vfs>,
    /// The slot size every entry in `slots` was (or is being) made for.
    size: Size,
    slots: HashMap<PathBuf, Thumb>,
    /// Paths in the order they were requested, oldest first, for eviction.
    order: VecDeque<PathBuf>,
    tx: UnboundedSender<Finished>,
    rx: UnboundedReceiver<Finished>,
}

impl Thumbnails {
    pub fn new(picker: Picker, handle: tokio::runtime::Handle, vfs: Arc<dyn Vfs>) -> Self {
        let (tx, rx) = unbounded_channel();
        Self {
            picker,
            handle,
            vfs,
            size: Size::new(0, 0),
            slots: HashMap::new(),
            order: VecDeque::new(),
            tx,
            rx,
        }
    }

    /// Starts making a thumbnail of each of `paths` that is an image and not already cached,
    /// `size` cells big. A different `size` than last time throws the cache away first, since
    /// every picture in it was encoded for the old one.
    pub fn request<'a>(&mut self, paths: impl IntoIterator<Item = &'a Path>, size: Size) {
        if size != self.size {
            self.size = size;
            self.slots.clear();
            self.order.clear();
        }
        if size.width == 0 || size.height == 0 {
            return;
        }
        for path in paths {
            if self.slots.contains_key(path) || !preview::is_image(path) {
                continue;
            }
            self.slots.insert(path.to_path_buf(), Thumb::Loading);
            self.order.push_back(path.to_path_buf());
            self.spawn(path.to_path_buf(), size);
        }
    }

    /// Takes in what the pool has finished since the last call, then evicts the oldest entries
    /// past [`CAPACITY`]. Call once per render tick, before drawing.
    pub fn poll(&mut self) {
        while let Ok(done) = self.rx.try_recv() {
            // A result for another size, or for a path evicted or cleared since, is stale.
            if done.size != self.size {
                continue;
            }
            if let Some(slot @ Thumb::Loading) = self.slots.get_mut(&done.path) {
                *slot = match done.protocol {
                    Some(protocol) => Thumb::Ready(protocol),
                    None => Thumb::Failed,
                };
            }
        }
        while self.order.len() > CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.slots.remove(&oldest);
            }
        }
    }

    /// What is cached for `path`; `None` means it was never requested (not an image, or not yet
    /// in a window the strip has shown).
    pub fn get(&self, path: &Path) -> Option<&Thumb> {
        self.slots.get(path)
    }

    /// Forgets every thumbnail, e.g. when leaving the view, so a folder of photos does not stay
    /// in memory behind a list nobody is looking at the pictures of.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.order.clear();
        self.size = Size::new(0, 0);
    }

    fn spawn(&self, path: PathBuf, size: Size) {
        let picker = self.picker.clone();
        let vfs = Arc::clone(&self.vfs);
        let tx = self.tx.clone();
        self.handle.spawn_blocking(move || {
            let protocol = preview::load_image(vfs.as_ref(), &path).and_then(|image| {
                picker
                    .new_protocol(image, size, Resize::Scale(Some(FilterType::Triangle)))
                    .ok()
            });
            let _ = tx.send(Finished {
                path,
                size,
                protocol,
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::LocalVfs;
    use std::time::{Duration, Instant};

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .expect("a runtime for the test")
    }

    fn png(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        image::RgbImage::from_pixel(8, 8, image::Rgb([200, 30, 30]))
            .save(&path)
            .expect("writing a test png");
        path
    }

    fn settle(thumbs: &mut Thumbnails, path: &Path) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            thumbs.poll();
            if !matches!(thumbs.get(path), Some(Thumb::Loading)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    fn cache(rt: &tokio::runtime::Runtime) -> Thumbnails {
        Thumbnails::new(
            Picker::halfblocks(),
            rt.handle().clone(),
            Arc::new(LocalVfs),
        )
    }

    #[test]
    fn an_image_becomes_a_ready_thumbnail_and_a_non_image_is_never_requested() {
        let rt = runtime();
        let dir = std::env::temp_dir().join(format!("mm-thumbs-a-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let img = png(&dir, "a.png");
        let txt = dir.join("a.txt");
        std::fs::write(&txt, "hello").expect("writing a test file");
        let mut thumbs = cache(&rt);
        thumbs.request([img.as_path(), txt.as_path()], Size::new(12, 4));
        assert!(settle(&mut thumbs, &img));
        assert!(matches!(thumbs.get(&img), Some(Thumb::Ready(_))));
        assert!(thumbs.get(&txt).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_broken_image_reports_failed_rather_than_loading_forever() {
        let rt = runtime();
        let dir = std::env::temp_dir().join(format!("mm-thumbs-b-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let bad = dir.join("bad.png");
        std::fs::write(&bad, b"not a png").expect("writing a test file");
        let mut thumbs = cache(&rt);
        thumbs.request([bad.as_path()], Size::new(12, 4));
        assert!(settle(&mut thumbs, &bad));
        assert!(matches!(thumbs.get(&bad), Some(Thumb::Failed)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_new_slot_size_drops_every_old_thumbnail() {
        let rt = runtime();
        let dir = std::env::temp_dir().join(format!("mm-thumbs-c-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let img = png(&dir, "c.png");
        let mut thumbs = cache(&rt);
        thumbs.request([img.as_path()], Size::new(12, 4));
        assert!(settle(&mut thumbs, &img));
        thumbs.request(std::iter::empty(), Size::new(18, 6));
        assert!(thumbs.get(&img).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_cache_never_holds_more_than_its_capacity() {
        let rt = runtime();
        let dir = std::env::temp_dir().join(format!("mm-thumbs-d-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let paths: Vec<PathBuf> = (0..CAPACITY + 20)
            .map(|i| png(&dir, &format!("{i:03}.png")))
            .collect();
        let mut thumbs = cache(&rt);
        thumbs.request(paths.iter().map(PathBuf::as_path), Size::new(12, 4));
        let last = paths.last().expect("some paths");
        assert!(settle(&mut thumbs, last));
        thumbs.poll();
        assert!(thumbs.slots.len() <= CAPACITY);
        assert_eq!(thumbs.slots.len(), thumbs.order.len());
        // The oldest went first.
        assert!(thumbs.get(&paths[0]).is_none());
        assert!(thumbs.get(last).is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_empty_slot_size_requests_nothing() {
        let rt = runtime();
        let mut thumbs = cache(&rt);
        thumbs.request([Path::new("/x/a.png")], Size::new(0, 0));
        assert!(thumbs.get(Path::new("/x/a.png")).is_none());
    }
}
