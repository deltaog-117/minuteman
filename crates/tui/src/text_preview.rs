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

//! The state of the preview column for a file that is not an image or a folder: which file it is
//! about, what the previewing plugin sent for it, and how far it is scrolled. The reading happens
//! in the plugin (`mman-preview`), on its own process and threads, so a file on a slow or
//! network-backed filesystem can never stall input handling just because it is being looked at;
//! this keeps what comes back and answers the interface's questions about it.
//!
//! Despite the name this covers every previewable file that is not an image: text is shown wrapped
//! and colored, an archive as its listing, and anything else as its first bytes for the hex view
//! (see `plugins/preview` for how that is decided). The pane it fills scrolls, and `Scroll` holds
//! where. Scrolling is the interface's, not the plugin's: the plugin sends the whole document once
//! and every key and wheel notch after that is answered here, without a round trip.

use std::path::{Path, PathBuf};
use std::time::Instant;

use plugin_api::{PreviewContent, SetPreview};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStatus {
    /// Nothing selected, or the selection has nothing to show (not a regular file, or no plugin
    /// to preview it).
    Empty,
    /// The plugin has been asked and has not answered yet.
    Loading,
    /// The plugin answered; `content` holds what to show.
    Ready,
    /// The plugin could not read the file, or it is too big to preview as text.
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

/// What to tell the previewing plugin after the interface changed which file the column is about:
/// the new file, or `None` for no file at all (so work in flight for the old one stops).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewAsk(pub Option<PathBuf>);

pub struct TextPreview {
    current: Option<PathBuf>,
    status: PreviewStatus,
    /// Only ever `Text`, `Hex` or `Archive`: a failed or empty answer shows in `status`.
    content: Option<PreviewContent>,
    scroll: Scroll,
    rows: RowCache,
    /// When the current selection's typewriter reveal started, so `preview_view::draw_text` can
    /// compute how many lines have appeared so far (see `anim::revealed_lines`). Set only when
    /// the selection changes to a new file — never by a reload, so a log being appended to while
    /// selected doesn't replay the reveal — and read but never advanced here, since reading it
    /// needs no `&mut self`.
    reveal_started: Option<Instant>,
}

impl Default for TextPreview {
    fn default() -> Self {
        Self::new()
    }
}

impl TextPreview {
    pub fn new() -> Self {
        Self {
            current: None,
            status: PreviewStatus::Empty,
            content: None,
            scroll: Scroll::default(),
            rows: RowCache::default(),
            reveal_started: None,
        }
    }

    /// When the current selection's typewriter reveal started, or `None` if it never started
    /// (nothing selected yet, or this preview was built with [`Self::showing`] for a test).
    pub fn reveal_started(&self) -> Option<Instant> {
        self.reveal_started
    }

    /// Arms the reveal as of `started`, for a test of `preview_view::draw_text`'s capping without
    /// waiting on a real answer.
    #[cfg(test)]
    pub fn arm_reveal_for_test(&mut self, started: Instant) {
        self.reveal_started = Some(started);
    }

    /// A preview already showing `content`, for tests of the code that draws one.
    #[cfg(test)]
    pub fn showing(content: PreviewContent) -> Self {
        let mut preview = Self::new();
        preview.content = Some(content);
        preview.status = PreviewStatus::Ready;
        preview
    }

    /// A preview in `status` with nothing to show, for the same purpose.
    #[cfg(test)]
    pub fn in_status(status: PreviewStatus) -> Self {
        let mut preview = Self::new();
        preview.status = status;
        preview
    }

    pub fn status(&self) -> PreviewStatus {
        self.status
    }

    #[cfg(test)]
    pub fn content(&self) -> Option<&PreviewContent> {
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

    /// The content, the wrapped-row cache and the scroll position as separate borrows, so the
    /// drawing code can read the content while it updates the other two.
    pub fn parts(&mut self) -> (Option<&PreviewContent>, &mut RowCache, &mut Scroll) {
        (self.content.as_ref(), &mut self.rows, &mut self.scroll)
    }

    /// Call once per render tick with the selected file, if it is one that belongs here (not a
    /// folder; the caller decides). `served` is
    /// whether a plugin is there to preview files at all; without one the column shows only the
    /// file's name. Returns what the plugin must be told when the file changed.
    pub fn update(&mut self, selected: Option<&Path>, served: bool) -> Option<PreviewAsk> {
        let target = selected.map(Path::to_path_buf).filter(|_| served);
        if target == self.current {
            return None;
        }
        let was_showing_a_file = self.current.is_some();
        self.current = target.clone();
        self.content = None;
        self.rows.clear();
        self.scroll.reset();
        match target {
            Some(path) => {
                self.status = PreviewStatus::Loading;
                self.reveal_started = Some(Instant::now());
                Some(PreviewAsk(Some(path)))
            }
            None => {
                self.status = PreviewStatus::Empty;
                self.reveal_started = None;
                // Only worth telling the plugin when it may be working on something.
                was_showing_a_file.then_some(PreviewAsk(None))
            }
        }
    }

    /// Asks for the current file again without clearing what is shown, for when it changed on disk
    /// while staying selected. A no-op when nothing previewable is selected. The scroll position
    /// is kept, so a log being appended to does not jump back to the top.
    pub fn reload(&self) -> Option<PreviewAsk> {
        self.current.clone().map(|path| PreviewAsk(Some(path)))
    }

    /// Takes the plugin's document for a file. One for a file the column is no longer about is
    /// dropped: the cursor moved on while the plugin was reading.
    pub fn accept(&mut self, set: SetPreview) {
        if Some(&set.path) != self.current.as_ref() {
            return;
        }
        self.rows.clear();
        match set.content {
            PreviewContent::Failed => {
                self.content = None;
                self.status = PreviewStatus::Failed;
            }
            // A picture is drawn by the image pipeline, which `Previews::accept` has told; this
            // pane has nothing of its own to show for it.
            PreviewContent::Empty | PreviewContent::Image { .. } => {
                self.content = None;
                self.status = PreviewStatus::Empty;
            }
            shown => {
                self.content = Some(shown);
                self.status = PreviewStatus::Ready;
            }
        }
    }

    /// The plugin could not be told (it is not running and cannot be started): stop waiting for a
    /// document that will not come and show the file's name.
    pub fn give_up(&mut self) {
        self.content = None;
        self.status = PreviewStatus::Empty;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugin_api::{HexHead, Token};
    use proptest::prelude::*;

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

    fn document(path: &str, content: PreviewContent) -> SetPreview {
        SetPreview {
            path: PathBuf::from(path),
            content,
        }
    }

    fn text(words: &str) -> PreviewContent {
        PreviewContent::Text {
            lines: vec![vec![(Token::Plain, words.to_string())]],
        }
    }

    #[test]
    fn selecting_a_file_asks_the_plugin_and_waits_for_its_answer() {
        let mut preview = TextPreview::new();
        let ask = preview.update(Some(Path::new("/a.txt")), true);
        assert_eq!(ask, Some(PreviewAsk(Some(PathBuf::from("/a.txt")))));
        assert_eq!(preview.status(), PreviewStatus::Loading);
        assert!(preview.reveal_started().is_some());

        preview.accept(document("/a.txt", text("hello")));
        assert_eq!(preview.status(), PreviewStatus::Ready);
        assert_eq!(preview.content(), Some(&text("hello")));
    }

    #[test]
    fn the_same_file_again_asks_nothing() {
        let mut preview = TextPreview::new();
        preview.update(Some(Path::new("/a.txt")), true);
        assert_eq!(preview.update(Some(Path::new("/a.txt")), true), None);
        assert_eq!(preview.update(Some(Path::new("/a.txt")), true), None);
    }

    #[test]
    fn a_document_for_a_file_the_cursor_left_is_dropped() {
        let mut preview = TextPreview::new();
        preview.update(Some(Path::new("/a.txt")), true);
        preview.update(Some(Path::new("/b.txt")), true);

        preview.accept(document("/a.txt", text("late")));
        assert_eq!(preview.status(), PreviewStatus::Loading);
        assert!(preview.content().is_none());

        preview.accept(document("/b.txt", text("on time")));
        assert_eq!(preview.content(), Some(&text("on time")));
    }

    #[test]
    fn each_kind_of_answer_lands_in_its_own_state() {
        let mut preview = TextPreview::new();
        preview.update(Some(Path::new("/f")), true);

        let hex = PreviewContent::Hex(HexHead {
            bytes: vec![0, 1],
            total: 2,
        });
        preview.accept(document("/f", hex.clone()));
        assert_eq!(
            (preview.status(), preview.content()),
            (PreviewStatus::Ready, Some(&hex))
        );

        preview.accept(document("/f", PreviewContent::Failed));
        assert_eq!(preview.status(), PreviewStatus::Failed);
        assert!(preview.content().is_none());

        preview.accept(document("/f", PreviewContent::Empty));
        assert_eq!(
            preview.status(),
            PreviewStatus::Empty,
            "a pipe, or a file the plugin has nothing for, shows its name"
        );
        assert!(preview.content().is_none());
    }

    #[test]
    fn clearing_the_selection_empties_the_column_and_tells_the_plugin_once() {
        let mut preview = TextPreview::new();
        assert_eq!(
            preview.update(None, true),
            None,
            "nothing was shown, so there is nothing to cancel"
        );
        preview.update(Some(Path::new("/a")), true);
        assert_eq!(preview.update(None, true), Some(PreviewAsk(None)));
        assert_eq!(preview.status(), PreviewStatus::Empty);
        assert!(preview.reveal_started().is_none());
        assert_eq!(preview.update(None, true), None);
    }

    #[test]
    fn without_a_plugin_the_column_shows_only_the_name_and_asks_nobody() {
        let mut preview = TextPreview::new();
        assert_eq!(preview.update(Some(Path::new("/a.txt")), false), None);
        assert_eq!(preview.status(), PreviewStatus::Empty);
    }

    #[test]
    fn a_plugin_that_cannot_be_reached_stops_the_wait() {
        let mut preview = TextPreview::new();
        preview.update(Some(Path::new("/a.txt")), true);
        preview.give_up();
        assert_eq!(preview.status(), PreviewStatus::Empty);
    }

    #[test]
    fn selecting_another_file_rewinds_the_scroll_but_a_reload_keeps_it() {
        let long: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let mut preview = TextPreview::new();
        preview.update(Some(Path::new("/a.txt")), true);
        preview.accept(document("/a.txt", text(&long)));
        preview.scroll_rows(40);
        assert_eq!(preview.parts().2.fit(200, 20), 40);

        // The file changed on disk while staying selected: the plugin is asked again, and the
        // new document lands in the same place.
        assert_eq!(
            preview.reload(),
            Some(PreviewAsk(Some(PathBuf::from("/a.txt"))))
        );
        preview.accept(document("/a.txt", text(&long)));
        assert_eq!(preview.parts().2.fit(200, 20), 40);

        preview.update(Some(Path::new("/b.txt")), true);
        preview.accept(document("/b.txt", text(&long)));
        assert_eq!(
            preview.parts().2.fit(200, 20),
            0,
            "a new file starts at the top"
        );
    }

    #[test]
    fn a_reload_with_nothing_selected_asks_nothing() {
        assert_eq!(TextPreview::new().reload(), None);
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
}
