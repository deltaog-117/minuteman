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
//! halfblocks) via `ratatui-image`, without ever blocking the render loop. Which files are
//! pictures is the previewing plugin's call: it answers a `preview` event with an image document
//! (see `plugin_api::ImageSource`), and this module decodes, fits and draws what that names.
//!
//! `ratatui-image`'s own docs are explicit that its adaptive `StatefulImage` widget "will block
//! the UI thread" unless paired with `ThreadProtocol` — its request/response channel design for
//! offloading resize+encode work. This module wires that up on `tokio::runtime::Handle::
//! spawn_blocking`, the same pattern `tui::app::App` already uses for bulk file operations:
//! decoding the file and (later) resizing/encoding it for the terminal both happen off-thread,
//! with results drained once per render tick via [`ImagePreview::update`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use plugin_api::{ImageSource, is_handoff_path};
use ratatui_image::picker::cap_parser::QueryStdioOptions;
use ratatui_image::picker::{Capability, Picker};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::thread::{ResizeRequest, ResizeResponse, ThreadProtocol};
use shared::{FileKind, LocalVfs, Vfs};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStatus {
    /// Nothing selected, or the plugin has not said the selection is a picture — or it said so
    /// for a picture its hook made and that could not be read, which falls back to showing just
    /// the file's name quietly rather than an error here.
    Empty,
    /// The picture is being decoded off-thread.
    Loading,
    /// Decoded; `protocol_mut` renders the image (possibly still resizing for the first time).
    Ready,
    /// A real image file failed to decode — not a valid/supported image despite the extension.
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
        /// Whether the picture was one the plugin made rather than the file itself — see
        /// `PreviewStatus::Empty` vs `PreviewStatus::Failed`.
        handed_over: bool,
    },
}

pub struct ImagePreview {
    picker: Picker,
    /// The terminal's own background color, from the same stdio round-trip `Picker` already runs
    /// at startup (an OSC 11 query bundled in alongside the graphics-capability probe). `None`
    /// when the terminal never answered. `theme::auto` (see `main`) is the only reader.
    detected_background: Option<(u8, u8, u8)>,
    protocol: ThreadProtocol,
    /// The file the preview column is about, whether or not the plugin calls it a picture.
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
    /// Where the images being previewed are read from.
    vfs: Arc<dyn Vfs>,
}

impl ImagePreview {
    /// Reads the previewed images through `vfs` instead of the local disk.
    pub fn with_vfs(mut self, vfs: Arc<dyn Vfs>) -> Self {
        self.vfs = vfs;
        self
    }

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
            vfs: Arc::new(LocalVfs),
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

    /// The terminal's graphics capabilities, found once at startup, for the thumbnail cache to
    /// encode with the same protocol the preview uses.
    pub fn picker(&self) -> Picker {
        self.picker.clone()
    }

    pub fn protocol_mut(&mut self) -> &mut ThreadProtocol {
        &mut self.protocol
    }

    /// Call once per render tick with the file the preview column is about. A different file
    /// clears what was shown; whether the new one is a picture is up to the plugin's answer
    /// ([`Self::show`]). Also drains any completed decode/resize work from the background threads.
    pub fn update(&mut self, selected: Option<&Path>) {
        if selected != self.current.as_deref() {
            self.current = selected.map(Path::to_path_buf);
            self.protocol.empty_protocol();
            self.status = PreviewStatus::Empty;
        }

        while let Ok(outcome) = self.decode_rx.try_recv() {
            let (path, generation) = match &outcome {
                DecodeOutcome::Decoded {
                    path, generation, ..
                }
                | DecodeOutcome::Failed {
                    path, generation, ..
                } => (path, *generation),
            };
            if Some(path) != self.current.as_ref() || generation != self.generation {
                continue; // stale — selection moved on, or a newer decode superseded this one
            }
            match outcome {
                DecodeOutcome::Decoded { protocol, .. } => {
                    self.protocol.replace_protocol(*protocol);
                    self.status = PreviewStatus::Ready;
                }
                DecodeOutcome::Failed { handed_over, .. } => {
                    self.status = if handed_over {
                        PreviewStatus::Empty
                    } else {
                        PreviewStatus::Failed
                    };
                }
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

    /// The plugin says the file at `path` is a picture, read as `source` says. An answer for a
    /// file the column has left is dropped (and a handed-over file deleted, since nothing else
    /// will). The picture already on screen stays until the new decode lands, so a file edited
    /// in place does not flash "loading preview…".
    pub fn show(&mut self, path: &Path, source: ImageSource) {
        if self.current.as_deref() != Some(path) {
            discard_handoff(&source);
            return;
        }
        if self.status != PreviewStatus::Ready {
            self.status = PreviewStatus::Loading;
        }
        self.spawn_decode(path.to_path_buf(), source);
    }

    /// The plugin says the file at `path` is not a picture (any more): stop showing one.
    pub fn forget(&mut self, path: &Path) {
        if self.current.as_deref() == Some(path) {
            self.protocol.empty_protocol();
            self.status = PreviewStatus::Empty;
        }
    }

    fn spawn_decode(&mut self, path: PathBuf, source: ImageSource) {
        self.generation += 1;
        let generation = self.generation;
        let tx = self.decode_tx.clone();
        let picker = self.picker.clone();
        let vfs = Arc::clone(&self.vfs);
        self.handle.spawn_blocking(move || {
            let handed_over = matches!(source, ImageSource::Handoff { .. });
            let image = match &source {
                ImageSource::Original => preview::load_image(vfs.as_ref(), &path),
                ImageSource::Handoff { file } => load_handoff(file),
            };
            // Whatever came of it, the scratch file is ours to delete now.
            discard_handoff(&source);
            let outcome = match image {
                Some(image) => DecodeOutcome::Decoded {
                    protocol: Box::new(picker.new_resize_protocol(image)),
                    path,
                    generation,
                },
                None => DecodeOutcome::Failed {
                    path,
                    generation,
                    handed_over,
                },
            };
            let _ = tx.send(outcome);
        });
    }
}

/// Decodes a picture a plugin handed over. The plugin is not trusted to name an arbitrary file,
/// since the interface deletes it afterwards: it must be a plain file (not a link) under the
/// scratch name `plugin_api` defines, however the document reached here.
fn load_handoff(file: &Path) -> Option<image::DynamicImage> {
    if !is_handoff_path(file) || LocalVfs.symlink_metadata(file).ok()?.kind != FileKind::File {
        return None;
    }
    preview::load_image(&LocalVfs, file)
}

/// Deletes the scratch file `source` names, if it names one that a plugin may hand over. Never
/// follows a link: `remove_file` removes the link itself.
fn discard_handoff(source: &ImageSource) {
    if let ImageSource::Handoff { file } = source
        && is_handoff_path(file)
    {
        let _ = LocalVfs.remove_file(file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugin_api::handoff_path;

    fn write_png(path: &Path) {
        image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3]))
            .save_with_format(path, image::ImageFormat::Png)
            .expect("the scratch folder is writable");
    }

    #[test]
    fn a_handed_over_picture_is_decoded_and_then_deleted() {
        let file = handoff_path(u64::MAX - 1);
        write_png(&file);
        let source = ImageSource::Handoff { file: file.clone() };
        assert!(load_handoff(&file).is_some());
        discard_handoff(&source);
        assert!(!file.exists());
    }

    #[test]
    fn a_file_that_is_not_a_scratch_picture_is_neither_read_nor_deleted() {
        let dir = std::env::temp_dir().join(format!("minuteman-image-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let precious = dir.join("holiday.png");
        write_png(&precious);

        assert!(load_handoff(&precious).is_none());
        discard_handoff(&ImageSource::Handoff {
            file: precious.clone(),
        });
        assert!(
            precious.exists(),
            "a name outside the scratch pattern is never deleted"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_link_under_the_scratch_name_is_refused_and_only_the_link_goes() {
        let dir = std::env::temp_dir().join(format!("minuteman-image-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let precious = dir.join("holiday.png");
        write_png(&precious);
        let link = handoff_path(u64::MAX - 2);
        std::os::unix::fs::symlink(&precious, &link).unwrap();

        assert!(load_handoff(&link).is_none(), "a link is not followed");
        discard_handoff(&ImageSource::Handoff { file: link.clone() });
        assert!(
            std::fs::symlink_metadata(&link).is_err(),
            "the link is gone"
        );
        assert!(precious.exists(), "what it pointed at is not");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
