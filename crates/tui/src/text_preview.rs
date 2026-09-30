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

//! Reads the selected file for the preview pane off the render thread, the same
//! never-block-the-render-loop treatment `image_preview` gives image decoding: a file on a slow
//! or network-backed filesystem must not stall input handling just because it is being looked at.
//!
//! Despite the name this covers every previewable file that is not an image: text is read whole,
//! an archive is listed, and anything else is read as its first bytes for the hex view (see
//! `preview::load` for how that is decided). The pane it fills scrolls, and `Scroll` holds where.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use preview::Loaded;
use preview::highlight::HighlightedLine;
use shared::{LocalVfs, Vfs};
use theming::{HookKind, PreviewHook};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::preview_hook::{self, HookOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStatus {
    /// Nothing selected, or the selection has nothing to show (not a regular file).
    Empty,
    /// The file is being read off-thread.
    Loading,
    /// Read successfully; `content` holds what to show.
    Ready,
    /// Read failed, or the file is too big to preview as text.
    Failed,
}

/// How far the preview pane is scrolled, in rows.
///
/// The offset may be set past the end (a key pressed at the bottom, a wheel notch on a short
/// file) because the pane's height and the content's length are only known when it is drawn;
/// `fit` then clamps it, so the offset a user sees is always a real one.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Scroll {
    offset: usize,
    /// Rows the pane showed the last time it was drawn; a key scrolls by half of it.
    viewport: usize,
}

impl Scroll {
    #[cfg(test)]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Moves by `rows`, up when negative. Stops at the top; the bottom is enforced by `fit`.
    pub fn by(&mut self, rows: isize) {
        self.offset = self.offset.saturating_add_signed(rows);
    }

    /// Half the pane's height, at least one row: what a scroll key moves by.
    pub fn half_page(&self) -> isize {
        (self.viewport / 2).max(1) as isize
    }

    /// Records the pane's height and clamps the offset so the last row of `total` can be the
    /// last one shown but nothing beyond it. Returns the clamped offset.
    pub fn fit(&mut self, total: usize, viewport: usize) -> usize {
        self.viewport = viewport;
        self.offset = self.offset.min(total.saturating_sub(viewport));
        self.offset
    }

    pub fn reset(&mut self) {
        self.offset = 0;
    }
}

struct ReadOutcome {
    path: PathBuf,
    generation: u64,
    loaded: Loaded,
    /// Tokenized alongside the read, off the render thread — `Some` exactly when `loaded` is
    /// `Loaded::Text`, since nothing else has source lines to color.
    highlighted: Option<Vec<HighlightedLine>>,
    /// Whether this read went through a configured `[[preview_hook]]` rather than
    /// `preview::load` — a hook's own failure/timeout/missing-program falls back to just the
    /// file's name (like an unsupported file), not the "preview failed" message a real read
    /// failure shows.
    via_hook: bool,
}

/// How many rows a text takes when wrapped to a width, remembered with that width. Measuring
/// walks the whole text, which is wasted work on every frame for a file that has not changed.
#[derive(Debug, Default)]
pub struct RowCache(Option<(u16, usize)>);

impl RowCache {
    /// The rows at `width`, measured with `measure` only when the width differs from the last
    /// answer or the cache was cleared.
    pub fn get(&mut self, width: u16, measure: impl FnOnce() -> usize) -> usize {
        match self.0 {
            Some((cached_width, rows)) if cached_width == width => rows,
            _ => {
                let rows = measure();
                self.0 = Some((width, rows));
                rows
            }
        }
    }

    fn clear(&mut self) {
        self.0 = None;
    }
}

pub struct TextPreview {
    current: Option<PathBuf>,
    status: PreviewStatus,
    /// Only ever `Text`, `Bytes` or `Archive`: a failed or unsupported read shows in `status`.
    content: Option<Loaded>,
    /// `content`'s tokens, one vec per source line, when `content` is `Loaded::Text` — see
    /// [`ReadOutcome::highlighted`].
    highlighted: Option<Vec<HighlightedLine>>,
    scroll: Scroll,
    rows: RowCache,
    /// When the current selection's typewriter reveal started, so `preview_view::draw_text` can
    /// compute how many lines have appeared so far (see `anim::revealed_lines`). Set only when
    /// the selection changes to a new file — never by `reload`, so a log being appended to while
    /// selected doesn't replay the reveal — and read but never advanced here, since reading it
    /// needs no `&mut self`.
    reveal_started: Option<Instant>,
    /// Counts reads started. Only the latest one's result is kept, so a slow read begun before
    /// the file changed can't overwrite the newer read of the same path.
    generation: u64,
    read_tx: UnboundedSender<ReadOutcome>,
    read_rx: UnboundedReceiver<ReadOutcome>,
    handle: tokio::runtime::Handle,
    /// Where the files being previewed are read from.
    vfs: Arc<dyn Vfs>,
}

impl TextPreview {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        let (read_tx, read_rx) = unbounded_channel();
        Self {
            current: None,
            status: PreviewStatus::Empty,
            content: None,
            highlighted: None,
            scroll: Scroll::default(),
            rows: RowCache::default(),
            reveal_started: None,
            generation: 0,
            read_tx,
            read_rx,
            handle,
            vfs: Arc::new(LocalVfs),
        }
    }

    /// Reads the previewed files through `vfs` instead of the local disk.
    pub fn with_vfs(mut self, vfs: Arc<dyn Vfs>) -> Self {
        self.vfs = vfs;
        self
    }

    /// When the current selection's typewriter reveal started, or `None` if it never started
    /// (nothing selected yet, or this preview was built with [`Self::showing`] for a test).
    pub fn reveal_started(&self) -> Option<Instant> {
        self.reveal_started
    }

    /// Arms the reveal as of `started`, for a test of `preview_view::draw_text`'s capping without
    /// waiting on a real background read.
    #[cfg(test)]
    pub fn arm_reveal_for_test(&mut self, started: Instant) {
        self.reveal_started = Some(started);
    }

    /// A preview already showing `loaded`, for tests of the code that draws one. A text's tokens
    /// are computed with no known file name, so it highlights as plain text — use
    /// [`Self::showing_named`] to test an actual language's colors.
    #[cfg(test)]
    pub fn showing(handle: tokio::runtime::Handle, loaded: Loaded) -> Self {
        Self::showing_named(handle, loaded, "")
    }

    /// [`Self::showing`], tokenizing a `Loaded::Text` as `file_name` would be highlighted for
    /// real, so a test can check specific tokens' colors.
    #[cfg(test)]
    pub fn showing_named(handle: tokio::runtime::Handle, loaded: Loaded, file_name: &str) -> Self {
        let mut preview = Self::new(handle);
        preview.highlighted = match &loaded {
            Loaded::Text(text) => Some(preview::highlight::highlight(text, file_name)),
            _ => None,
        };
        preview.content = Some(loaded);
        preview.status = PreviewStatus::Ready;
        preview
    }

    /// A preview in `status` with nothing to show, for the same purpose.
    #[cfg(test)]
    pub fn in_status(handle: tokio::runtime::Handle, status: PreviewStatus) -> Self {
        let mut preview = Self::new(handle);
        preview.status = status;
        preview
    }

    pub fn status(&self) -> PreviewStatus {
        self.status
    }

    #[cfg(test)]
    pub fn content(&self) -> Option<&Loaded> {
        self.content.as_ref()
    }

    /// Scrolls by `rows` (up when negative).
    pub fn scroll_rows(&mut self, rows: isize) {
        self.scroll.by(rows);
    }

    /// Scrolls by half a screen, `direction` being `1` for down and `-1` for up.
    pub fn scroll_half_pages(&mut self, direction: isize) {
        self.scroll.by(direction * self.scroll.half_page());
    }

    /// The content, its highlighted tokens (when it is text), the wrapped-row cache and the
    /// scroll position as separate borrows, so the drawing code can read the content while it
    /// updates the other two.
    pub fn parts(
        &mut self,
    ) -> (
        Option<&Loaded>,
        Option<&[HighlightedLine]>,
        &mut RowCache,
        &mut Scroll,
    ) {
        (
            self.content.as_ref(),
            self.highlighted.as_deref(),
            &mut self.rows,
            &mut self.scroll,
        )
    }

    /// Re-reads the current file without clearing what is shown, for when it changed on disk
    /// while staying selected. A no-op when nothing previewable is selected. The scroll position
    /// is kept, so a log being appended to does not jump back to the top.
    pub fn reload(&mut self, hooks: &[PreviewHook]) {
        if let Some(path) = self.current.clone() {
            let hook = text_hook_for(&path, hooks);
            self.spawn_read(path, hook);
        }
    }

    /// Call once per render tick with the selected file, if it is one that belongs here (not a
    /// folder, and not a file `image_preview` already claims; the caller decides). Starts
    /// reading it if it is new, and drains any completed read from the background thread.
    pub fn update(&mut self, selected: Option<&Path>, hooks: &[PreviewHook]) {
        let target = selected.map(Path::to_path_buf);

        if target != self.current {
            self.current = target.clone();
            self.content = None;
            self.highlighted = None;
            self.rows.clear();
            self.scroll.reset();
            match target {
                Some(path) => {
                    self.status = PreviewStatus::Loading;
                    self.reveal_started = Some(Instant::now());
                    let hook = text_hook_for(&path, hooks);
                    self.spawn_read(path, hook);
                }
                None => {
                    self.status = PreviewStatus::Empty;
                    self.reveal_started = None;
                }
            }
        }

        while let Ok(ReadOutcome {
            path,
            generation,
            loaded,
            highlighted,
            via_hook,
        }) = self.read_rx.try_recv()
        {
            if Some(&path) != self.current.as_ref() || generation != self.generation {
                continue; // stale — selection moved on, or a newer read superseded this one
            }
            self.rows.clear();
            match loaded {
                Loaded::Failed if via_hook => {
                    self.content = None;
                    self.highlighted = None;
                    self.status = PreviewStatus::Empty;
                }
                Loaded::Failed => {
                    self.content = None;
                    self.highlighted = None;
                    self.status = PreviewStatus::Failed;
                }
                Loaded::Unsupported => {
                    self.content = None;
                    self.highlighted = None;
                    self.status = PreviewStatus::Empty;
                }
                shown => {
                    self.content = Some(shown);
                    self.highlighted = highlighted;
                    self.status = PreviewStatus::Ready;
                }
            }
        }
    }

    fn spawn_read(&mut self, path: PathBuf, hook: Option<PreviewHook>) {
        self.generation += 1;
        let generation = self.generation;
        let tx = self.read_tx.clone();
        let vfs = Arc::clone(&self.vfs);
        self.handle.spawn_blocking(move || {
            // A hook hands the file to another program, which needs it on this machine's disk; on
            // any other backend the file is read directly instead.
            let hook = hook.zip(vfs.local_path(&path));
            let (loaded, highlighted, via_hook) = match hook {
                Some((hook, local)) => match preview_hook::run(&hook, &local) {
                    HookOutcome::Text(text) => {
                        let highlighted = tokenize(&text, &path);
                        (Loaded::Text(text), Some(highlighted), true)
                    }
                    _ => (Loaded::Failed, None, true),
                },
                // Tokenizing runs syntect's line-oriented parser over the whole file, the same
                // amount of work as the read just above it — it belongs off the render thread too.
                None => {
                    let loaded = preview::load(vfs.as_ref(), &path);
                    let highlighted = match &loaded {
                        Loaded::Text(text) => Some(tokenize(text, &path)),
                        _ => None,
                    };
                    (loaded, highlighted, false)
                }
            };
            let _ = tx.send(ReadOutcome {
                path,
                generation,
                loaded,
                highlighted,
                via_hook,
            });
        });
    }
}

/// `path`'s name, tokenized for its syntax (or all `Plain` if none is known — see
/// `preview::highlight::highlight`).
fn tokenize(text: &str, path: &Path) -> Vec<HighlightedLine> {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    preview::highlight::highlight(text, file_name)
}

/// The first configured `Text`-kind hook whose extension matches `path`, if any — the `Image`-
/// kind ones belong to `image_preview` instead.
fn text_hook_for(path: &Path, hooks: &[PreviewHook]) -> Option<PreviewHook> {
    preview_hook::hook_for(hooks, path)
        .filter(|hook| hook.kind == HookKind::Text)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::time::{Duration, Instant};

    proptest! {
        /// However the keys and wheel are used, the offset that gets drawn is a real one: never
        /// past the point where the last row is the last one shown, and zero when everything fits.
        #[test]
        fn the_drawn_offset_is_always_within_the_content(
            moves in proptest::collection::vec(-40isize..40, 0..30),
            total in 0usize..200,
            viewport in 1usize..60,
        ) {
            let mut scroll = Scroll::default();
            for rows in moves {
                scroll.by(rows);
                let offset = scroll.fit(total, viewport);
                prop_assert!(offset <= total.saturating_sub(viewport));
                prop_assert_eq!(offset, scroll.offset());
            }
        }

        /// Scrolling down then up by the same amount, well inside the content, comes back.
        #[test]
        fn down_then_up_returns_to_where_it_started(start in 0usize..50, rows in 0isize..30) {
            let mut scroll = Scroll::default();
            scroll.by(start as isize);
            scroll.fit(1000, 20);
            let before = scroll.offset();
            scroll.by(rows);
            scroll.by(-rows);
            prop_assert_eq!(scroll.offset(), before);
        }
    }

    #[test]
    fn a_key_scrolls_half_the_pane_and_at_least_one_row() {
        let mut scroll = Scroll::default();
        assert_eq!(scroll.half_page(), 1, "before anything is drawn");
        scroll.fit(500, 30);
        assert_eq!(scroll.half_page(), 15);
        scroll.fit(500, 1);
        assert_eq!(scroll.half_page(), 1);
    }

    #[test]
    fn scrolling_up_stops_at_the_top() {
        let mut scroll = Scroll::default();
        scroll.by(-5);
        assert_eq!(scroll.offset(), 0);
        scroll.by(10);
        scroll.by(-100);
        assert_eq!(scroll.offset(), 0);
    }

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-text-preview-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn settle(preview: &mut TextPreview, path: Option<&Path>) {
        settle_with_hooks(preview, path, &[]);
    }

    fn settle_with_hooks(preview: &mut TextPreview, path: Option<&Path>, hooks: &[PreviewHook]) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            preview.update(path, hooks);
            if preview.status() != PreviewStatus::Loading {
                return;
            }
            assert!(Instant::now() < deadline, "the read never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn text_hook(command: &str) -> PreviewHook {
        PreviewHook {
            extensions: vec!["pdf".into()],
            command: command.into(),
            kind: HookKind::Text,
            timeout_ms: None,
        }
    }

    #[test]
    fn a_hook_covered_file_is_shown_as_the_hooks_output() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = scratch("hook-text");
        let source = dir.join("report.pdf");
        std::fs::write(&source, b"%PDF-not-real-content").unwrap();
        let hooks = vec![text_hook("printf 'extracted text' > {out}")];
        let mut preview = TextPreview::new(runtime.handle().clone());

        settle_with_hooks(&mut preview, Some(&source), &hooks);
        assert_eq!(preview.status(), PreviewStatus::Ready);
        assert_eq!(
            preview.content(),
            Some(&Loaded::Text("extracted text".into()))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failing_hook_falls_back_to_empty_not_failed() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = scratch("hook-fail");
        let source = dir.join("report.pdf");
        std::fs::write(&source, b"%PDF-not-real-content").unwrap();
        let hooks = vec![text_hook("false")];
        let mut preview = TextPreview::new(runtime.handle().clone());

        settle_with_hooks(&mut preview, Some(&source), &hooks);
        assert_eq!(
            preview.status(),
            PreviewStatus::Empty,
            "a hook's own failure should fall back quietly, not show \"preview failed\""
        );
        assert!(preview.content().is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn text_bytes_and_a_pipe_each_land_in_their_own_state() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = scratch("kinds");
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        std::fs::write(dir.join("b.bin"), [0u8, 1, 2, 3]).unwrap();
        let fifo = dir.join("pipe");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .is_ok_and(|s| s.success())
        );
        let mut preview = TextPreview::new(runtime.handle().clone());

        settle(&mut preview, Some(&dir.join("a.txt")));
        assert_eq!(preview.status(), PreviewStatus::Ready);
        assert_eq!(preview.content(), Some(&Loaded::Text("hello".into())));

        settle(&mut preview, Some(&dir.join("b.bin")));
        assert_eq!(preview.status(), PreviewStatus::Ready);
        assert!(matches!(preview.content(), Some(Loaded::Bytes(_))));

        settle(&mut preview, Some(&fifo));
        assert_eq!(
            preview.status(),
            PreviewStatus::Empty,
            "a pipe has nothing to show"
        );
        assert!(preview.content().is_none());

        settle(&mut preview, None);
        assert_eq!(preview.status(), PreviewStatus::Empty);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn selecting_another_file_rewinds_the_scroll_but_a_reload_keeps_it() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = scratch("scroll");
        let long = (0..200).map(|i| format!("line {i}\n")).collect::<String>();
        std::fs::write(dir.join("a.txt"), &long).unwrap();
        std::fs::write(dir.join("b.txt"), &long).unwrap();
        let mut preview = TextPreview::new(runtime.handle().clone());

        settle(&mut preview, Some(&dir.join("a.txt")));
        preview.scroll_rows(40);
        assert_eq!(preview.parts().3.fit(200, 20), 40);

        // The file changed on disk while staying selected: same place.
        preview.reload(&[]);
        settle(&mut preview, Some(&dir.join("a.txt")));
        assert_eq!(preview.parts().3.fit(200, 20), 40);

        settle(&mut preview, Some(&dir.join("b.txt")));
        assert_eq!(
            preview.parts().3.fit(200, 20),
            0,
            "a new file starts at the top"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_wrapped_row_count_is_measured_once_per_width_until_cleared() {
        let mut cache = RowCache::default();
        let mut measured = 0;
        let mut ask = |cache: &mut RowCache, width: u16| {
            cache.get(width, || {
                measured += 1;
                width as usize * 2
            })
        };
        assert_eq!(ask(&mut cache, 40), 80);
        assert_eq!(ask(&mut cache, 40), 80);
        assert_eq!(ask(&mut cache, 50), 100, "a new width is measured again");
        cache.clear();
        assert_eq!(ask(&mut cache, 50), 100, "new content is measured again");
        assert_eq!(measured, 3);
    }

    #[test]
    fn a_file_on_another_backend_is_read_through_it() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mem = shared::MemVfs::new();
        mem.add_file("/a.txt", "from memory");
        mem.add_file("/b.bin", vec![0u8, 1, 2, 3]);
        let mut preview = TextPreview::new(runtime.handle().clone()).with_vfs(Arc::new(mem));

        settle(&mut preview, Some(Path::new("/a.txt")));
        assert_eq!(preview.content(), Some(&Loaded::Text("from memory".into())));

        settle(&mut preview, Some(Path::new("/b.bin")));
        assert!(matches!(preview.content(), Some(Loaded::Bytes(_))));
    }

    #[test]
    fn a_hook_is_skipped_for_a_file_that_is_not_on_the_local_disk() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mem = shared::MemVfs::new();
        mem.add_file("/report.pdf", "plain text in a file named like a pdf");
        // A hook that would fail if run; off the local disk it is never handed the file.
        let hooks = vec![text_hook("false")];
        let mut preview = TextPreview::new(runtime.handle().clone()).with_vfs(Arc::new(mem));

        settle_with_hooks(&mut preview, Some(Path::new("/report.pdf")), &hooks);

        assert_eq!(preview.status(), PreviewStatus::Ready);
        assert!(matches!(preview.content(), Some(Loaded::Text(_))));
    }
}
