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

//! Spawns and drives plugin processes: one `tokio` task writes a plugin's stdin from an unbounded
//! queue (so firing an event never blocks the caller), a second reads its stdout line by line and
//! either runs a `file_ops` call on the blocking pool and replies, or forwards a `log` line to
//! `PluginManager::poll`, and a third waits on the process so an abnormal exit gets reported and
//! `kill_on_drop` has something alive to act on for the plugin's whole lifetime. A `get_state`
//! request is answered by that second task straight from a `watch` channel the render loop
//! publishes to each tick, so it never waits on the render thread.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crossterm::event::KeyCode;
use serde_json::json;
use shared::{LocalVfs, Vfs};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::watch;

use crate::error::PluginError;
use crate::protocol::{self, Incoming, LogLevel, PluginRequest, StateSnapshot, WireEntry};

/// One line to show the user, from a plugin's own `log` notification or the host's diagnostics
/// about it (a malformed line, an abnormal exit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogMessage {
    pub plugin: String,
    pub level: LogLevel,
    pub message: String,
}

struct RunningPlugin {
    name: String,
    on_keys: Vec<KeyCode>,
    commands: Vec<String>,
    stdin_tx: UnboundedSender<String>,
}

/// What a plugin is configured to answer: the keys that fire it and the `:` command words it
/// declared. Both may be empty, for a plugin only ever started by `:plugin <name>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Triggers {
    pub keys: Vec<KeyCode>,
    pub commands: Vec<String>,
}

/// Owns every configured plugin process for the running session: spawns them, fires key and
/// lifecycle events at them, and runs the `file_ops` calls they send back. Every plugin talks the
/// same versioned, line-delimited JSON-RPC protocol (see `protocol`) over its own stdin/stdout, so
/// it can be written in any language that can read a line and print one — no host-side bindings
/// per language, no compile step for the plugin itself.
///
/// Deliberately unsandboxed for this first stage: a plugin runs with the same OS permissions as
/// Minuteman itself. See the roadmap's Long-Term Vision entry on a WASM/Extism host for the
/// sandboxed tier this is designed to sit alongside later, not be replaced by.
pub struct PluginManager {
    plugins: Vec<RunningPlugin>,
    logs_tx: UnboundedSender<LogMessage>,
    logs_rx: UnboundedReceiver<LogMessage>,
    /// The latest browser state, shared with every plugin's reader (see `publish_state`).
    state_tx: watch::Sender<StateSnapshot>,
}

impl Default for PluginManager {
    fn default() -> Self {
        let (logs_tx, logs_rx) = unbounded_channel();
        Self {
            plugins: Vec::new(),
            logs_tx,
            logs_rx,
            state_tx: watch::channel(StateSnapshot::default()).0,
        }
    }
}

impl PluginManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts one plugin process, fires its `init` event, and registers it to fire `key` on each
    /// of `triggers.keys` and `command` on each of `triggers.commands`. A failure to spawn (bad command, missing binary) is returned rather
    /// than panicking — the caller decides whether that is fatal or just a warning, the same
    /// "one bad entry must never take the whole session down" rule `theming::Config::load`
    /// already follows for a malformed config file.
    pub fn spawn(
        &mut self,
        handle: &tokio::runtime::Handle,
        name: &str,
        command: &str,
        args: &[String],
        triggers: Triggers,
        cwd: &Path,
    ) -> Result<(), PluginError> {
        let mut child = {
            // `Command::spawn` registers the child with the calling thread's tokio reactor, so
            // it needs `handle` bound as current for the duration of the call — this function
            // itself runs outside any async context (it is called once at startup, synchronously,
            // by `tui::App::with_plugins`).
            let _guard = handle.enter();
            Command::new(command)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|source| PluginError::Spawn {
                    name: name.to_string(),
                    command: command.to_string(),
                    source,
                })?
        };

        let stdin = child.stdin.take().expect("spawned with a piped stdin");
        let stdout = child.stdout.take().expect("spawned with a piped stdout");
        let (stdin_tx, stdin_rx) = unbounded_channel::<String>();

        handle.spawn(write_loop(stdin, stdin_rx));
        handle.spawn(read_loop(
            name.to_string(),
            stdout,
            stdin_tx.clone(),
            self.logs_tx.clone(),
            self.state_tx.subscribe(),
            handle.clone(),
        ));
        handle.spawn(reap(name.to_string(), child, self.logs_tx.clone()));

        // A plugin may ask for the state in answer to `init`, before the first tick has published
        // one, so until then the answer names at least the directory the session started in.
        self.state_tx.send_if_modified(|state| {
            let unset = state.cwd.as_os_str().is_empty();
            if unset {
                state.cwd = cwd.to_path_buf();
            }
            unset
        });

        let init = protocol::HostEvent::new(protocol::EVENT_INIT, cwd.to_path_buf(), Vec::new());
        let _ = stdin_tx.send(init.to_line());

        self.plugins.push(RunningPlugin {
            name: name.to_string(),
            on_keys: triggers.keys,
            commands: triggers.commands,
            stdin_tx,
        });
        Ok(())
    }

    /// Fires the plugin bound to `code`, if any, with the browsed directory and the current
    /// selection (marks, or the single entry under the cursor — the same batch every built-in
    /// bulk action uses), and which key it was. When two plugins claim a key the one configured
    /// first gets it. Returns whether a plugin consumed the key, so the caller can tell a truly
    /// unbound key apart from one a plugin handled.
    pub fn dispatch_key(&self, code: KeyCode, cwd: &Path, selection: &[PathBuf]) -> bool {
        let Some(plugin) = self.plugins.iter().find(|p| p.on_keys.contains(&code)) else {
            return false;
        };
        let event =
            protocol::HostEvent::new(protocol::EVENT_KEY, cwd.to_path_buf(), selection.to_vec())
                .with_key(code);
        // Sending into a channel whose reader already ended (the process died) is a silent
        // no-op — the plugin is gone, so there is nothing left to notify.
        let _ = plugin.stdin_tx.send(event.to_line());
        true
    }

    /// Fires `command` at the plugin that declared that word (first configured wins), with the
    /// same directory and selection a key event carries plus the arguments. Returns whether a
    /// plugin answered, so the caller can fall through to running the line as a shell command.
    pub fn dispatch_command(
        &self,
        command: &str,
        args: Vec<String>,
        cwd: &Path,
        selection: &[PathBuf],
    ) -> bool {
        match self
            .plugins
            .iter()
            .find(|p| p.commands.iter().any(|word| word == command))
        {
            Some(plugin) => {
                self.send_command(plugin, command, args, cwd, selection);
                true
            }
            None => false,
        }
    }

    /// Runs `:plugin <name> [args]`: fires the named plugin whether or not it declared any key or
    /// command, with the command word `"plugin"`. Returns whether a plugin by that name exists.
    pub fn dispatch_named(
        &self,
        name: &str,
        args: Vec<String>,
        cwd: &Path,
        selection: &[PathBuf],
    ) -> bool {
        match self.plugins.iter().find(|p| p.name == name) {
            Some(plugin) => {
                self.send_command(plugin, "plugin", args, cwd, selection);
                true
            }
            None => false,
        }
    }

    /// Whether no plugin is running, so a caller can skip building a snapshot nobody will read.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Makes `snapshot` what a plugin's `get_state` returns from now on. Call once per render
    /// tick; an unchanged snapshot is dropped, so a quiet browser does no work. Returns whether it
    /// replaced the previous one.
    pub fn publish_state(&self, snapshot: StateSnapshot) -> bool {
        self.state_tx.send_if_modified(|current| {
            let changed = *current != snapshot;
            if changed {
                *current = snapshot;
            }
            changed
        })
    }

    /// The names of every running plugin, in the order they were configured.
    pub fn names(&self) -> Vec<&str> {
        self.plugins.iter().map(|p| p.name.as_str()).collect()
    }

    fn send_command(
        &self,
        plugin: &RunningPlugin,
        command: &str,
        args: Vec<String>,
        cwd: &Path,
        selection: &[PathBuf],
    ) {
        let event = protocol::HostEvent::new(
            protocol::EVENT_COMMAND,
            cwd.to_path_buf(),
            selection.to_vec(),
        )
        .with_command(command, args);
        let _ = plugin.stdin_tx.send(event.to_line());
    }

    /// Drains every plugin's pending log lines. Call once per render tick; the caller decides
    /// what to do with each one (the TUI folds it into the status bar).
    pub fn poll(&mut self) -> Vec<LogMessage> {
        let mut messages = Vec::new();
        while let Ok(message) = self.logs_rx.try_recv() {
            messages.push(message);
        }
        messages
    }
}

/// Writes every queued line to the plugin's stdin, in order, until the queue's sender side (the
/// `PluginManager` and every event fired through it) is gone or the pipe itself breaks.
async fn write_loop(mut stdin: ChildStdin, mut rx: UnboundedReceiver<String>) {
    while let Some(line) = rx.recv().await {
        if stdin.write_all(line.as_bytes()).await.is_err() {
            break;
        }
    }
}

/// Reads the plugin's stdout one line at a time: a `log` notification is forwarded to
/// `logs_tx`, a `get_state` request is answered at once from `state_rx`, any other request is
/// executed on the blocking pool and replied to over `stdin_tx`, and a malformed line is reported the same way `log` is (with a reply if it carried an `id`).
async fn read_loop(
    plugin: String,
    stdout: ChildStdout,
    stdin_tx: UnboundedSender<String>,
    logs_tx: UnboundedSender<LogMessage>,
    state_rx: watch::Receiver<StateSnapshot>,
    handle: tokio::runtime::Handle,
) {
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let next = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(e) => {
                let _ = logs_tx.send(LogMessage {
                    plugin,
                    level: LogLevel::Error,
                    message: format!("stdout error: {e}"),
                });
                break;
            }
        };
        if next.trim().is_empty() {
            continue;
        }

        match protocol::parse_line(&next) {
            Ok(Incoming::Log { level, message }) => {
                let _ = logs_tx.send(LogMessage {
                    plugin: plugin.clone(),
                    level,
                    message,
                });
            }
            Ok(Incoming::UnknownNotification) => {}
            Ok(Incoming::Request { id, request }) => {
                let reply = match request {
                    PluginRequest::GetState => Ok(state_json(&state_rx.borrow())),
                    request => match handle.spawn_blocking(move || execute(request)).await {
                        Ok(result) => result,
                        Err(e) => Err(format!("request task panicked: {e}")),
                    },
                };
                let line = match reply {
                    Ok(result) => protocol::ok_response(&id, result),
                    Err(message) => protocol::err_response(&id, message),
                };
                let _ = stdin_tx.send(line);
            }
            Err(e) => {
                let _ = logs_tx.send(LogMessage {
                    plugin: plugin.clone(),
                    level: LogLevel::Warn,
                    message: e.message.clone(),
                });
                if let Some(id) = e.id {
                    let _ = stdin_tx.send(protocol::err_response(&id, e.message));
                }
            }
        }
    }
}

/// Waits out the process so `kill_on_drop` has something alive to act on for its whole lifetime,
/// and reports an abnormal exit (the common case — finishing its job and exiting 0 — stays quiet).
async fn reap(
    plugin: String,
    mut child: tokio::process::Child,
    logs_tx: UnboundedSender<LogMessage>,
) {
    if let Ok(status) = child.wait().await
        && !status.success()
    {
        let _ = logs_tx.send(LogMessage {
            plugin,
            level: LogLevel::Warn,
            message: format!("exited: {status}"),
        });
    }
}

/// Runs one plugin-requested `file_ops` call synchronously against the local filesystem — always
/// called on the blocking pool, never inline on an async task.
fn execute(request: PluginRequest) -> Result<serde_json::Value, String> {
    let vfs = LocalVfs;
    match request {
        // `read_loop` answers this one from the published snapshot and never gets here.
        PluginRequest::GetState => Err("get_state is answered by the host".to_string()),
        PluginRequest::ReadDir { path } => vfs
            .list_dir(&path)
            .map(|entries| json!(entries.iter().map(WireEntry::from).collect::<Vec<_>>()))
            .map_err(|e| e.to_string()),
        PluginRequest::Copy { src, dst, policy } => file_ops::copy(&vfs, &src, &dst, policy)
            .map(outcome_json)
            .map_err(|e| e.to_string()),
        PluginRequest::Move { src, dst, policy } => file_ops::mv(&vfs, &src, &dst, policy)
            .map(outcome_json)
            .map_err(|e| e.to_string()),
        PluginRequest::Delete { path } => file_ops::delete(&vfs, &path)
            .map(|()| json!({"outcome": "completed"}))
            .map_err(|e| e.to_string()),
        PluginRequest::CreateDir { path } => file_ops::create_directory(&vfs, &path)
            .map(|()| json!({"outcome": "completed"}))
            .map_err(|e| e.to_string()),
        PluginRequest::CreateFile { path } => file_ops::create_new_file(&vfs, &path)
            .map(|()| json!({"outcome": "completed"}))
            .map_err(|e| e.to_string()),
        PluginRequest::Touch { path } => file_ops::touch(&vfs, &path)
            .map(|()| json!({"outcome": "completed"}))
            .map_err(|e| e.to_string()),
        PluginRequest::Rename {
            path,
            new_name,
            policy,
        } => file_ops::rename(&vfs, &path, &new_name, policy)
            .map(outcome_json)
            .map_err(|e| e.to_string()),
    }
}

fn state_json(state: &StateSnapshot) -> serde_json::Value {
    serde_json::to_value(state).expect("a snapshot of plain strings and paths always serializes")
}

fn outcome_json(outcome: file_ops::Outcome) -> serde_json::Value {
    json!({
        "outcome": match outcome {
            file_ops::Outcome::Completed => "completed",
            file_ops::Outcome::Skipped => "skipped",
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_manager_starts_with_no_plugins_and_no_pending_logs() {
        let mut manager = PluginManager::new();
        assert!(manager.poll().is_empty());
        assert!(!manager.dispatch_key(KeyCode::Char('x'), Path::new("/"), &[]));
        assert!(!manager.dispatch_command("x", Vec::new(), Path::new("/"), &[]));
        assert!(!manager.dispatch_named("x", Vec::new(), Path::new("/"), &[]));
        assert!(manager.names().is_empty());
        assert!(manager.is_empty());
    }

    #[test]
    fn publishing_the_same_state_twice_reports_a_change_only_the_first_time() {
        let manager = PluginManager::new();
        let snapshot = StateSnapshot {
            cwd: PathBuf::from("/a"),
            ..StateSnapshot::default()
        };

        assert!(manager.publish_state(snapshot.clone()));
        assert!(!manager.publish_state(snapshot));
        assert!(manager.publish_state(StateSnapshot {
            cwd: PathBuf::from("/b"),
            ..StateSnapshot::default()
        }));
    }
}
