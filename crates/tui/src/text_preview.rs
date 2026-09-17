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

//! Reads the selected file as UTF-8 text off the render thread, the same
//! never-block-the-render-loop treatment `image_preview` gives image decoding: a file on a slow
//! or network-backed filesystem must not stall input handling just because it happens to be text.

use std::path::{Path, PathBuf};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStatus {
    /// Nothing selected, or the selection isn't previewable as text.
    Empty,
    /// The file is being read off-thread.
    Loading,
    /// Read successfully; `content` holds the file's text.
    Ready,
    /// Read failed — too large, binary, or not valid UTF-8 despite the text-like name.
    Failed,
}

enum ReadOutcome {
    Read { path: PathBuf, content: String },
    Failed { path: PathBuf },
}

pub struct TextPreview {
    current: Option<PathBuf>,
    status: PreviewStatus,
    content: String,
    read_tx: UnboundedSender<ReadOutcome>,
    read_rx: UnboundedReceiver<ReadOutcome>,
    handle: tokio::runtime::Handle,
}

impl TextPreview {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        let (read_tx, read_rx) = unbounded_channel();
        Self {
            current: None,
            status: PreviewStatus::Empty,
            content: String::new(),
            read_tx,
            read_rx,
            handle,
        }
    }

    pub fn status(&self) -> PreviewStatus {
        self.status
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    /// Call once per render tick. Starts reading `selected` if it's a new text file, and drains
    /// any completed read from the background thread.
    pub fn update(&mut self, selected: Option<&Path>) {
        let target = selected
            .filter(|path| preview::is_text(path))
            .map(Path::to_path_buf);

        if target != self.current {
            self.current = target.clone();
            self.content.clear();
            match target {
                Some(path) => {
                    self.status = PreviewStatus::Loading;
                    self.spawn_read(path);
                }
                None => self.status = PreviewStatus::Empty,
            }
        }

        while let Ok(outcome) = self.read_rx.try_recv() {
            let path = match &outcome {
                ReadOutcome::Read { path, .. } | ReadOutcome::Failed { path } => path,
            };
            if Some(path) != self.current.as_ref() {
                continue; // stale — selection moved on before this read finished
            }
            match outcome {
                ReadOutcome::Read { content, .. } => {
                    self.content = content;
                    self.status = PreviewStatus::Ready;
                }
                ReadOutcome::Failed { .. } => self.status = PreviewStatus::Failed,
            }
        }
    }

    fn spawn_read(&self, path: PathBuf) {
        let tx = self.read_tx.clone();
        self.handle.spawn_blocking(move || {
            let outcome = match preview::load_text(&path) {
                Some(content) => ReadOutcome::Read { path, content },
                None => ReadOutcome::Failed { path },
            };
            let _ = tx.send(outcome);
        });
    }
}
