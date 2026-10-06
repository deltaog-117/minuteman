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

//! A plugin's `run_action` and `list_actions`: running one of the interface's own actions
//! (`paste`, `sort`, ...) by its stable name. Unlike a file operation, an action works on state only
//! the render thread holds (the browser, the open popups, the clipboard), so the reader that got
//! the request hands it over as an `ActionRequest` and waits; the render thread takes it on its
//! next turn, runs it or refuses it, and the reply goes back to the plugin from there.
//!
//! This crate does not know the actions themselves: the interface declares their names (see
//! `PluginManager::declare_actions`) and decides what each name does.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

/// What a reader replies to the plugin with: a result or the reason there is none.
type Reply = Result<Value, String>;

/// One plugin's request to run an action, waiting on the render thread. Answer it with `ran` or
/// `refuse`; dropping it unanswered tells the plugin the interface closed first.
#[derive(Debug)]
pub struct ActionRequest {
    plugin: String,
    name: String,
    reply: oneshot::Sender<Reply>,
}

impl ActionRequest {
    /// The plugin that asked.
    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    /// The action it asked for, as sent: not yet known to name a real one.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Tells the plugin the action ran.
    pub fn ran(self) {
        let _ = self.reply.send(Ok(json!({ "ran": self.name })));
    }

    /// Tells the plugin the action did not run, and why.
    pub fn refuse(self, reason: &str) {
        let _ = self
            .reply
            .send(Err(format!("cannot run '{}': {reason}", self.name)));
    }
}

/// How a plugin's reader reaches the render thread for actions, shared by every plugin.
#[derive(Debug, Clone)]
pub(crate) struct ActionLink {
    tx: UnboundedSender<ActionRequest>,
    /// The names the interface declared, for `list_actions`.
    names: Arc<[String]>,
}

impl ActionLink {
    pub(crate) fn new(tx: UnboundedSender<ActionRequest>) -> Self {
        Self {
            tx,
            names: Arc::from([]),
        }
    }

    pub(crate) fn with_names(&self, names: Vec<String>) -> Self {
        Self {
            tx: self.tx.clone(),
            names: Arc::from(names),
        }
    }

    pub(crate) fn list(&self) -> Value {
        json!(*self.names)
    }

    /// Hands `name` to the render thread and waits for its answer. The reader that calls this
    /// reads nothing more from its plugin meanwhile, the same as while a file operation runs.
    pub(crate) async fn run(&self, plugin: &str, name: String) -> Reply {
        let (reply, answer) = oneshot::channel();
        let request = ActionRequest {
            plugin: plugin.to_string(),
            name,
            reply,
        };
        if self.tx.send(request).is_err() {
            return Err("the interface has closed".to_string());
        }
        answer
            .await
            .unwrap_or_else(|_| Err("the interface closed before running it".to_string()))
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

    #[test]
    fn a_request_reaches_the_other_end_and_its_answer_comes_back() {
        let (tx, mut rx) = unbounded_channel();
        let link = ActionLink::new(tx);
        let reply = runtime().block_on(async {
            let waiting = link.run("p", "paste".to_string());
            let answer = async {
                let request = rx.recv().await.expect("the request was sent");
                assert_eq!((request.plugin(), request.name()), ("p", "paste"));
                request.ran();
            };
            tokio::join!(waiting, answer).0
        });
        assert_eq!(reply, Ok(json!({ "ran": "paste" })));
    }

    #[test]
    fn a_refusal_names_the_action_and_the_reason() {
        let (tx, mut rx) = unbounded_channel();
        let link = ActionLink::new(tx);
        let reply = runtime().block_on(async {
            let waiting = link.run("p", "sort".to_string());
            let answer = async {
                rx.recv()
                    .await
                    .expect("the request was sent")
                    .refuse("a prompt is open");
            };
            tokio::join!(waiting, answer).0
        });
        assert_eq!(
            reply,
            Err("cannot run 'sort': a prompt is open".to_string())
        );
    }

    #[test]
    fn a_request_dropped_unanswered_or_never_taken_is_an_error_not_a_hang() {
        let (tx, mut rx) = unbounded_channel();
        let link = ActionLink::new(tx);
        let dropped = runtime().block_on(async {
            let waiting = link.run("p", "undo".to_string());
            let drop_it = async { drop(rx.recv().await) };
            tokio::join!(waiting, drop_it).0
        });
        assert!(dropped.is_err());

        drop(rx);
        let closed = runtime().block_on(link.run("p", "undo".to_string()));
        assert_eq!(closed, Err("the interface has closed".to_string()));
    }

    #[test]
    fn the_listed_names_are_the_declared_ones_in_order() {
        let (tx, _rx) = unbounded_channel();
        let link = ActionLink::new(tx);
        assert_eq!(link.list(), json!([]));
        let named = link.with_names(vec!["paste".into(), "sort".into()]);
        assert_eq!(named.list(), json!(["paste", "sort"]));
    }
}
