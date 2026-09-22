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

//! Renders the selected file as an inline image (Kitty/iTerm2/Sixel, falling back to Unicode
//! halfblocks) via `ratatui-image`, without ever blocking the render loop.
//!
//! `ratatui-image`'s own docs are explicit that its adaptive `StatefulImage` widget "will block
//! the UI thread" unless paired with `ThreadProtocol` — its request/response channel design for
//! offloading resize+encode work. This module wires that up on `tokio::runtime::Handle::
//! spawn_blocking`, the same pattern `tui::app::App` already uses for bulk file operations:
//! decoding the file and (later) resizing/encoding it for the terminal both happen off-thread,
//! with results drained once per render tick via [`ImagePreview::update`].

use std::path::{Path, PathBuf};

use ratatui_image::picker::cap_parser::QueryStdioOptions;
use ratatui_image::picker::{Capability, Picker};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::thread::{ResizeRequest, ResizeResponse, ThreadProtocol};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStatus {
    /// Nothing selected, or the selection isn't an image.
    Empty,
    /// The file is being decoded off-thread.
    Loading,
    /// Decoded; `protocol_mut` renders the image (possibly still resizing for the first time).
    Ready,
    /// Decode failed — not a valid/supported image despite the extension.
    Failed,
}

enum DecodeOutcome {
    Decoded {
        path: PathBuf,
        generation: u64,
        protocol: Box<StatefulProtocol>,
    },
    Failed {
        path: PathBuf,
        generation: u64,
    },
}

pub struct ImagePreview {
    picker: Picker,
    /// The terminal's own background color, from the same stdio round-trip `Picker` already runs
    /// at startup (an OSC 11 query bundled in alongside the graphics-capability probe). `None`
    /// when the terminal never answered. `theme::auto` (see `main`) is the only reader.
    detected_background: Option<(u8, u8, u8)>,
    protocol: ThreadProtocol,
    current: Option<PathBuf>,
    status: PreviewStatus,
    /// Counts decodes started; only the latest one's result is kept (see `TextPreview`).
    generation: u64,
    decode_tx: UnboundedSender<DecodeOutcome>,
    decode_rx: UnboundedReceiver<DecodeOutcome>,
    resize_request_rx: UnboundedReceiver<ResizeRequest>,
    resize_response_tx: UnboundedSender<ResizeResponse>,
    resize_response_rx: UnboundedReceiver<ResizeResponse>,
    handle: tokio::runtime::Handle,
}

impl ImagePreview {
    /// Queries the terminal for graphics-protocol support (Kitty/iTerm2/Sixel) and its background
    /// color (OSC 11, for `theme::auto`), falling back to halfblocks and no detected background
    /// if the terminal never answers or a real error occurs — a typo'd or unusual terminal must
    /// never block startup, the same rule `Config::load` and `Theme::named` already follow for
    /// their own fallbacks.
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        let picker = Picker::from_query_stdio_with_options(QueryStdioOptions {
            terminal_background_color_osc: true,
            ..QueryStdioOptions::default()
        })
        .unwrap_or_else(|_| Picker::halfblocks());
        let detected_background = picker.capabilities().iter().find_map(|cap| match cap {
            Capability::Background(r, g, b) => Some((*r, *g, *b)),
            _ => None,
        });
        let (resize_request_tx, resize_request_rx) = unbounded_channel();
        let (resize_response_tx, resize_response_rx) = unbounded_channel();
        let (decode_tx, decode_rx) = unbounded_channel();
        Self {
            picker,
            detected_background,
            protocol: ThreadProtocol::new(resize_request_tx, None),
            current: None,
            status: PreviewStatus::Empty,
            generation: 0,
            decode_tx,
            decode_rx,
            resize_request_rx,
            resize_response_tx,
            resize_response_rx,
            handle,
        }
    }

    pub fn status(&self) -> PreviewStatus {
        self.status
    }

    /// The terminal's background color as reported by the OSC 11 query `new` already ran, or
    /// `None` if it never answered.
    pub fn detected_background(&self) -> Option<(u8, u8, u8)> {
        self.detected_background
    }

    pub fn protocol_mut(&mut self) -> &mut ThreadProtocol {
        &mut self.protocol
    }

    /// Decodes the current image again without clearing what is shown, for when it changed on
    /// disk while staying selected. A no-op when no image is selected.
    pub fn reload(&mut self) {
        if let Some(path) = self.current.clone() {
            self.spawn_decode(path);
        }
    }

    /// Call once per render tick. Starts decoding `selected` if it's a new image, and drains any
    /// completed decode/resize work from the background threads.
    pub fn update(&mut self, selected: Option<&Path>) {
        let target = selected
            .filter(|path| preview::is_image(path))
            .map(Path::to_path_buf);

        if target != self.current {
            self.current = target.clone();
            self.protocol.empty_protocol();
            match target {
                Some(path) => {
                    self.status = PreviewStatus::Loading;
                    self.spawn_decode(path);
                }
                None => self.status = PreviewStatus::Empty,
            }
        }

        while let Ok(outcome) = self.decode_rx.try_recv() {
            let (path, generation) = match &outcome {
                DecodeOutcome::Decoded {
                    path, generation, ..
                }
                | DecodeOutcome::Failed { path, generation } => (path, *generation),
            };
            if Some(path) != self.current.as_ref() || generation != self.generation {
                continue; // stale — selection moved on, or a newer decode superseded this one
            }
            match outcome {
                DecodeOutcome::Decoded { protocol, .. } => {
                    self.protocol.replace_protocol(*protocol);
                    self.status = PreviewStatus::Ready;
                }
                DecodeOutcome::Failed { .. } => self.status = PreviewStatus::Failed,
            }
        }

        while let Ok(request) = self.resize_request_rx.try_recv() {
            let tx = self.resize_response_tx.clone();
            self.handle.spawn_blocking(move || {
                if let Ok(response) = request.resize_encode() {
                    let _ = tx.send(response);
                }
            });
        }

        while let Ok(response) = self.resize_response_rx.try_recv() {
            self.protocol.update_resized_protocol(response);
        }
    }

    fn spawn_decode(&mut self, path: PathBuf) {
        self.generation += 1;
        let generation = self.generation;
        let tx = self.decode_tx.clone();
        let picker = self.picker.clone();
        self.handle.spawn_blocking(move || {
            let outcome = match preview::load_image(&path) {
                Some(image) => DecodeOutcome::Decoded {
                    protocol: Box::new(picker.new_resize_protocol(image)),
                    path,
                    generation,
                },
                None => DecodeOutcome::Failed { path, generation },
            };
            let _ = tx.send(outcome);
        });
    }
}
