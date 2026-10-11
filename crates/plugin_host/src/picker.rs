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

//! A plugin's picker and the views it fills, as the render thread sees them. Like an action, a picker belongs to state only
//! the render thread holds (the popup, the keys), so the reader that gets a plugin's `show_picker`
//! hands it over as a `PickerRequest` and waits for the answer, and the plugin's `picker_items`
//! and `picker_close` notifications are queued behind it in order. The render thread takes them
//! with `PluginManager::take_ui` once per turn.
//!
//! This crate does not draw anything: it only carries what the plugin said to the interface, and
//! what the interface said back (see `PluginManager::send_picker_query`).

use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::protocol::{PickerSpec, PickerUpdate, SetPreview, SetView};

type Reply = Result<Value, String>;

/// A plugin's request to open its picker, waiting on the render thread. Answer it with `opened` or
/// `refuse`; dropping it unanswered tells the plugin the interface closed first.
#[derive(Debug)]
pub struct PickerRequest {
    plugin: String,
    spec: PickerSpec,
    reply: oneshot::Sender<Reply>,
}

impl PickerRequest {
    /// The plugin that asked.
    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    pub fn spec(&self) -> &PickerSpec {
        &self.spec
    }

    /// Tells the plugin its picker is open.
    pub fn opened(self) {
        let _ = self.reply.send(Ok(json!({ "picker": self.spec.picker })));
    }

    /// Tells the plugin its picker did not open, and why.
    pub fn refuse(self, reason: &str) {
        let _ = self
            .reply
            .send(Err(format!("cannot open the picker: {reason}")));
    }
}

/// Something a plugin asked of the interface that is not an action.
#[derive(Debug)]
pub enum UiRequest {
    ShowPicker(PickerRequest),
    /// The plugin replaced what its picker lists.
    Items {
        plugin: String,
        update: PickerUpdate,
    },
    /// The plugin closed its picker.
    Close {
        plugin: String,
        picker: u64,
    },
    /// The plugin filled or emptied a slot of the interface.
    SetView {
        plugin: String,
        set: SetView,
    },
    /// The plugin gave the preview column a document.
    SetPreview {
        plugin: String,
        set: SetPreview,
    },
}

/// How a plugin's reader reaches the render thread for pickers, shared by every plugin.
#[derive(Debug, Clone)]
pub(crate) struct UiLink {
    tx: UnboundedSender<UiRequest>,
}

impl UiLink {
    pub(crate) fn new(tx: UnboundedSender<UiRequest>) -> Self {
        Self { tx }
    }

    /// Hands `spec` to the render thread and waits for its answer, as `ActionLink::run` does.
    pub(crate) async fn show_picker(&self, plugin: &str, spec: PickerSpec) -> Reply {
        let (reply, answer) = oneshot::channel();
        let request = PickerRequest {
            plugin: plugin.to_string(),
            spec,
            reply,
        };
        if self.tx.send(UiRequest::ShowPicker(request)).is_err() {
            return Err("the interface has closed".to_string());
        }
        answer
            .await
            .unwrap_or_else(|_| Err("the interface closed before opening it".to_string()))
    }

    pub(crate) fn items(&self, plugin: &str, update: PickerUpdate) {
        let _ = self.tx.send(UiRequest::Items {
            plugin: plugin.to_string(),
            update,
        });
    }

    pub(crate) fn set_view(&self, plugin: &str, set: SetView) {
        let _ = self.tx.send(UiRequest::SetView {
            plugin: plugin.to_string(),
            set,
        });
    }

    pub(crate) fn set_preview(&self, plugin: &str, set: SetPreview) {
        let _ = self.tx.send(UiRequest::SetPreview {
            plugin: plugin.to_string(),
            set,
        });
    }

    pub(crate) fn close(&self, plugin: &str, picker: u64) {
        let _ = self.tx.send(UiRequest::Close {
            plugin: plugin.to_string(),
            picker,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime builds with no I/O drivers")
    }

    fn spec(picker: u64) -> PickerSpec {
        PickerSpec {
            picker,
            title: "find".to_string(),
            preview: true,
        }
    }

    #[test]
    fn an_opened_picker_is_acknowledged_with_its_number() {
        let (tx, mut rx) = unbounded_channel();
        let link = UiLink::new(tx);
        let reply = runtime().block_on(async {
            let waiting = link.show_picker("p", spec(7));
            let answer = async {
                let Some(UiRequest::ShowPicker(request)) = rx.recv().await else {
                    panic!("the request was sent");
                };
                assert_eq!((request.plugin(), request.spec().picker), ("p", 7));
                request.opened();
            };
            tokio::join!(waiting, answer).0
        });
        assert_eq!(reply, Ok(json!({ "picker": 7 })));
    }

    #[test]
    fn a_refusal_carries_the_reason() {
        let (tx, mut rx) = unbounded_channel();
        let link = UiLink::new(tx);
        let reply = runtime().block_on(async {
            let waiting = link.show_picker("p", spec(1));
            let answer = async {
                let Some(UiRequest::ShowPicker(request)) = rx.recv().await else {
                    panic!("the request was sent");
                };
                request.refuse("another picker is open");
            };
            tokio::join!(waiting, answer).0
        });
        assert_eq!(
            reply,
            Err("cannot open the picker: another picker is open".to_string())
        );
    }

    #[test]
    fn a_request_nobody_answers_or_takes_is_an_error_not_a_hang() {
        let (tx, mut rx) = unbounded_channel();
        let link = UiLink::new(tx);
        let dropped = runtime().block_on(async {
            let waiting = link.show_picker("p", spec(1));
            let drop_it = async { drop(rx.recv().await) };
            tokio::join!(waiting, drop_it).0
        });
        assert!(dropped.is_err());

        drop(rx);
        let closed = runtime().block_on(link.show_picker("p", spec(1)));
        assert_eq!(closed, Err("the interface has closed".to_string()));
    }

    #[test]
    fn items_and_closes_arrive_in_the_order_they_were_sent() {
        let (tx, mut rx) = unbounded_channel();
        let link = UiLink::new(tx);
        link.close("p", 1);
        link.close("p", 2);
        let numbers: Vec<u64> = std::iter::from_fn(|| rx.try_recv().ok())
            .map(|request| match request {
                UiRequest::Close { picker, .. } => picker,
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(numbers, [1, 2]);
    }
}
