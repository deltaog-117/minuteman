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
//! `kill_on_drop` has something alive to act on for the plugin's whole lifetime. The first task also
//! drains a plugin's `Mailbox` of folded `dir_changed` and `selection_changed` events. A `get_state`
//! request is answered by that second task straight from a `watch` channel the render loop
//! publishes to each tick, so it never waits on the render thread. A `run_action` request is the
//! one that does: it is handed to the render thread (see `actions`) and answered from there. A
//! plugin whose process has ended is started again the next time something is meant for it (see
//! `life`).

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::runtime::Handle;

use crossterm::event::KeyCode;
use plugin_api::{
    CanvasInput, ChunkKind, FileChunk, MAX_READ_BYTES, PROVIDES_ARCHIVE, PROVIDES_DEVICES,
    PROVIDES_INSPECT, PROVIDES_PREVIEW, PROVIDES_VFS_ARCHIVE, PROVIDES_VFS_SSH, ReadFile, Stat,
    StatKind, StatReply, Surface,
};
use serde_json::json;
use shared::{FileKind, LocalVfs, Vfs};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::watch;

use crate::actions::{ActionLink, ActionRequest};
use crate::error::PluginError;
use crate::life::{self, Exit, Life, PluginInfo, PluginState, Verdict};
use crate::mailbox::{Mailbox, Queued};
use crate::picker::{UiLink, UiRequest};
use crate::protocol::{
    self, EventKind, HostEventKey, Incoming, JobOutcome, LogLevel, PluginRequest, StateSnapshot,
    WireEntry,
};
use crate::registry::SharedRegistry;
use crate::serve::{Pending, ServeLink};
use crate::tally::Tallies;
use crate::walk::Walks;

/// One line to show the user, from a plugin's own `log` notification or the host's diagnostics
/// about it (a malformed line, an abnormal exit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogMessage {
    pub plugin: String,
    pub level: LogLevel,
    pub message: String,
}

/// One process of a plugin: the lines to write to it, what is waiting for it, and how it ends.
struct Process {
    stdin_tx: UnboundedSender<String>,
    mailbox: Arc<Mailbox>,
    life: Arc<Life>,
}

struct RunningPlugin {
    name: String,
    command: String,
    args: Vec<String>,
    handle: Handle,
    on_keys: Vec<KeyCode>,
    events: Vec<EventKind>,
    /// The interface's actions this plugin answers (`provides` in its config).
    provides: Vec<String>,
    registry: SharedRegistry,
    actions: ActionLink,
    vfs: Arc<dyn Vfs>,
    ui: UiLink,
    /// Set for a plugin that answers for paths: each process it has is attached to this.
    serve: Option<Arc<ServeLink>>,
    process: Process,
    /// Crashes in a row, each soon after starting (see `life::after_crash`).
    crashes: u32,
    /// How many times the host started the process again.
    restarts: u32,
    /// Set once the plugin has crashed too often; only `restart` clears it.
    given_up: bool,
    /// The status of the last crash, for `:plugins`.
    last_crash: Option<String>,
}

impl RunningPlugin {
    /// Whether a process is there to write to, starting a new one first if the last has ended and
    /// the plugin has not crashed too often in a row. With `announce`, a plugin the host has given
    /// up on says so, since a key or command that does nothing is otherwise a mystery.
    fn ensure_alive(
        &mut self,
        logs_tx: &UnboundedSender<LogMessage>,
        state_tx: &watch::Sender<StateSnapshot>,
        announce: bool,
    ) -> bool {
        if self.given_up {
            if announce {
                self.say(
                    logs_tx,
                    LogLevel::Warn,
                    format!(
                        "stopped after {} crashes in a row; `:plugins restart {}` starts it again",
                        life::MAX_QUICK_CRASHES,
                        self.name
                    ),
                );
            }
            return false;
        }
        match self.process.life.exit() {
            Exit::Running => return true,
            Exit::Finished => self.crashes = 0,
            Exit::Crashed { status, ran_for } => {
                self.last_crash = Some(status);
                match life::after_crash(self.crashes, ran_for) {
                    Verdict::Respawn { crashes } => self.crashes = crashes,
                    Verdict::GiveUp { crashes } => {
                        self.crashes = crashes;
                        self.given_up = true;
                        return self.ensure_alive(logs_tx, state_tx, announce);
                    }
                }
            }
        }
        match self.start_again(logs_tx, state_tx) {
            Ok(()) => true,
            // A plugin that cannot be started at all is given up on: a retry changes nothing.
            Err(e) => {
                self.given_up = true;
                self.last_crash = Some(e.to_string());
                self.say(logs_tx, LogLevel::Error, e.to_string());
                false
            }
        }
    }

    /// Starts a fresh process in place of the last, in the directory last published.
    fn start_again(
        &mut self,
        logs_tx: &UnboundedSender<LogMessage>,
        state_tx: &watch::Sender<StateSnapshot>,
    ) -> Result<(), PluginError> {
        let cwd = state_tx.borrow().cwd.clone();
        self.process = start(
            &self.handle,
            &self.name,
            &self.command,
            &self.args,
            &cwd,
            Wiring {
                logs_tx: logs_tx.clone(),
                state_rx: state_tx.subscribe(),
                registry: self.registry.clone(),
                actions: self.actions.clone(),
                vfs: Arc::clone(&self.vfs),
                ui: self.ui.clone(),
                serve: self.serve.clone(),
            },
        )?;
        self.restarts += 1;
        Ok(())
    }

    /// Whether `event` may be written to the plugin now. A plugin only ever started by events
    /// (no keys, no commands) has nothing else to bring it back, so one of those is started
    /// again for an event; any other waits for a key or a command, including one it registered
    /// itself while running.
    fn ready_for_event(
        &mut self,
        logs_tx: &UnboundedSender<LogMessage>,
        state_tx: &watch::Sender<StateSnapshot>,
    ) -> bool {
        if self.process.life.exit() == Exit::Running {
            return true;
        }
        self.on_keys.is_empty()
            && self.registry.lock().words_of(&self.name).is_empty()
            && self.ensure_alive(logs_tx, state_tx, false)
    }

    fn say(&self, logs_tx: &UnboundedSender<LogMessage>, level: LogLevel, message: String) {
        let _ = logs_tx.send(LogMessage {
            plugin: self.name.clone(),
            level,
            message,
        });
    }

    fn send_command(&self, command: &str, args: Vec<String>, cwd: &Path, selection: &[PathBuf]) {
        let event = protocol::HostEvent::new(
            protocol::EVENT_COMMAND,
            cwd.to_path_buf(),
            selection.to_vec(),
        )
        .with_command(command, args);
        let _ = self.process.stdin_tx.send(event.to_line());
    }

    fn info(&self) -> PluginInfo {
        let crashed = || self.last_crash.clone().unwrap_or_default();
        let state = if self.given_up {
            PluginState::Stopped(crashed())
        } else {
            match self.process.life.exit() {
                Exit::Running => PluginState::Running,
                Exit::Finished => PluginState::Finished,
                Exit::Crashed { status, .. } => PluginState::Crashed(status),
            }
        };
        PluginInfo {
            name: self.name.clone(),
            state,
            restarts: self.restarts,
        }
    }
}

/// What a plugin's process is connected to besides its own pipes: where its log lines go, the
/// published state it reads, the command registry it writes and the way to the interface's
/// actions.
struct Wiring {
    logs_tx: UnboundedSender<LogMessage>,
    state_rx: watch::Receiver<StateSnapshot>,
    registry: SharedRegistry,
    actions: ActionLink,
    /// What a `walk` request reads through: the same paths the browser shows.
    vfs: Arc<dyn Vfs>,
    ui: UiLink,
    /// The link to attach the process to, when the plugin answers for paths.
    serve: Option<Arc<ServeLink>>,
}

/// How `PluginManager::restart` went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restart {
    NotFound,
    Restarted,
    /// The process could not be started; the reason.
    Failed(String),
}

/// What came of `PluginManager::dispatch_action`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionDispatch {
    /// No plugin provides that action, so the interface handles it itself.
    NoProvider,
    /// A plugin provides it but cannot be started; the reason was logged (see `poll`).
    Unavailable,
    /// The plugin has the action.
    Sent,
}

/// What a plugin is configured to answer: the keys that fire it, the `:` command words it
/// declared, the events it asked to hear about and the actions it provides. All may be empty, for a plugin only ever
/// started by `:plugin <name>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Triggers {
    pub keys: Vec<KeyCode>,
    pub commands: Vec<String>,
    pub events: Vec<EventKind>,
    /// The names of the interface's actions this plugin answers instead of the interface: when
    /// the user triggers one, the plugin gets an `action` event and nothing else happens. The
    /// first plugin configured with a name gets it.
    pub actions: Vec<String>,
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
    /// Who answers which `:` command word, shared with every plugin's reader.
    registry: SharedRegistry,
    /// Where every plugin's reader sends `run_action`, and the names `list_actions` answers.
    actions: ActionLink,
    actions_rx: UnboundedReceiver<ActionRequest>,
    /// Where every plugin's reader sends what concerns its picker, for the render thread.
    ui: UiLink,
    ui_rx: UnboundedReceiver<UiRequest>,
    /// What a plugin's `walk` reads through. The local disk until the interface says otherwise.
    vfs: Arc<dyn Vfs>,
    /// The way to the plugin that answers for the paths inside archives.
    serve: Arc<ServeLink>,
    /// The way to the plugin that answers for the paths on remote machines.
    serve_ssh: Arc<ServeLink>,
    /// Whether the interface has published at least once; the first publish is the browser
    /// arriving, not a change, so it fires no event.
    published: AtomicBool,
}

impl Default for PluginManager {
    fn default() -> Self {
        let (logs_tx, logs_rx) = unbounded_channel();
        let (actions_tx, actions_rx) = unbounded_channel();
        let (ui_tx, ui_rx) = unbounded_channel();
        Self {
            plugins: Vec::new(),
            logs_tx,
            logs_rx,
            state_tx: watch::channel(StateSnapshot::default()).0,
            registry: SharedRegistry::default(),
            actions: ActionLink::new(actions_tx),
            actions_rx,
            ui: UiLink::new(ui_tx),
            ui_rx,
            vfs: Arc::new(LocalVfs),
            serve: ServeLink::new(),
            serve_ssh: ServeLink::new(),
            published: AtomicBool::new(false),
        }
    }
}

impl PluginManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts one plugin process, fires its `init` event, and registers it to fire `key` on each
    /// of `triggers.keys` and `command` on each of `triggers.commands`. A failure to spawn (bad
    /// command, missing binary) is returned rather than panicking — the caller decides whether that is fatal or just a warning, the same
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
        // A plugin may ask for the state in answer to `init`, before the first tick has published
        // one, so until then the answer names at least the directory the session started in.
        self.state_tx.send_if_modified(|state| {
            let unset = state.cwd.as_os_str().is_empty();
            if unset {
                state.cwd = cwd.to_path_buf();
            }
            unset
        });

        {
            // A word the interface or an earlier plugin already has stays theirs, as it always
            // did: a built-in is tried first, and the plugin configured first wins a word.
            let mut registry = self.registry.lock();
            for word in &triggers.commands {
                let _ = registry.register(name, word, false);
            }
        }

        // The word a plugin provides says which kind of path it answers for, and so which link
        // its process is attached to.
        let serve = triggers
            .actions
            .iter()
            .find_map(|provided| match provided.as_str() {
                PROVIDES_VFS_ARCHIVE => Some(Arc::clone(&self.serve)),
                PROVIDES_VFS_SSH => Some(Arc::clone(&self.serve_ssh)),
                _ => None,
            });
        let process = start(
            handle,
            name,
            command,
            args,
            cwd,
            Wiring {
                logs_tx: self.logs_tx.clone(),
                state_rx: self.state_tx.subscribe(),
                registry: self.registry.clone(),
                actions: self.actions.clone(),
                vfs: Arc::clone(&self.vfs),
                ui: self.ui.clone(),
                serve: serve.clone(),
            },
        )?;
        self.plugins.push(RunningPlugin {
            name: name.to_string(),
            command: command.to_string(),
            args: args.to_vec(),
            handle: handle.clone(),
            on_keys: triggers.keys,
            events: triggers.events,
            provides: triggers.actions,
            registry: self.registry.clone(),
            actions: self.actions.clone(),
            vfs: Arc::clone(&self.vfs),
            ui: self.ui.clone(),
            serve,
            process,
            crashes: 0,
            restarts: 0,
            given_up: false,
            last_crash: None,
        });
        Ok(())
    }

    /// Fires the plugin bound to `code`, if any, with the browsed directory and the current
    /// selection (marks, or the single entry under the cursor — the same batch every built-in
    /// bulk action uses), and which key it was. When two plugins claim a key the one configured
    /// first gets it. Returns whether a plugin consumed the key, so the caller can tell a truly
    /// unbound key apart from one a plugin handled. A plugin whose process has ended is started
    /// again first (see `life`).
    pub fn dispatch_key(&mut self, code: KeyCode, cwd: &Path, selection: &[PathBuf]) -> bool {
        let Self {
            plugins,
            logs_tx,
            state_tx,
            ..
        } = self;
        let Some(plugin) = plugins.iter_mut().find(|p| p.on_keys.contains(&code)) else {
            return false;
        };
        if plugin.ensure_alive(logs_tx, state_tx, true) {
            let event = protocol::HostEvent::new(
                protocol::EVENT_KEY,
                cwd.to_path_buf(),
                selection.to_vec(),
            )
            .with_key(code);
            let _ = plugin.process.stdin_tx.send(event.to_line());
        }
        true
    }

    /// Fires `command` at the plugin that declared that word (first configured wins), with the
    /// same directory and selection a key event carries plus the arguments. Returns whether a
    /// plugin answered, so the caller can fall through to running the line as a shell command.
    pub fn dispatch_command(
        &mut self,
        command: &str,
        args: Vec<String>,
        cwd: &Path,
        selection: &[PathBuf],
    ) -> bool {
        let Some(owner) = self.registry.lock().owner(command).map(str::to_string) else {
            return false;
        };
        let Self {
            plugins,
            logs_tx,
            state_tx,
            ..
        } = self;
        let Some(plugin) = plugins.iter_mut().find(|p| p.name == owner) else {
            return false;
        };
        if plugin.ensure_alive(logs_tx, state_tx, true) {
            plugin.send_command(command, args, cwd, selection);
        }
        true
    }

    /// Runs `:plugin <name> [args]`: fires the named plugin whether or not it declared any key or
    /// command, with the command word `"plugin"`. Returns whether a plugin by that name exists.
    pub fn dispatch_named(
        &mut self,
        name: &str,
        args: Vec<String>,
        cwd: &Path,
        selection: &[PathBuf],
    ) -> bool {
        let Self {
            plugins,
            logs_tx,
            state_tx,
            ..
        } = self;
        let Some(plugin) = plugins.iter_mut().find(|p| p.name == name) else {
            return false;
        };
        if plugin.ensure_alive(logs_tx, state_tx, true) {
            plugin.send_command("plugin", args, cwd, selection);
        }
        true
    }

    /// Points the plugins started from now on at `vfs`, which their `walk` requests read through
    /// (the local disk until this is called). Call before spawning, like `declare_actions`.
    pub fn set_vfs(&mut self, vfs: Arc<dyn Vfs>) {
        self.vfs = vfs;
    }

    /// The way to the plugin that answers for the paths inside archives, for the `Vfs` that sends
    /// them there. Hand it to that `Vfs` before the plugin is spawned: it is attached to each
    /// process of the plugin as it starts, and says the plugin is unavailable until then.
    pub fn serve_link(&self) -> Arc<ServeLink> {
        Arc::clone(&self.serve)
    }

    /// Uses `link` as the way to the plugin that answers for the paths inside archives, for a
    /// `Vfs` that was built with it before this manager existed. Call before spawning.
    pub fn set_serve_link(&mut self, link: Arc<ServeLink>) {
        self.serve = link;
    }

    /// Whether a plugin that answers for the paths inside archives is running right now.
    pub fn serves_archives(&self) -> bool {
        self.serve.is_running()
    }

    /// The way to the plugin that answers for the paths on remote machines; see [`Self::serve_link`].
    pub fn serve_ssh_link(&self) -> Arc<ServeLink> {
        Arc::clone(&self.serve_ssh)
    }

    /// Uses `link` as the way to the plugin that answers for the paths on remote machines; see
    /// [`Self::set_serve_link`].
    pub fn set_serve_ssh_link(&mut self, link: Arc<ServeLink>) {
        self.serve_ssh = link;
    }

    /// Whether a plugin that answers for the paths on remote machines is running right now.
    pub fn serves_ssh(&self) -> bool {
        self.serve_ssh.is_running()
    }

    /// Hands the action `name` to the plugin that provides it (the first configured with that
    /// name), with the browsed directory, the selection and whether dot-files are shown. A plugin
    /// whose process has ended is started again first (see `life`), and one the host has given up
    /// on says so through `poll`.
    pub fn dispatch_action(
        &mut self,
        name: &str,
        cwd: &Path,
        selection: &[PathBuf],
    ) -> ActionDispatch {
        let show_hidden = self.state_tx.borrow().show_hidden;
        let Self {
            plugins,
            logs_tx,
            state_tx,
            ..
        } = self;
        let Some(plugin) = plugins
            .iter_mut()
            .find(|p| p.provides.iter().any(|provided| provided == name))
        else {
            return ActionDispatch::NoProvider;
        };
        if !plugin.ensure_alive(logs_tx, state_tx, true) {
            return ActionDispatch::Unavailable;
        }
        let event =
            protocol::HostEvent::action(name, cwd.to_path_buf(), selection.to_vec(), show_hidden);
        let _ = plugin.process.stdin_tx.send(event.to_line());
        ActionDispatch::Sent
    }

    /// Tells the plugin that provides `preview` which file the preview column is about (`None`
    /// when it shows none), or that the file it already showed changed. Returns whether a plugin
    /// got it: `false` when none provides `preview`, or the one that does cannot be started, so
    /// the interface can show the file's name instead of waiting for a document that will not
    /// come. A plugin whose process has ended is started again first, like for any event.
    pub fn dispatch_preview(&mut self, path: Option<&Path>) -> bool {
        let Self {
            plugins,
            logs_tx,
            state_tx,
            ..
        } = self;
        let Some(plugin) = plugins.iter_mut().find(|p| {
            p.provides
                .iter()
                .any(|provided| provided == PROVIDES_PREVIEW)
        }) else {
            return false;
        };
        if !plugin.ready_for_event(logs_tx, state_tx) {
            return false;
        }
        let event = protocol::HostEvent::preview(path.map(Path::to_path_buf));
        plugin.process.stdin_tx.send(event.to_line()).is_ok()
    }

    /// Whether a plugin that provides `preview` has a live process right now. A document that was
    /// being made when that process died will never come, which the interface can tell by this.
    pub fn previewer_running(&self) -> bool {
        self.plugins.iter().any(|p| {
            p.provides
                .iter()
                .any(|provided| provided == PROVIDES_PREVIEW)
                && p.process.life.exit() == Exit::Running
        })
    }

    /// Whether a plugin is configured to provide `preview`, running or not.
    pub fn has_previewer(&self) -> bool {
        self.plugins.iter().any(|p| {
            p.provides
                .iter()
                .any(|provided| provided == PROVIDES_PREVIEW)
        })
    }

    /// Tells the plugin that provides `inspect` which entry the Inspect panel is about (`None`
    /// when the panel closed, so a count under way can stop). Returns whether a plugin got it:
    /// `false` when none provides `inspect`, or the one that does cannot be started, so the
    /// interface can say where to get it instead of waiting for rows that will not come.
    pub fn dispatch_inspect(&mut self, path: Option<&Path>) -> bool {
        let Self {
            plugins,
            logs_tx,
            state_tx,
            ..
        } = self;
        let Some(plugin) = plugins.iter_mut().find(|p| {
            p.provides
                .iter()
                .any(|provided| provided == PROVIDES_INSPECT)
        }) else {
            return false;
        };
        if !plugin.ready_for_event(logs_tx, state_tx) {
            return false;
        }
        let event = protocol::HostEvent::inspect(path.map(Path::to_path_buf));
        plugin.process.stdin_tx.send(event.to_line()).is_ok()
    }

    /// Whether a plugin that provides `inspect` has a live process right now. Rows that were
    /// being made when that process died will never come, which the interface can tell by this.
    pub fn inspector_running(&self) -> bool {
        self.plugins.iter().any(|p| {
            p.provides
                .iter()
                .any(|provided| provided == PROVIDES_INSPECT)
                && p.process.life.exit() == Exit::Running
        })
    }

    /// Tells the plugin that draws `surface` what happened in its view. Returns whether a plugin
    /// got it: `false` when none provides the view, or the one that does cannot be started, so
    /// the interface can say where to get it instead of waiting for a canvas that will not come.
    /// A plugin whose process has ended is started again first, except to be told the view
    /// closed: that never starts a process just to tell it so.
    pub fn dispatch_canvas(&mut self, surface: Surface, input: CanvasInput) -> bool {
        let Self {
            plugins,
            logs_tx,
            state_tx,
            ..
        } = self;
        let Some(plugin) = plugins
            .iter_mut()
            .find(|p| p.provides.iter().any(|word| word == surface.provides()))
        else {
            return false;
        };
        if input == CanvasInput::Close && plugin.process.life.exit() != Exit::Running {
            return true;
        }
        if !plugin.ready_for_event(logs_tx, state_tx) {
            return false;
        }
        let event = protocol::HostEvent::canvas(surface, input);
        plugin.process.stdin_tx.send(event.to_line()).is_ok()
    }

    /// Whether a plugin is configured to draw `surface`, running or not.
    pub fn has_canvas(&self, surface: Surface) -> bool {
        self.plugins
            .iter()
            .any(|p| p.provides.iter().any(|word| word == surface.provides()))
    }

    /// Whether a plugin that draws `surface` has a live process right now. A canvas that was
    /// being kept current when that process died will never be updated, which the interface can
    /// tell by this.
    pub fn canvas_running(&self, surface: Surface) -> bool {
        self.plugins.iter().any(|p| {
            p.provides.iter().any(|word| word == surface.provides())
                && p.process.life.exit() == Exit::Running
        })
    }

    /// Hands `command` to the plugin that provides `devices`. Returns whether a plugin got it:
    /// `false` when none provides `devices`, or the one that does cannot be started, so the
    /// interface can say where to get it instead of waiting for a list that will not come. A
    /// plugin whose process has ended is started again first, except to be told to stop
    /// watching: that never starts a process just to tell it so.
    pub fn dispatch_devices(&mut self, command: protocol::DevicesCommand) -> bool {
        let Self {
            plugins,
            logs_tx,
            state_tx,
            ..
        } = self;
        let Some(plugin) = plugins.iter_mut().find(|p| {
            p.provides
                .iter()
                .any(|provided| provided == PROVIDES_DEVICES)
        }) else {
            return false;
        };
        let stopping = matches!(command, protocol::DevicesCommand::Watch { on: false });
        if stopping && plugin.process.life.exit() != Exit::Running {
            return true;
        }
        if !plugin.ready_for_event(logs_tx, state_tx) {
            return false;
        }
        let event = protocol::HostEvent::devices(command);
        plugin.process.stdin_tx.send(event.to_line()).is_ok()
    }

    /// Whether a plugin that provides `devices` has a live process right now. A list that was
    /// being kept current when that process died will never be updated, which the interface can
    /// tell by this.
    pub fn devices_running(&self) -> bool {
        self.plugins.iter().any(|p| {
            p.provides
                .iter()
                .any(|provided| provided == PROVIDES_DEVICES)
                && p.process.life.exit() == Exit::Running
        })
    }

    /// Hands `work` to the plugin that provides `archive`, to be reported under the number `job`.
    /// Returns whether a plugin got it: `false` when none provides `archive`, or the one that does
    /// cannot be started, so the interface can say where to get it instead of waiting for a result
    /// that will not come. A plugin whose process has ended is started again first.
    pub fn dispatch_archive(&mut self, job: u64, work: protocol::ArchiveWork) -> bool {
        let Self {
            plugins,
            logs_tx,
            state_tx,
            ..
        } = self;
        let Some(plugin) = plugins.iter_mut().find(|p| {
            p.provides
                .iter()
                .any(|provided| provided == PROVIDES_ARCHIVE)
        }) else {
            return false;
        };
        if !plugin.ready_for_event(logs_tx, state_tx) {
            return false;
        }
        let event = protocol::HostEvent::archive(job, work);
        plugin.process.stdin_tx.send(event.to_line()).is_ok()
    }

    /// Asks the plugin that provides `archive` to stop the request numbered `job`. A plugin that
    /// has gone has nothing to stop, so nothing is started to be told.
    pub fn cancel_archive(&self, job: u64) {
        if let Some(plugin) = self.plugins.iter().find(|p| {
            p.provides
                .iter()
                .any(|provided| provided == PROVIDES_ARCHIVE)
        }) {
            self.write_to(&plugin.name, protocol::HostEvent::archive_cancel(job));
        }
    }

    /// Whether a plugin that provides `archive` has a live process right now. A request that was
    /// being worked on when that process died will never end, which the interface can tell by this.
    pub fn archiver_running(&self) -> bool {
        self.plugins.iter().any(|p| {
            p.provides
                .iter()
                .any(|provided| provided == PROVIDES_ARCHIVE)
                && p.process.life.exit() == Exit::Running
        })
    }

    /// Tells `plugin` that what is typed into its picker is now `text`. `generation` rises with
    /// every change, and the plugin's `picker_items` carry the one they answer. Nothing is sent to
    /// a plugin whose process has ended; the interface notices that through `is_running`.
    pub fn send_picker_query(&self, plugin: &str, picker: u64, generation: u64, text: &str) {
        self.write_to(
            plugin,
            protocol::HostEvent::picker_query(picker, generation, text),
        );
    }

    /// Tells `plugin` its picker was closed, by the user or because a result was chosen.
    pub fn send_picker_closed(&self, plugin: &str, picker: u64) {
        self.write_to(plugin, protocol::HostEvent::picker_closed(picker));
    }

    fn write_to(&self, plugin: &str, event: protocol::HostEvent) {
        if let Some(plugin) = self.plugins.iter().find(|p| p.name == plugin)
            && plugin.process.life.exit() == Exit::Running
        {
            let _ = plugin.process.stdin_tx.send(event.to_line());
        }
    }

    /// Whether `plugin` has a live process right now.
    pub fn is_running(&self, plugin: &str) -> bool {
        self.plugins
            .iter()
            .any(|p| p.name == plugin && p.process.life.exit() == Exit::Running)
    }

    /// The oldest thing a plugin asked of the interface's picker that is waiting for the render
    /// thread, if any. Call until it returns `None` once per render tick; answer a `ShowPicker`
    /// with `opened` or `refuse`.
    pub fn take_ui(&mut self) -> Option<UiRequest> {
        self.ui_rx.try_recv().ok()
    }

    /// Every plugin's state, in the order they were configured, for `:plugins`.
    pub fn summaries(&self) -> Vec<PluginInfo> {
        self.plugins.iter().map(RunningPlugin::info).collect()
    }

    /// Stops the named plugin's process if it is still alive and starts a fresh one, forgetting
    /// any crashes, so a plugin the host gave up on gets another go.
    pub fn restart(&mut self, name: &str) -> Restart {
        let Self {
            plugins,
            logs_tx,
            state_tx,
            ..
        } = self;
        let Some(plugin) = plugins.iter_mut().find(|p| p.name == name) else {
            return Restart::NotFound;
        };
        plugin.process.life.stop();
        plugin.registry.lock().clear_runtime(name);
        plugin.crashes = 0;
        plugin.given_up = false;
        plugin.last_crash = None;
        match plugin.start_again(logs_tx, state_tx) {
            Ok(()) => Restart::Restarted,
            Err(e) => {
                plugin.given_up = true;
                plugin.last_crash = Some(e.to_string());
                Restart::Failed(e.to_string())
            }
        }
    }

    /// Declares the words of the interface's own `:` commands, which no plugin may claim. Call
    /// before spawning, so a plugin's config cannot get ahead of them.
    pub fn reserve_commands(&mut self, words: impl IntoIterator<Item = String>) {
        self.registry.lock().reserve(words);
    }

    /// Declares the names of the interface's actions a plugin may run with `run_action`, which
    /// `list_actions` answers with in this order. Call before spawning, like `reserve_commands`:
    /// a plugin started earlier keeps the list it was given.
    pub fn declare_actions(&mut self, names: impl IntoIterator<Item = String>) {
        self.actions = self.actions.with_names(names.into_iter().collect());
    }

    /// The oldest `run_action` waiting for the render thread, if any. Call once per render tick
    /// and answer what it returns; a plugin waits for that answer before it sends anything else.
    pub fn take_action(&mut self) -> Option<ActionRequest> {
        self.actions_rx.try_recv().ok()
    }

    /// Whether no plugin is running, so a caller can skip building a snapshot nobody will read.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Makes `snapshot` what a plugin's `get_state` returns from now on, and leaves a
    /// `dir_changed` or `selection_changed` for each plugin subscribed to what differs from the
    /// state it replaces (the first snapshot ever published fires nothing). Call once per render
    /// tick; an unchanged snapshot is dropped, so a quiet browser does no work. Returns whether it
    /// replaced the previous one.
    pub fn publish_state(&mut self, snapshot: StateSnapshot) -> bool {
        let first = !self.published.swap(true, Ordering::Relaxed);
        let mut previous = None;
        let changed = self.state_tx.send_if_modified(|current| {
            if *current == snapshot {
                return false;
            }
            previous = Some(std::mem::replace(current, snapshot.clone()));
            true
        });
        if let (false, Some(previous)) = (first, previous) {
            self.post_changes(&previous, &snapshot);
        }
        changed
    }

    /// Fires `job_finished` at every plugin that subscribed to it, with the directory and marks
    /// as of the last published state. Jobs are rare and each one matters, so unlike the change
    /// events these are never folded together.
    pub fn notify_job_finished(&mut self, kind: &str, outcome: JobOutcome) {
        let state = self.state_tx.borrow().clone();
        let Self {
            plugins,
            logs_tx,
            state_tx,
            ..
        } = self;
        for plugin in plugins
            .iter_mut()
            .filter(|p| p.events.contains(&EventKind::JobFinished))
        {
            if plugin.ready_for_event(logs_tx, state_tx) {
                let event = protocol::HostEvent::new(
                    protocol::EVENT_JOB_FINISHED,
                    state.cwd.clone(),
                    state.marks.clone(),
                )
                .with_job(kind, outcome);
                let _ = plugin.process.stdin_tx.send(event.to_line());
            }
        }
    }

    fn post_changes(&mut self, previous: &StateSnapshot, now: &StateSnapshot) {
        let queued = |from: Option<PathBuf>, to: Option<PathBuf>| Queued {
            from,
            to,
            cwd: now.cwd.clone(),
            marks: now.marks.clone(),
        };
        let dir_moved = previous.cwd != now.cwd;
        let selection_moved = previous.cursor != now.cursor || previous.marks != now.marks;
        let Self {
            plugins,
            logs_tx,
            state_tx,
            ..
        } = self;
        for plugin in plugins.iter_mut() {
            if dir_moved
                && plugin.events.contains(&EventKind::DirChanged)
                && plugin.ready_for_event(logs_tx, state_tx)
            {
                plugin
                    .process
                    .mailbox
                    .post_dir(queued(Some(previous.cwd.clone()), Some(now.cwd.clone())));
            }
            if selection_moved
                && plugin.events.contains(&EventKind::SelectionChanged)
                && plugin.ready_for_event(logs_tx, state_tx)
            {
                plugin
                    .process
                    .mailbox
                    .post_selection(queued(previous.cursor.clone(), now.cursor.clone()));
            }
        }
    }

    /// The names of every running plugin, in the order they were configured.
    pub fn names(&self) -> Vec<&str> {
        self.plugins.iter().map(|p| p.name.as_str()).collect()
    }

    /// Starts again a plugin that answers for paths and whose process has ended. Nothing else is
    /// ever meant for such a plugin to wake it, and the interface waits on it in the middle of a
    /// listing, so it is not left to the next key press; the usual limit on crashes in a row
    /// applies (see `life`), after which only `restart` brings it back.
    fn revive_servers(&mut self) {
        let Self {
            plugins,
            logs_tx,
            state_tx,
            ..
        } = self;
        for plugin in plugins.iter_mut().filter(|p| p.serve.is_some()) {
            if plugin.process.life.exit() != Exit::Running {
                plugin.ensure_alive(logs_tx, state_tx, false);
            }
        }
    }

    /// Drains every plugin's pending log lines. Call once per render tick; the caller decides
    /// what to do with each one (the TUI folds it into the status bar).
    pub fn poll(&mut self) -> Vec<LogMessage> {
        self.revive_servers();
        let mut messages = Vec::new();
        while let Ok(message) = self.logs_rx.try_recv() {
            messages.push(message);
        }
        messages
    }
}

/// Writes every queued line to the plugin's stdin, in order, until the queue's sender side (the
/// `PluginManager` and every event fired through it) is gone or the pipe itself breaks. Whatever
/// has been folded into the plugin's mailbox is written whenever the queue is idle; a write that
/// waits on a full pipe holds both back, which is what lets the mailbox keep folding.
async fn write_loop(
    mut stdin: ChildStdin,
    mut rx: UnboundedReceiver<String>,
    mailbox: Arc<Mailbox>,
) {
    loop {
        // Biased towards the queue, whose first line is always `init`: a plugin must hear that
        // before any change event, even one posted the instant it was started.
        let lines = tokio::select! {
            biased;
            line = rx.recv() => match line {
                Some(line) => vec![line],
                None => break,
            },
            () = mailbox.posted() => mailbox.take_lines(),
        };
        for line in lines {
            if stdin.write_all(line.as_bytes()).await.is_err() {
                return;
            }
        }
    }
}

/// Reads the plugin's stdout one line at a time: a `log` notification is forwarded to
/// `logs_tx`, a `get_state` request is answered at once from `state_rx`, a `register_command` or
/// `unregister_command` at once from `registry`, a `run_action` once the render thread has run it
/// (`list_actions` at once from `actions`), a `show_picker` once the render thread has answered it
/// (the picker's `picker_items` and `picker_close` go to the render thread without a reply), a
/// `walk` at once with the walk running on the blocking pool, any other request is
/// executed on the blocking pool and replied to over `stdin_tx`, and a malformed line is reported the same way `log` is (with a reply if it carried an `id`).
async fn read_loop(
    plugin: String,
    stdout: ChildStdout,
    stdin_tx: UnboundedSender<String>,
    pending: Arc<Pending>,
    wiring: Wiring,
    handle: tokio::runtime::Handle,
) {
    let Wiring {
        logs_tx,
        state_rx,
        registry,
        actions,
        vfs,
        ui,
        serve: _,
    } = wiring;
    let walks = Arc::new(Walks::default());
    let tallies = Arc::new(Tallies::default());
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
            Ok(Incoming::PickerItems(update)) => ui.items(&plugin, update),
            Ok(Incoming::PickerClose(close)) => ui.close(&plugin, close.picker),
            Ok(Incoming::SetView(set)) => ui.set_view(&plugin, set),
            Ok(Incoming::SetPreview(set)) => ui.set_preview(&plugin, set),
            Ok(Incoming::SetInspect(set)) => ui.set_inspect(&plugin, set),
            Ok(Incoming::SetCanvas(set)) => ui.set_canvas(&plugin, set),
            Ok(Incoming::CanvasNote(note)) => ui.canvas_note(&plugin, note),
            Ok(Incoming::SetDevices(set)) => ui.set_devices(&plugin, set),
            Ok(Incoming::DevicesNote(note)) => ui.devices_note(&plugin, note),
            Ok(Incoming::ArchiveProgress(progress)) => ui.archive_progress(&plugin, progress),
            Ok(Incoming::ArchiveDone(done)) => ui.archive_done(&plugin, done),
            Ok(Incoming::WalkCancel(cancel)) => walks.cancel(cancel.walk),
            Ok(Incoming::TallyCancel(cancel)) => tallies.cancel(cancel.tally),
            Ok(Incoming::Response { id, outcome }) => {
                if let Some(close) = pending.answer(&id, outcome) {
                    let _ = stdin_tx.send(close);
                }
            }
            Ok(Incoming::UnknownNotification) => {}
            Ok(Incoming::Request { id, request }) => {
                let reply = match request {
                    PluginRequest::GetState => Ok(state_json(&state_rx.borrow())),
                    PluginRequest::RegisterCommand { name } => registry
                        .lock()
                        .register(&plugin, &name, true)
                        .map(|()| json!({ "registered": name }))
                        .map_err(|e| format!("cannot register '{name}': {e}")),
                    PluginRequest::UnregisterCommand { name } => registry
                        .lock()
                        .unregister(&plugin, &name)
                        .map(|()| json!({ "unregistered": name }))
                        .map_err(|e| format!("cannot unregister '{name}': {e}")),
                    PluginRequest::RunAction { name } => actions.run(&plugin, name).await,
                    PluginRequest::ListActions => Ok(actions.list()),
                    PluginRequest::ShowPicker(spec) => ui.show_picker(&plugin, spec).await,
                    // A person is typing the answer, which can take minutes, and the replies of the
                    // plugin's other requests come through this loop meanwhile: answer from a task.
                    PluginRequest::Ask(spec) => {
                        let (ui, stdin_tx, plugin, id) =
                            (ui.clone(), stdin_tx.clone(), plugin.clone(), id.clone());
                        handle.spawn(async move {
                            let line = match ui.ask(&plugin, spec).await {
                                Ok(result) => protocol::ok_response(&id, result),
                                Err(message) => protocol::err_response(&id, message),
                            };
                            let _ = stdin_tx.send(line);
                        });
                        continue;
                    }
                    PluginRequest::Walk(request) => {
                        let number = request.walk;
                        walks
                            .start(&handle, Arc::clone(&vfs), request, stdin_tx.clone())
                            .map(|()| json!({ "walk": number }))
                    }
                    PluginRequest::Tally(request) => {
                        let number = request.tally;
                        tallies
                            .start(&handle, Arc::clone(&vfs), request, stdin_tx.clone())
                            .map(|()| json!({ "tally": number }))
                    }
                    PluginRequest::ReadFile(read) => {
                        let vfs = Arc::clone(&vfs);
                        match handle
                            .spawn_blocking(move || read_chunk(vfs.as_ref(), &read))
                            .await
                        {
                            Ok(result) => result,
                            Err(e) => Err(format!("request task panicked: {e}")),
                        }
                    }
                    PluginRequest::Stat(stat) => {
                        let vfs = Arc::clone(&vfs);
                        match handle
                            .spawn_blocking(move || stat_entry(vfs.as_ref(), &stat))
                            .await
                        {
                            Ok(result) => result,
                            Err(e) => Err(format!("request task panicked: {e}")),
                        }
                    }
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
    // The process is gone (or its output is): nobody is left to read what its walks find, and
    // nothing will answer what the host asked of it.
    pending.fail_all();
    walks.cancel_all();
    tallies.cancel_all();
}

/// Waits out the process so `kill_on_drop` has something alive to act on for its whole lifetime,
/// records how it ended for `PluginManager` to find, and reports a crash (the common case,
/// finishing its job and exiting 0, stays quiet, and so does a process the host stopped itself).
async fn reap(
    plugin: String,
    mut child: tokio::process::Child,
    logs_tx: UnboundedSender<LogMessage>,
    life: Arc<Life>,
) {
    let status = tokio::select! {
        status = child.wait() => status,
        () = life.stop_requested() => {
            let _ = child.kill().await;
            child.wait().await
        }
    };
    let exit = match status {
        Ok(status) if status.success() => Exit::Finished,
        Ok(status) => Exit::Crashed {
            status: status.to_string(),
            ran_for: life.age(),
        },
        Err(e) => Exit::Crashed {
            status: format!("could not wait for it: {e}"),
            ran_for: life.age(),
        },
    };
    if let Exit::Crashed { status, .. } = &exit
        && !life.was_stopped_by_host()
    {
        let _ = logs_tx.send(LogMessage {
            plugin,
            level: LogLevel::Warn,
            message: format!("exited: {status}"),
        });
    }
    life.set_exit(exit);
}

/// Spawns one plugin process with the three tasks that drive it, and queues its `init` event.
fn start(
    handle: &Handle,
    name: &str,
    command: &str,
    args: &[String],
    cwd: &Path,
    wiring: Wiring,
) -> Result<Process, PluginError> {
    let logs_tx = wiring.logs_tx.clone();
    let mut child = {
        // `Command::spawn` registers the child with the calling thread's tokio reactor, so it
        // needs `handle` bound as current for the duration of the call: this runs outside any
        // async context, once at startup and again whenever a plugin is started anew.
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
    let mailbox = Arc::new(Mailbox::default());
    let life = Life::new();
    let pending = Arc::new(Pending::default());
    let serve = wiring.serve.clone();

    handle.spawn(write_loop(stdin, stdin_rx, Arc::clone(&mailbox)));
    handle.spawn(read_loop(
        name.to_string(),
        stdout,
        stdin_tx.clone(),
        Arc::clone(&pending),
        wiring,
        handle.clone(),
    ));
    handle.spawn(reap(name.to_string(), child, logs_tx, Arc::clone(&life)));

    let init = protocol::HostEvent::new(protocol::EVENT_INIT, cwd.to_path_buf(), Vec::new());
    let _ = stdin_tx.send(init.to_line());
    // After `init`, so the first thing the process reads is still the event it expects.
    if let Some(serve) = serve {
        serve.attach(stdin_tx.clone(), pending, Arc::clone(&life));
    }

    Ok(Process {
        stdin_tx,
        mailbox,
        life,
    })
}

/// Runs one plugin-requested `file_ops` call synchronously against the local filesystem — always
/// called on the blocking pool, never inline on an async task.
fn execute(request: PluginRequest) -> Result<serde_json::Value, String> {
    let vfs = LocalVfs;
    match request {
        // `read_loop` answers these itself and never sends them here.
        PluginRequest::GetState
        | PluginRequest::RegisterCommand { .. }
        | PluginRequest::UnregisterCommand { .. }
        | PluginRequest::RunAction { .. }
        | PluginRequest::ListActions
        | PluginRequest::ShowPicker(_)
        | PluginRequest::Ask(_)
        | PluginRequest::Walk(_)
        | PluginRequest::ReadFile(_)
        | PluginRequest::Stat(_)
        | PluginRequest::Tally(_) => Err("answered by the host's reader, not here".to_string()),
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

/// Answers a `read_file`: what is at the path, and the bytes asked for when it is a regular file.
/// Anything else is described but never opened, since opening a pipe with no writer blocks
/// forever and a device has no meaningful first bytes.
fn read_chunk(vfs: &dyn Vfs, request: &ReadFile) -> Result<serde_json::Value, String> {
    let meta = vfs.metadata(&request.path).map_err(|e| e.to_string())?;
    let kind = match meta.kind {
        FileKind::File => ChunkKind::File,
        FileKind::Dir => ChunkKind::Dir,
        FileKind::Symlink | FileKind::Other => ChunkKind::Other,
    };
    let wanted = request.len.min(MAX_READ_BYTES);
    let mut data = Vec::new();
    if kind == ChunkKind::File && wanted > 0 {
        let mut file = vfs.open_read(&request.path).map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(request.offset))
            .map_err(|e| e.to_string())?;
        file.take(wanted)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
    }
    serde_json::to_value(FileChunk {
        kind,
        size: meta.len,
        modified: meta
            .modified
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since| since.as_secs()),
        local_path: vfs.local_path(&request.path),
        data,
    })
    .map_err(|e| e.to_string())
}

/// Answers a `stat`: the entry itself, not what a final symlink points at, with every field the
/// backend has. Owner and group stay numbers; the plugin names them only when `local_path` says
/// they are this machine's.
fn stat_entry(vfs: &dyn Vfs, request: &Stat) -> Result<serde_json::Value, String> {
    let path = request.path.as_path();
    let meta = vfs.symlink_metadata(path).map_err(|e| e.to_string())?;
    let kind = match meta.kind {
        FileKind::File => StatKind::File,
        FileKind::Dir => StatKind::Dir,
        FileKind::Symlink => StatKind::Symlink,
        FileKind::Other => StatKind::Other,
    };
    let is_link = kind == StatKind::Symlink;
    let seconds = |time: Option<std::time::SystemTime>| {
        time.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since| since.as_secs())
    };
    serde_json::to_value(StatReply {
        kind,
        size: meta.len,
        link_target: is_link.then(|| vfs.read_link(path).ok()).flatten(),
        link_broken: is_link && !vfs.exists(path),
        allocated: meta.allocated,
        mode: meta.mode,
        uid: meta.uid,
        gid: meta.gid,
        nlink: meta.nlink,
        modified: seconds(meta.modified),
        accessed: seconds(meta.accessed),
        created: seconds(meta.created),
        local_path: vfs.local_path(path),
    })
    .map_err(|e| e.to_string())
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
        assert!(!manager.dispatch_inspect(Some(Path::new("/a"))));
        assert!(!manager.inspector_running());
        assert!(!manager.has_canvas(Surface::Atlas));
        assert!(!manager.canvas_running(Surface::Wiring));
        assert!(!manager.dispatch_canvas(Surface::Atlas, CanvasInput::Close));
        assert!(manager.names().is_empty());
        assert!(manager.is_empty());
        assert!(manager.take_action().is_none());
    }

    fn chunk(vfs: &dyn Vfs, path: &str, offset: u64, len: u64) -> Result<FileChunk, String> {
        let request = ReadFile {
            path: PathBuf::from(path),
            offset,
            len,
        };
        read_chunk(vfs, &request)
            .map(|value| serde_json::from_value(value).expect("the reply is a FileChunk"))
    }

    #[test]
    fn read_file_returns_the_range_asked_for_and_the_whole_length() {
        let vfs = shared::MemVfs::new();
        vfs.add_file("/f.txt", b"0123456789".to_vec());

        let middle = chunk(&vfs, "/f.txt", 3, 4).expect("readable");
        assert_eq!(middle.kind, ChunkKind::File);
        assert_eq!((middle.size, middle.data.as_slice()), (10, &b"3456"[..]));

        let past_the_end = chunk(&vfs, "/f.txt", 8, 100).expect("readable");
        assert_eq!(past_the_end.data, b"89");
        let beyond = chunk(&vfs, "/f.txt", 50, 5).expect("readable");
        assert!(beyond.data.is_empty());
    }

    fn stat(vfs: &dyn Vfs, path: &str) -> Result<StatReply, String> {
        let request = Stat {
            path: PathBuf::from(path),
        };
        stat_entry(vfs, &request).map(|value| serde_json::from_value(value).expect("a reply"))
    }

    #[test]
    fn stat_describes_a_file_and_a_folder_with_what_the_backend_has() {
        let vfs = shared::MemVfs::new();
        vfs.add_dir("/d");
        vfs.add_file("/d/f", vec![0u8; 5000]);

        let file = stat(&vfs, "/d/f").expect("described");
        assert_eq!((file.kind, file.size), (StatKind::File, 5000));
        assert_eq!((file.allocated, file.nlink), (Some(8192), Some(1)));
        assert_eq!(
            file.local_path, None,
            "a memory backend is not the local disk"
        );
        assert_eq!(stat(&vfs, "/d").expect("described").kind, StatKind::Dir);
        assert!(stat(&vfs, "/nowhere").is_err());
    }

    #[test]
    fn stat_describes_a_link_as_a_link_and_says_when_it_is_broken() {
        let vfs = shared::MemVfs::new();
        vfs.add_file("/d/real", b"x".to_vec());
        vfs.add_symlink("/d/ok", "/d/real");
        vfs.add_symlink("/d/dangling", "/d/missing");

        let ok = stat(&vfs, "/d/ok").expect("described");
        assert_eq!(ok.kind, StatKind::Symlink);
        assert_eq!(ok.link_target, Some(PathBuf::from("/d/real")));
        assert!(!ok.link_broken);
        let dangling = stat(&vfs, "/d/dangling").expect("described");
        assert_eq!(dangling.link_target, Some(PathBuf::from("/d/missing")));
        assert!(dangling.link_broken);
    }

    #[test]
    fn stat_on_the_local_disk_names_the_same_path_for_the_plugin() {
        let dir = std::env::temp_dir().join(format!("mman-stat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let reply = stat(&LocalVfs, &dir.to_string_lossy()).expect("described");
        assert_eq!(reply.local_path, Some(dir.clone()));
        assert!(reply.uid.is_some() && reply.modified.is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_file_with_no_length_only_describes_the_path() {
        let vfs = shared::MemVfs::new();
        vfs.add_dir("/d");
        vfs.add_file("/d/f", b"abc".to_vec());

        let file = chunk(&vfs, "/d/f", 0, 0).expect("described");
        assert_eq!(
            (file.kind, file.size, file.data.len()),
            (ChunkKind::File, 3, 0)
        );
        let dir = chunk(&vfs, "/d", 0, 4096).expect("described");
        assert_eq!(dir.kind, ChunkKind::Dir);
        assert!(dir.data.is_empty(), "a folder is never read");
        assert!(chunk(&vfs, "/nowhere", 0, 1).is_err());
    }

    #[test]
    fn read_file_never_returns_more_than_the_cap_in_one_reply() {
        let vfs = shared::MemVfs::new();
        vfs.add_file("/big", vec![7u8; (MAX_READ_BYTES + 10) as usize]);
        let reply = chunk(&vfs, "/big", 0, u64::MAX).expect("readable");
        assert_eq!(reply.data.len() as u64, MAX_READ_BYTES);
        assert_eq!(reply.size, MAX_READ_BYTES + 10);
    }

    #[test]
    fn a_previewer_is_only_found_when_one_provides_preview() {
        let mut manager = PluginManager::new();
        assert!(!manager.has_previewer());
        assert!(!manager.previewer_running());
        assert!(
            !manager.dispatch_preview(Some(Path::new("/a"))),
            "with nobody to tell, the interface has to know"
        );
    }

    #[test]
    fn publishing_the_same_state_twice_reports_a_change_only_the_first_time() {
        let mut manager = PluginManager::new();
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
