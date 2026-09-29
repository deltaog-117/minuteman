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

//! Clipboard + prompt + background-operation state driving `file_ops` from the TUI. Keeps
//! `main.rs`'s event loop a thin `Action -> App method` dispatcher, the same way
//! `browser::BrowserState` keeps it thin for navigation.
//!
//! Paste and delete run on `tokio`'s blocking thread pool (via `Handle::spawn_blocking`) rather
//! than inline, so a large copy/move/delete never freezes the render loop. Rename and create
//! stay synchronous — both are single, near-instant metadata operations (rename never crosses
//! filesystems: `file_ops::rename` always targets the same parent directory).

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::Result;
use browser::BrowserState;
use browser::search::{Outcome as SearchOutcome, Query};
use crossterm::event::KeyCode;
use file_ops::archive::{self, ExtractLimits, Format, Level};
use file_ops::{ConflictPolicy, FileOpsError, Outcome};
use plugins::PluginManager;
use shared::{LocalVfs, Vfs, VfsError};
use shell_overlay::CommandOutcome;
use theming::{OpenRule, OpenWith, PluginSpec};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::associations::{self, Associations};
use crate::command::{self, Command};
use crate::compress_popup::{self, CompressPopup};
use crate::disk_usage::DiskUsageView;
use crate::extract_popup::{self, ExtractPopup};
use crate::git_status::{GitStatus, Repo};
use crate::inspect::InspectView;
use crate::live_refresh::LiveRefresh;
use crate::marked_size::{MarkedSize, Total};
use crate::open;
use crate::search_job::{SearchJob, SearchState};
use crate::system_hud::SystemHud;

/// How many lines of a shell command's output the status line shows before summarizing the rest.
const STATUS_OUTPUT_LINES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardMode {
    Copy,
    Move,
}

#[derive(Debug, Clone)]
pub struct Clipboard {
    pub paths: Vec<PathBuf>,
    pub mode: ClipboardMode,
}

#[derive(Debug)]
pub enum ConflictSource {
    /// `index` is the position in `clip.paths` that hit the conflict; the caller resumes the
    /// batch at `index` (retry) or `index + 1` (skip) once the user answers.
    Paste {
        clip: Clipboard,
        dst_dir: PathBuf,
        index: usize,
        dst: PathBuf,
    },
    Rename {
        target: PathBuf,
        new_name: String,
    },
}

/// Where a `/` search started, so `Esc` (or an emptied query) can put the browser back: the
/// directory it was in and the cursor's index there. A search can carry the browser into another
/// directory, so the index alone is no longer enough.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOrigin {
    pub dir: PathBuf,
    pub index: usize,
}

/// A command waiting for the terminal: `main` suspends the interface, runs `line` under `sh -c`
/// in `cwd`, and hands the result back through `App::finish_handover`. The app can't do that
/// itself because it doesn't own the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handover {
    pub line: String,
    pub cwd: PathBuf,
}

#[derive(Debug)]
pub enum Prompt {
    RenameInput {
        target: PathBuf,
        buffer: String,
    },
    CreateInput {
        buffer: String,
    },
    /// Confirms a permanent delete of one or more targets — either the single cursor entry, or
    /// every currently marked path if any are marked (Ranger-style: marks win over the cursor).
    ConfirmDelete {
        targets: Vec<PathBuf>,
    },
    /// Confirms sending one or more targets to the desktop trash (same marks-win-over-cursor
    /// batching as `ConfirmDelete`). Reversible, so `Enter` confirms it as well as `y`.
    ConfirmTrash {
        targets: Vec<PathBuf>,
    },
    Conflict(ConflictSource),
    /// Incremental filename search (`/`). `origin` is where to return to on `Esc`.
    SearchInput {
        buffer: String,
        origin: SearchOrigin,
    },
    /// The `:`-command prompt (`:q`, `:cd <path>`, `:mkdir`, `:touch`, or any shell command).
    CommandInput {
        buffer: String,
    },
    /// The command typed for the "Open with" menu's "Other…": what should open `path`.
    OpenOther {
        path: PathBuf,
        buffer: String,
    },
}

impl Prompt {
    /// The status bar's mode label for this prompt.
    pub fn label(&self) -> &'static str {
        match self {
            Prompt::RenameInput { .. } => "RENAME",
            Prompt::CreateInput { .. } => "CREATE",
            Prompt::ConfirmDelete { .. } => "DELETE",
            Prompt::ConfirmTrash { .. } => "TRASH",
            Prompt::Conflict(_) => "CONFLICT",
            Prompt::SearchInput { .. } => "SEARCH",
            Prompt::CommandInput { .. } => "COMMAND",
            Prompt::OpenOther { .. } => "OPEN WITH",
        }
    }

    /// Whether the user is typing into a buffer (so a cursor belongs at the end of the text),
    /// as opposed to answering a one-key question.
    pub fn is_text_input(&self) -> bool {
        matches!(
            self,
            Prompt::RenameInput { .. }
                | Prompt::CreateInput { .. }
                | Prompt::SearchInput { .. }
                | Prompt::CommandInput { .. }
                | Prompt::OpenOther { .. }
        )
    }

    /// Whether confirming this prompt is destructive (permanent delete, overwrite).
    pub fn is_destructive(&self) -> bool {
        matches!(self, Prompt::ConfirmDelete { .. } | Prompt::Conflict(_))
    }

    pub fn display(&self) -> String {
        match self {
            Prompt::RenameInput { buffer, .. } => format!("rename: {buffer}"),
            Prompt::CreateInput { buffer } => {
                format!("create (end with / for a directory): {buffer}")
            }
            Prompt::ConfirmDelete { targets } if targets.len() == 1 => {
                format!("delete '{}' permanently? (y/N)", display_name(&targets[0]))
            }
            Prompt::ConfirmDelete { targets } => {
                format!("delete {} marked items permanently? (y/N)", targets.len())
            }
            Prompt::ConfirmTrash { targets } if targets.len() == 1 => {
                format!("trash '{}'? (enter/y)", display_name(&targets[0]))
            }
            Prompt::ConfirmTrash { targets } => {
                format!("trash {} marked items? (enter/y)", targets.len())
            }
            Prompt::Conflict(ConflictSource::Paste { dst, .. }) => format!(
                "'{}' already exists — overwrite / skip / abort? (o/s/a)",
                display_name(dst)
            ),
            Prompt::Conflict(ConflictSource::Rename { new_name, .. }) => {
                format!("'{new_name}' already exists — overwrite / skip / abort? (o/s/a)")
            }
            Prompt::SearchInput { buffer, .. } => format!("/{buffer}"),
            Prompt::CommandInput { buffer } => format!(":{buffer}"),
            Prompt::OpenOther { path, buffer } => {
                format!("open {} with: {buffer}", display_name(path))
            }
        }
    }
}

/// What a running background operation is doing, and what it needs to retry with if it turns
/// out to hit a conflict (only `Paste` can — `Delete` never has an `AlreadyExists` case).
///
/// `Paste` processes `clip.paths` one item at a time — `index` is the item currently in flight,
/// `dst` its destination under `dst_dir`. `poll_bulk` advances `index` and re-spawns the next
/// item itself once the current one finishes, so a multi-item batch stays a single continuous
/// "busy" operation from the UI's perspective rather than N separate ones.
#[derive(Debug)]
enum BulkKind {
    Paste {
        clip: Clipboard,
        dst_dir: PathBuf,
        index: usize,
        dst: PathBuf,
    },
    Delete {
        targets: Vec<PathBuf>,
    },
    Trash {
        targets: Vec<PathBuf>,
    },
    /// Unpacking archives, each into its own directory.
    Extract,
    /// Packing into one archive.
    Compress,
    /// A `:` command line running under `sh -c`, from `started`.
    Shell {
        command: String,
        started: Instant,
    },
}

impl BulkKind {
    fn progressing_label(&self) -> &'static str {
        match self {
            BulkKind::Paste { clip, .. } => match clip.mode {
                ClipboardMode::Copy => "copying",
                ClipboardMode::Move => "moving",
            },
            BulkKind::Delete { .. } => "deleting",
            BulkKind::Trash { .. } => "trashing",
            BulkKind::Extract => "extracting",
            BulkKind::Compress => "compressing",
            BulkKind::Shell { .. } => "running",
        }
    }

    fn past_label(&self) -> &'static str {
        match self {
            BulkKind::Paste { clip, .. } => match clip.mode {
                ClipboardMode::Copy => "copy",
                ClipboardMode::Move => "move",
            },
            BulkKind::Delete { .. } => "delete",
            BulkKind::Trash { .. } => "trash",
            BulkKind::Extract => "extract",
            BulkKind::Compress => "compress",
            BulkKind::Shell { .. } => "command",
        }
    }
}

/// One archive to write: `sources` packed into `out`.
struct CompressJob {
    sources: Vec<PathBuf>,
    out: PathBuf,
}

/// One archive to unpack: `archive` into the directory `dest`.
struct ExtractJob {
    archive: PathBuf,
    dest: PathBuf,
}

enum BulkMsg {
    Progress { path: String },
    Done(Result<Outcome, FileOpsError>),
    ShellDone(std::io::Result<CommandOutcome>),
}

/// A background operation in flight. `cancel` is `None` for `Delete`, since
/// `Vfs::remove_dir_all` is one opaque blocking call with no per-item hook to check against.
/// A `Shell` command is cancelled by killing its shell.
struct BulkOp {
    kind: BulkKind,
    items_done: u64,
    current: String,
    cancel: Option<Arc<AtomicBool>>,
    rx: UnboundedReceiver<BulkMsg>,
}

/// A snapshot of the running background operation, for the header's progress gauge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub label: &'static str,
    /// Items finished so far (files and directories, as `file_ops` reports them).
    pub done: u64,
    /// `(index of the item in flight, total items)` for a multi-item paste; `None` otherwise,
    /// since a single directory's total size isn't known up front.
    pub batch: Option<(usize, usize)>,
}

pub struct App {
    pub clipboard: Option<Clipboard>,
    pub prompt: Option<Prompt>,
    pub status: Option<String>,
    bulk: Option<BulkOp>,
    live: LiveRefresh,
    /// What the marked entries add up to, for the header pill (see `marked_size`).
    marked_size: MarkedSize,
    /// The browsed directory's repository, for the status bar (see `git_status`).
    git: GitStatus,
    /// The header's system segment (see `system_hud`), when `[config] system_hud` turns it on.
    system: Option<SystemHud>,
    handle: tokio::runtime::Handle,
    /// Program names a `:` command hands the terminal to (see `command::parse`).
    interactive: Vec<String>,
    /// The `[[open_rule]]` tables: which program opens which files, ahead of the desktop's choice.
    open_rules: Vec<OpenRule>,
    /// The hand-written `[[open_with]]` entries the menu lists.
    open_with: Vec<OpenWith>,
    /// What the desktop has registered for each file type, read on first use rather than at
    /// startup, since most sessions never open a file.
    associations: std::cell::OnceCell<Associations>,
    handover: Option<Handover>,
    /// The `/` prompt's search in flight, if any. Only ever `Some` while that prompt is open.
    search_job: Option<SearchJob>,
    search_state: SearchState,
    plugins: PluginManager,
    /// Whether the last key event carried the keyboard protocol's Caps Lock flag, for the header
    /// pill (see `main`'s `with_caps_lock_applied`, which reads the same flag to fix up a
    /// letter's case). Only ever known while the terminal reports it; stays `false` otherwise.
    caps_lock: bool,
}

impl App {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self {
            clipboard: None,
            prompt: None,
            status: None,
            bulk: None,
            live: LiveRefresh::new(handle.clone()),
            marked_size: MarkedSize::new(handle.clone()),
            git: GitStatus::new(handle.clone(), true),
            system: None,
            handle,
            interactive: Vec::new(),
            open_rules: Vec::new(),
            open_with: Vec::new(),
            associations: std::cell::OnceCell::new(),
            handover: None,
            search_job: None,
            search_state: SearchState::Idle,
            plugins: PluginManager::new(),
            caps_lock: false,
        }
    }

    /// Opens the disk usage view on `path`, starting its scan on the blocking pool.
    pub fn begin_disk_usage(&self, path: PathBuf) -> DiskUsageView {
        DiskUsageView::open(self.handle.clone(), path)
    }

    /// Turns the status bar's git segment on or off (`git_status` in `config.toml`).
    pub fn with_git_status(mut self, enabled: bool) -> Self {
        self.git = GitStatus::new(self.handle.clone(), enabled);
        self
    }

    /// Turns the header's system segment on (`[config] system_hud`), starting its clock now.
    pub fn with_system_hud(mut self, enabled: bool) -> Self {
        self.system = enabled.then(|| SystemHud::new(Instant::now()));
        self
    }

    /// Sets which program names `:` hands the terminal to.
    pub fn with_interactive_commands(mut self, interactive: Vec<String>) -> Self {
        self.interactive = interactive;
        self
    }

    /// Sets the `[[open_rule]]` and `[[open_with]]` tables.
    pub fn with_open_config(mut self, rules: Vec<OpenRule>, open_with: Vec<OpenWith>) -> Self {
        self.open_rules = rules;
        self.open_with = open_with;
        self
    }

    /// Starts every configured `[[plugin]]` process (see `plugins`), each firing its `init`
    /// event immediately and its `key` event whenever its own `on_key` is pressed. A plugin that
    /// fails to spawn (bad command, missing binary) is skipped with a warning on stderr rather
    /// than treated as fatal — the same "one bad entry must never take the whole session down"
    /// rule `theming::Config::load` already follows for a malformed config file.
    pub fn with_plugins(mut self, specs: Vec<PluginSpec>, cwd: &Path) -> Self {
        for spec in specs {
            if let Err(e) = self.plugins.spawn(
                &self.handle,
                &spec.name,
                &spec.command,
                &spec.args,
                spec.on_key,
                cwd,
            ) {
                eprintln!("minuteman: {e}");
            }
        }
        self
    }

    pub fn status_line(&self) -> String {
        if let Some(bulk) = &self.bulk {
            return match &bulk.kind {
                BulkKind::Delete { targets } | BulkKind::Trash { targets } => {
                    format!(
                        "{}… {}",
                        bulk.kind.progressing_label(),
                        batch_label(targets)
                    )
                }
                BulkKind::Extract | BulkKind::Compress => format!(
                    "{}… {} — Esc to cancel",
                    bulk.kind.progressing_label(),
                    bulk.current
                ),
                BulkKind::Shell { command, .. } => {
                    format!(
                        "{}… {command} — Esc to cancel",
                        bulk.kind.progressing_label()
                    )
                }
                BulkKind::Paste { clip, index, .. } => {
                    let cancel_hint = if bulk.cancel.is_some() {
                        " — Esc to cancel"
                    } else {
                        ""
                    };
                    let batch_hint = if clip.paths.len() > 1 {
                        format!(" [{}/{}]", index + 1, clip.paths.len())
                    } else {
                        String::new()
                    };
                    format!(
                        "{}… {} done ({}){batch_hint}{cancel_hint}",
                        bulk.kind.progressing_label(),
                        bulk.items_done,
                        bulk.current
                    )
                }
            };
        }
        match &self.prompt {
            Some(prompt @ Prompt::SearchInput { .. }) => match self.search_state.note() {
                Some(note) => format!("{note} {}", prompt.display()),
                None => prompt.display(),
            },
            Some(prompt) => prompt.display(),
            None => self.status.clone().unwrap_or_default(),
        }
    }

    pub fn is_busy(&self) -> bool {
        self.bulk.is_some()
    }

    pub fn progress(&self) -> Option<Progress> {
        let bulk = self.bulk.as_ref()?;
        let batch = match &bulk.kind {
            BulkKind::Paste { clip, index, .. } if clip.paths.len() > 1 => {
                Some((*index, clip.paths.len()))
            }
            _ => None,
        };
        // A command has no item count, so its pill counts elapsed seconds instead — which also
        // keeps the gauge moving while it runs.
        let done = match &bulk.kind {
            BulkKind::Shell { started, .. } => started.elapsed().as_secs(),
            _ => bulk.items_done,
        };
        Some(Progress {
            label: bulk.kind.progressing_label(),
            done,
            batch,
        })
    }

    /// Keeps the browser's lists in step with changes made on disk behind its back (see
    /// `live_refresh`). Call once per render tick; returns whether the selected file itself
    /// changed, so its preview needs re-reading.
    pub fn poll_disk(&mut self, browser: &mut BrowserState, vfs: &LocalVfs, now: Instant) -> bool {
        self.live.poll(browser, vfs, now)
    }

    /// Keeps the figures the HUD shows about the browsed directory and the marks up to date: the
    /// marked entries' total size and the repository's git status. Both run their work on the
    /// blocking pool (see `marked_size` and `git_status`); call once per render tick.
    pub fn poll_hud(&mut self, browser: &BrowserState, now: Instant) {
        self.marked_size.poll(&browser.marked_paths());
        self.git.poll(browser.current_dir(), now);
        if let Some(system) = self.system.as_mut() {
            system.tick(now);
        }
    }

    /// Surfaces every plugin's pending `log` line as the status message (last one wins, the same
    /// way every other status write already behaves). Call once per render tick.
    pub fn poll_plugins(&mut self) {
        for message in self.plugins.poll() {
            self.status = Some(format!("[{}] {}", message.plugin, message.message));
        }
    }

    /// Fires the plugin bound to `code`, if any is configured, with the current marks (or the
    /// single selected entry — see `marked_or_selected`) and the browsed directory. Returns
    /// whether a plugin consumed the key, so a truly unbound key stays a no-op.
    pub fn dispatch_plugin_key(&mut self, code: KeyCode, browser: &BrowserState) -> bool {
        let selection = Self::marked_or_selected(browser);
        self.plugins
            .dispatch_key(code, browser.current_dir(), &selection)
    }

    /// What the marks add up to, once a walk has finished.
    pub fn marked_total(&self) -> Option<Total> {
        self.marked_size.total()
    }

    /// The repository the browsed directory is in, when git answered.
    pub fn git_repo(&self) -> Option<&Repo> {
        self.git.repo()
    }

    /// The header's system segment text, when `[config] system_hud` is on.
    pub fn system_summary(&self, now: Instant) -> Option<String> {
        self.system.as_ref().map(|system| system.summary(now))
    }

    /// Records whether the key event just read carried the keyboard protocol's Caps Lock flag.
    /// Call on every key event (not just the ones that reach a binding), so the header pill
    /// stays correct even while a modal view or prompt owns the keyboard.
    pub fn set_caps_lock(&mut self, on: bool) {
        self.caps_lock = on;
    }

    /// Whether Caps Lock is currently on, for the header pill.
    pub fn caps_lock(&self) -> bool {
        self.caps_lock
    }

    /// Drains any progress/completion messages from the running background operation, if any.
    /// Call once per render tick.
    pub fn poll_bulk(&mut self, browser: &mut BrowserState, vfs: &dyn Vfs) -> Result<()> {
        let Some(bulk) = self.bulk.as_mut() else {
            return Ok(());
        };

        let mut done = None;
        let mut shell_done = None;
        while let Ok(msg) = bulk.rx.try_recv() {
            match msg {
                BulkMsg::Progress { path } => {
                    bulk.items_done += 1;
                    bulk.current = path;
                }
                BulkMsg::Done(result) => done = Some(result),
                BulkMsg::ShellDone(result) => shell_done = Some(result),
            }
        }

        if let Some(result) = shell_done {
            let bulk = self.bulk.take().expect("checked Some above");
            // Whatever the command did to the directory, show it now rather than on the next
            // background refresh; it may also have deleted marked paths.
            browser.reload(vfs)?;
            browser.prune_marks(vfs);
            if let BulkKind::Shell { command, .. } = bulk.kind {
                self.status = Some(match result {
                    Ok(outcome) => shell_status(&command, &outcome),
                    Err(e) => format!("could not run '{command}': {e}"),
                });
            }
            return Ok(());
        }

        let Some(result) = done else {
            return Ok(());
        };
        let bulk = self.bulk.take().expect("checked Some above");
        let past_label = bulk.kind.past_label();

        // Delete and Trash both remove entries from the listing outright (no conflict case,
        // unlike Paste), so they share every branch below that Paste doesn't.
        // A compress can also remove entries (it may trash the originals), so it is treated the
        // same way to keep the listing and the marks in step.
        let is_delete = matches!(
            bulk.kind,
            BulkKind::Delete { .. } | BulkKind::Trash { .. } | BulkKind::Compress
        );
        // Each item of a batch gets its own cancel flag, so a cancel that lands just as an item
        // finishes would otherwise be forgotten and the batch would carry on with the next one.
        let cancelled = bulk
            .cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed));

        match result {
            Ok(Outcome::Completed | Outcome::Skipped) if cancelled => {
                browser.reload(vfs)?;
                self.status = Some(format!("{past_label} cancelled"));
            }
            Ok(Outcome::Completed) => {
                browser.reload(vfs)?;
                if let BulkKind::Paste {
                    clip,
                    dst_dir,
                    index,
                    ..
                } = bulk.kind
                {
                    let Some(clip) = self.try_continue_paste(clip, dst_dir, index) else {
                        return Ok(());
                    };
                    // Only the cut this paste came from: a drop moves files without ever
                    // touching the clipboard, and must not empty one the user filled meanwhile.
                    if clip.mode == ClipboardMode::Move
                        && self
                            .clipboard
                            .as_ref()
                            .is_some_and(|held| held.paths == clip.paths)
                    {
                        self.clipboard = None;
                    }
                    if clip.mode == ClipboardMode::Move {
                        browser.prune_marks(vfs);
                    }
                } else if is_delete {
                    browser.prune_marks(vfs);
                }
                self.status = Some(format!("{past_label} complete"));
            }
            Ok(Outcome::Skipped) => {
                if let BulkKind::Paste {
                    clip,
                    dst_dir,
                    index,
                    ..
                } = bulk.kind
                    && self.try_continue_paste(clip, dst_dir, index).is_none()
                {
                    return Ok(());
                }
                self.status = Some(format!("{past_label} skipped"));
            }
            Err(FileOpsError::Cancelled) => self.status = Some(format!("{past_label} cancelled")),
            Err(FileOpsError::Vfs(VfsError::AlreadyExists(existing))) => match bulk.kind {
                BulkKind::Paste {
                    clip,
                    dst_dir,
                    index,
                    dst,
                } => {
                    self.prompt = Some(Prompt::Conflict(ConflictSource::Paste {
                        clip,
                        dst_dir,
                        index,
                        dst,
                    }));
                }
                // No prompt for these: overwriting could clobber a whole extracted tree, so the
                // user is told what is in the way and can remove or rename it.
                BulkKind::Extract | BulkKind::Compress => {
                    self.status = Some(format!(
                        "{past_label} failed: {} already exists",
                        existing.display()
                    ));
                }
                BulkKind::Delete { .. } | BulkKind::Trash { .. } | BulkKind::Shell { .. } => {
                    self.status = Some(format!("{past_label} failed: unexpected conflict"));
                }
            },
            // A delete can fail partway through a multi-target batch, having already removed
            // some marked paths from disk — reload/prune so the browser reflects that instead
            // of showing stale entries and stale marks alongside the failure message.
            Err(e) => {
                if is_delete {
                    browser.reload(vfs)?;
                    browser.prune_marks(vfs);
                }
                self.status = Some(format!("{past_label} failed: {e}"));
            }
        }
        Ok(())
    }

    /// Requests cancellation of the running background operation, if it supports it.
    pub fn cancel_bulk(&mut self) {
        match self.bulk.as_ref().and_then(|b| b.cancel.as_ref()) {
            Some(cancel) => {
                cancel.store(true, Ordering::Relaxed);
                self.status = Some("cancelling…".into());
            }
            None => self.status = Some("nothing cancellable in progress".into()),
        }
    }

    /// Cancels everything pending in one go, from any directory: the yanked or cut clipboard,
    /// every mark, and a running copy/move/command. A cut only takes effect when pasted, so
    /// forgetting one never touches disk. A running delete can't be interrupted (see `BulkOp`),
    /// and the status line says so rather than pretending.
    pub fn cancel_all(&mut self, browser: &mut BrowserState) {
        let mut cancelled = Vec::new();
        let mut uncancellable = false;
        if let Some(bulk) = &self.bulk {
            match &bulk.cancel {
                Some(flag) => {
                    flag.store(true, Ordering::Relaxed);
                    cancelled.push(format!("running {}", bulk.kind.past_label()));
                }
                None => uncancellable = true,
            }
        }
        if let Some(clip) = self.clipboard.take() {
            let verb = match clip.mode {
                ClipboardMode::Copy => "yank",
                ClipboardMode::Move => "cut",
            };
            cancelled.push(format!("{verb} of {}", batch_label(&clip.paths)));
        }
        match browser.clear_marks() {
            0 => {}
            1 => cancelled.push("1 mark".into()),
            marks => cancelled.push(format!("{marks} marks")),
        }

        self.status = Some(match (cancelled.is_empty(), uncancellable) {
            (true, false) => "nothing to cancel".into(),
            (true, true) => "a running delete cannot be cancelled".into(),
            (false, false) => format!("cancelled: {}", cancelled.join(", ")),
            (false, true) => format!(
                "cancelled: {} (a running delete cannot be)",
                cancelled.join(", ")
            ),
        });
    }

    /// Snapshots the current batch: every marked path if any are marked, otherwise just the
    /// entry under the cursor. Ranger-style "act on marks if any, else the current file"
    /// convention, shared by yank/cut/delete so all three batch the same way.
    fn marked_or_selected(browser: &BrowserState) -> Vec<PathBuf> {
        match browser.marked_paths() {
            marked if !marked.is_empty() => marked,
            _ => browser
                .selected_entry()
                .map(|entry| vec![entry.path.clone()])
                .unwrap_or_default(),
        }
    }

    pub fn yank(&mut self, browser: &BrowserState) {
        let paths = Self::marked_or_selected(browser);
        if paths.is_empty() {
            self.status = Some("nothing selected".into());
            return;
        }
        self.status = Some(format!("yanked {}", batch_label(&paths)));
        self.clipboard = Some(Clipboard {
            paths,
            mode: ClipboardMode::Copy,
        });
    }

    pub fn cut(&mut self, browser: &BrowserState) {
        let paths = Self::marked_or_selected(browser);
        if paths.is_empty() {
            self.status = Some("nothing selected".into());
            return;
        }
        self.status = Some(format!("marked {} to move", batch_label(&paths)));
        self.clipboard = Some(Clipboard {
            paths,
            mode: ClipboardMode::Move,
        });
    }

    /// Marks (via `Select`) win over the cursor: if any entries are marked, trash confirms
    /// against the whole marked set; otherwise it falls back to the single entry under the
    /// cursor, matching Ranger's "act on marks if any, else the current file" convention.
    pub fn begin_trash(&mut self, browser: &BrowserState) {
        let targets = Self::marked_or_selected(browser);
        if targets.is_empty() {
            self.status = Some("nothing selected".into());
            return;
        }
        self.prompt = Some(Prompt::ConfirmTrash { targets });
    }

    /// Same batching as `begin_trash`, but for a permanent, trash-bypassing delete.
    pub fn begin_delete_permanently(&mut self, browser: &BrowserState) {
        let targets = Self::marked_or_selected(browser);
        if targets.is_empty() {
            self.status = Some("nothing selected".into());
            return;
        }
        self.prompt = Some(Prompt::ConfirmDelete { targets });
    }

    pub fn begin_rename(&mut self, browser: &BrowserState) {
        match browser.selected_entry() {
            Some(entry) => {
                self.prompt = Some(Prompt::RenameInput {
                    target: entry.path.clone(),
                    buffer: entry.name.clone(),
                });
            }
            None => self.status = Some("nothing selected".into()),
        }
    }

    pub fn begin_create(&mut self) {
        self.prompt = Some(Prompt::CreateInput {
            buffer: String::new(),
        });
    }

    pub fn begin_search(&mut self, browser: &BrowserState) {
        self.end_search();
        self.prompt = Some(Prompt::SearchInput {
            buffer: String::new(),
            origin: SearchOrigin {
                dir: browser.current_dir().to_path_buf(),
                index: browser.selected_index(),
            },
        });
    }

    pub fn begin_command(&mut self) {
        self.prompt = Some(Prompt::CommandInput {
            buffer: String::new(),
        });
    }

    pub fn begin_paste(&mut self, browser: &BrowserState) {
        self.begin_paste_into(browser.current_dir().to_path_buf());
    }

    /// Pastes the clipboard into `dst_dir` — the browsed directory for `Paste`, or a folder that
    /// was right-clicked for the menu's "Paste into folder".
    pub fn begin_paste_into(&mut self, dst_dir: PathBuf) {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        let Some(clip) = self.clipboard.clone() else {
            self.status = Some("clipboard is empty".into());
            return;
        };
        self.spawn_paste_item(clip, dst_dir, 0, ConflictPolicy::Abort);
    }

    /// Moves or copies `paths` into `dst_dir` for a drag and drop, through the same background
    /// paste a `p` does (progress, cancel, and the overwrite/skip/abort prompt on a conflict).
    /// The yank/cut clipboard is left alone: a drop is not a paste of what the user copied.
    pub fn begin_drop(&mut self, paths: Vec<PathBuf>, mode: ClipboardMode, dst_dir: PathBuf) {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        self.spawn_paste_item(Clipboard { paths, mode }, dst_dir, 0, ConflictPolicy::Abort);
    }

    /// Opens the Inspect panel for `path`, or says on the status line why it cannot.
    pub fn begin_inspect(&mut self, path: &Path) -> Option<InspectView> {
        match InspectView::open(&self.handle, path) {
            Ok(view) => Some(view),
            Err(message) => {
                self.status = Some(message);
                None
            }
        }
    }

    fn associations(&self) -> &Associations {
        self.associations.get_or_init(Associations::from_env)
    }

    /// Every choice the "Open with" submenu offers for `path`, best first: matching
    /// `[[open_rule]]`s, the `[[open_with]]` entries, then the programs installed for its type.
    pub fn open_choices(&self, path: &Path) -> Vec<OpenWith> {
        let assoc = self.associations();
        let mime = assoc.mime_of(path);
        let discovered = assoc
            .apps_for(&mime)
            .iter()
            .map(associations::choice)
            .collect();
        open::choices(
            &self.open_rules,
            &self.open_with,
            discovered,
            &display_name(path),
            &mime,
        )
    }

    /// Opens `path` the way the user would expect: the first `[[open_rule]]` that covers it, else
    /// the program the desktop has as the default for its type, else whatever `xdg-open` picks.
    pub fn open_default(&mut self, browser: &BrowserState, path: &Path) {
        let mime = self.associations().mime_of(path);
        let choice = open::first_rule(&self.open_rules, &display_name(path), &mime)
            .map(open::rule_choice)
            .or_else(|| {
                self.associations()
                    .default_for(&mime)
                    .map(|entry| associations::choice(&entry))
            });
        match choice {
            Some(choice) => self.open_choice(browser, &choice, path),
            None => self.open_with(browser, open::DEFAULT_OPENER, path),
        }
    }

    /// Opens `path` with one of the menu's choices, taking over the terminal if it needs it.
    pub fn open_choice(&mut self, browser: &BrowserState, choice: &OpenWith, path: &Path) {
        self.launch(browser, &choice.command, path, choice.terminal);
    }

    /// Asks for a command to open `path` with — the menu's "Other…".
    pub fn begin_open_other(&mut self, path: PathBuf) {
        self.prompt = Some(Prompt::OpenOther {
            path,
            buffer: String::new(),
        });
    }

    /// Opens `path` with `command` (see `open::command_line` for where the path goes). A program
    /// in the `interactive_commands` list takes over the terminal like a `:` command does; any
    /// other is started on its own, since it opens a window and must not be waited on.
    pub fn open_with(&mut self, browser: &BrowserState, command: &str, path: &Path) {
        self.launch(browser, command, path, false);
    }

    /// The one place a file is handed to a program. `terminal` forces the terminal handover for a
    /// program that needs it whatever its name (a `Terminal=true` desktop entry).
    fn launch(&mut self, browser: &BrowserState, command: &str, path: &Path, terminal: bool) {
        let Some(program) = open::program_word(command) else {
            self.status = Some("open with: the command is empty".into());
            return;
        };
        let program = program.as_str();
        // `VAR=value prog` is shell syntax, not a program name to look for.
        if !program.contains('=') && !open::program_exists(program) {
            self.status = Some(format!("{program}: command not found"));
            return;
        }
        let line = open::command_line(command, path);
        if terminal || command::names_interactive_program(program, &self.interactive) {
            if self.is_busy() {
                self.status = Some(format!("an operation is already in progress: {line}"));
            } else {
                self.handover = Some(Handover {
                    line,
                    cwd: browser.current_dir().to_path_buf(),
                });
            }
            return;
        }
        self.status = Some(
            match shell_overlay::spawn_detached(browser.current_dir(), &line) {
                Ok(()) => format!(
                    "opening {} with {}",
                    display_name(path),
                    Path::new(program).file_name().map_or_else(
                        || program.to_string(),
                        |name| name.to_string_lossy().into_owned()
                    )
                ),
                Err(e) => format!("cannot open {}: {e}", display_name(path)),
            },
        );
    }

    /// Routes a raw key to the active prompt. No-op if there is no active prompt. Returns
    /// `ControlFlow::Break` if a `:`-command (`:q`/`:quit`) requested the app exit.
    pub fn handle_prompt_key(
        &mut self,
        code: KeyCode,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
    ) -> Result<ControlFlow<()>> {
        let Some(prompt) = self.prompt.take() else {
            return Ok(ControlFlow::Continue(()));
        };

        let mut flow = ControlFlow::Continue(());
        match prompt {
            Prompt::RenameInput { target, mut buffer } => match code {
                KeyCode::Esc => self.status = Some("rename cancelled".into()),
                KeyCode::Enter if buffer.is_empty() => {
                    self.status = Some("rename cancelled: empty name".into());
                }
                KeyCode::Enter => {
                    self.finish_rename(vfs, browser, target, buffer, ConflictPolicy::Abort)?;
                }
                KeyCode::Backspace => {
                    buffer.pop();
                    self.prompt = Some(Prompt::RenameInput { target, buffer });
                }
                KeyCode::Char(c) => {
                    buffer.push(c);
                    self.prompt = Some(Prompt::RenameInput { target, buffer });
                }
                _ => self.prompt = Some(Prompt::RenameInput { target, buffer }),
            },
            Prompt::CreateInput { mut buffer } => match code {
                KeyCode::Esc => self.status = Some("create cancelled".into()),
                KeyCode::Enter if buffer.is_empty() => {
                    self.status = Some("create cancelled: empty name".into());
                }
                KeyCode::Enter => self.finish_create(vfs, browser, buffer)?,
                KeyCode::Backspace => {
                    buffer.pop();
                    self.prompt = Some(Prompt::CreateInput { buffer });
                }
                KeyCode::Char(c) => {
                    buffer.push(c);
                    self.prompt = Some(Prompt::CreateInput { buffer });
                }
                _ => self.prompt = Some(Prompt::CreateInput { buffer }),
            },
            Prompt::ConfirmDelete { targets } => match code {
                KeyCode::Char('y') => self.spawn_delete(targets),
                _ => self.status = Some("delete cancelled".into()),
            },
            Prompt::ConfirmTrash { targets } => match code {
                KeyCode::Char('y') | KeyCode::Enter => self.spawn_trash(targets),
                _ => self.status = Some("trash cancelled".into()),
            },
            Prompt::Conflict(source) => match code {
                KeyCode::Char('o') => {
                    self.resolve_conflict(vfs, browser, source, ConflictPolicy::Overwrite)?
                }
                KeyCode::Char('s') => {
                    self.resolve_conflict(vfs, browser, source, ConflictPolicy::Skip)?
                }
                _ => self.status = Some("aborted".into()),
            },
            Prompt::SearchInput { mut buffer, origin } => match code {
                KeyCode::Esc => {
                    self.end_search();
                    self.status = Some(match Self::restore_origin(vfs, browser, &origin) {
                        Ok(()) => "search cancelled".into(),
                        Err(e) => format!("search cancelled; could not go back: {e}"),
                    });
                }
                KeyCode::Enter => {
                    // Still walking: stop where the cursor is rather than jump later, unasked.
                    if self.search_job.is_some() {
                        self.status = Some("search stopped".into());
                    }
                    self.end_search();
                }
                KeyCode::Backspace => {
                    buffer.pop();
                    self.update_search(vfs, browser, &buffer, &origin);
                    self.prompt = Some(Prompt::SearchInput { buffer, origin });
                }
                KeyCode::Char(c) => {
                    buffer.push(c);
                    self.update_search(vfs, browser, &buffer, &origin);
                    self.prompt = Some(Prompt::SearchInput { buffer, origin });
                }
                _ => self.prompt = Some(Prompt::SearchInput { buffer, origin }),
            },
            Prompt::OpenOther { path, mut buffer } => match code {
                KeyCode::Esc => self.status = Some("open with cancelled".into()),
                KeyCode::Enter => {
                    let command = buffer.trim().to_string();
                    if command.is_empty() {
                        self.status = Some("open with cancelled".into());
                    } else {
                        self.open_with(browser, &command, &path);
                    }
                }
                KeyCode::Backspace => {
                    buffer.pop();
                    self.prompt = Some(Prompt::OpenOther { path, buffer });
                }
                KeyCode::Char(c) => {
                    buffer.push(c);
                    self.prompt = Some(Prompt::OpenOther { path, buffer });
                }
                _ => self.prompt = Some(Prompt::OpenOther { path, buffer }),
            },
            Prompt::CommandInput { mut buffer } => match code {
                KeyCode::Esc => self.status = Some("command cancelled".into()),
                KeyCode::Enter => flow = self.run_command(vfs, browser, &buffer)?,
                KeyCode::Backspace => {
                    buffer.pop();
                    self.prompt = Some(Prompt::CommandInput { buffer });
                }
                KeyCode::Char(c) => {
                    buffer.push(c);
                    self.prompt = Some(Prompt::CommandInput { buffer });
                }
                _ => self.prompt = Some(Prompt::CommandInput { buffer }),
            },
        }
        Ok(flow)
    }

    /// Reacts to the search text changing: looks in the directory the search started in first —
    /// instantly, from the list already on screen — and otherwise walks everything below it on
    /// the blocking pool (see `search_job`), the hit arriving later through `poll_search`. A
    /// stale walk is dropped (and so cancelled) first. An empty query puts the browser back.
    fn update_search(
        &mut self,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        text: &str,
        origin: &SearchOrigin,
    ) {
        self.search_job = None;
        let Some(query) = Query::new(text) else {
            self.search_state = SearchState::Idle;
            if let Err(e) = Self::restore_origin(vfs, browser, origin) {
                self.status = Some(format!("could not go back: {e}"));
            }
            return;
        };

        if browser.current_dir() == origin.dir
            && let Some(index) = browser.find_match(text)
        {
            browser.select_index(index);
            self.search_state = SearchState::Found;
            return;
        }
        self.search_job = Some(SearchJob::start(
            &self.handle,
            origin.dir.clone(),
            query,
            browser.show_hidden(),
        ));
        self.search_state = SearchState::Searching;
    }

    /// Ends the `/` prompt's search, cancelling a walk still in flight.
    fn end_search(&mut self) {
        self.search_job = None;
        self.search_state = SearchState::Idle;
    }

    /// Returns the browser to where a search began: its directory, then its cursor.
    fn restore_origin(
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        origin: &SearchOrigin,
    ) -> Result<(), VfsError> {
        if browser.current_dir() != origin.dir {
            browser.goto(vfs, &origin.dir)?;
        }
        browser.select_index(origin.index);
        Ok(())
    }

    /// Applies a finished search to the browser: opens the hit's directory with the cursor on it.
    /// Call once per render tick, like `poll_bulk`.
    pub fn poll_search(&mut self, browser: &mut BrowserState, vfs: &dyn Vfs) {
        let Some(outcome) = self.search_job.as_mut().and_then(SearchJob::poll) else {
            return;
        };
        self.search_job = None;
        self.search_state = match outcome {
            SearchOutcome::Found(path) => match browser.reveal(vfs, &path) {
                Ok(()) => SearchState::Found,
                Err(e) => {
                    self.status = Some(format!("search: could not open {}: {e}", path.display()));
                    SearchState::Idle
                }
            },
            SearchOutcome::NotFound { truncated } => SearchState::NotFound { truncated },
            SearchOutcome::Cancelled => SearchState::Idle,
        };
    }

    /// The command waiting for the terminal, if a `:` command asked for one.
    pub fn take_handover(&mut self) -> Option<Handover> {
        self.handover.take()
    }

    /// Reports how a handed-over command ended and re-reads the listing, which it may have
    /// changed (an editor saving, a pager doing nothing).
    pub fn finish_handover(
        &mut self,
        browser: &mut BrowserState,
        vfs: &dyn Vfs,
        handover: &Handover,
        result: &std::io::Result<ExitStatus>,
    ) -> Result<()> {
        browser.reload(vfs)?;
        browser.prune_marks(vfs);
        self.status = Some(handover_status(&handover.line, result));
        Ok(())
    }

    /// Parses and runs a `:`-command buffer (without the leading `:`). Built-ins run right here;
    /// anything else runs under `sh -c` on the blocking pool (see `command`). Problems surface
    /// as a status message rather than an error — a typo shouldn't need a `Result` unwind.
    fn run_command(
        &mut self,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        buffer: &str,
    ) -> Result<ControlFlow<()>> {
        match command::parse(buffer, &self.interactive) {
            Ok(None) => {}
            Ok(Some(Command::Quit)) => return Ok(ControlFlow::Break(())),
            Ok(Some(Command::Cd(path))) => match browser.goto(vfs, Path::new(&path)) {
                Ok(()) => self.status = Some(format!("cd {path}")),
                Err(e) => self.status = Some(format!("cd failed: {e}")),
            },
            Ok(Some(Command::Mkdir {
                parents: true,
                names,
            })) => {
                self.run_builtin(
                    vfs,
                    browser,
                    "mkdir",
                    &names,
                    file_ops::create_directory_all,
                )?;
            }
            Ok(Some(Command::Mkdir {
                parents: false,
                names,
            })) => {
                self.run_builtin(vfs, browser, "mkdir", &names, file_ops::create_directory)?;
            }
            Ok(Some(Command::Touch { names })) => {
                self.run_builtin(vfs, browser, "touch", &names, file_ops::touch)?;
            }
            Ok(Some(Command::Trash)) => self.begin_trash(browser),
            Ok(Some(Command::Extract)) => self.begin_extract(browser),
            Ok(Some(Command::Compress { name })) => self.begin_compress(browser, &name),
            Ok(Some(Command::Shell(line))) => self.spawn_shell_command(browser, line),
            Ok(Some(Command::Interactive(line))) if self.is_busy() => {
                self.status = Some(format!("an operation is already in progress: {line}"));
            }
            Ok(Some(Command::Interactive(line))) => {
                self.handover = Some(Handover {
                    line,
                    cwd: browser.current_dir().to_path_buf(),
                });
            }
            Err(message) => self.status = Some(message),
        }
        Ok(ControlFlow::Continue(()))
    }

    /// Applies `op` to each of `names` (relative to the browsed directory) and reports the
    /// outcome, reloading the listing so the result is visible at once. Keeps going past a
    /// failure, like coreutils, so one bad name doesn't stop the rest.
    fn run_builtin(
        &mut self,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        verb: &str,
        names: &[String],
        op: fn(&dyn Vfs, &Path) -> Result<(), FileOpsError>,
    ) -> Result<()> {
        let mut done = 0;
        let mut first_error = None;
        for name in names {
            match op(vfs, &browser.current_dir().join(name)) {
                Ok(()) => done += 1,
                Err(e) => {
                    first_error.get_or_insert_with(|| format!("{verb} {name}: {e}"));
                }
            }
        }
        browser.reload(vfs)?;

        self.status = Some(match (first_error, names) {
            (Some(error), _) if done > 0 => format!("{error} ({done} of {} done)", names.len()),
            (Some(error), _) => error,
            (None, [single]) => format!("{verb} {single}"),
            (None, _) => format!("{verb}: {done} done"),
        });
        Ok(())
    }

    /// Runs `line` under `sh -c` in the browsed directory on the blocking pool, so a slow
    /// command never freezes the render loop. `Esc` kills it (see `cancel_bulk`).
    fn spawn_shell_command(&mut self, browser: &BrowserState, line: String) {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        let (tx, rx) = unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_bg = Arc::clone(&cancel);
        let cwd = browser.current_dir().to_path_buf();
        let line_bg = line.clone();

        self.handle.spawn_blocking(move || {
            let _ = tx.send(BulkMsg::ShellDone(shell_overlay::run_command(
                &cwd, &line_bg, &cancel_bg,
            )));
        });

        self.bulk = Some(BulkOp {
            kind: BulkKind::Shell {
                command: line,
                started: Instant::now(),
            },
            items_done: 0,
            current: String::new(),
            cancel: Some(cancel),
            rx,
        });
    }

    fn resolve_conflict(
        &mut self,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        source: ConflictSource,
        policy: ConflictPolicy,
    ) -> Result<()> {
        match source {
            ConflictSource::Paste {
                clip,
                dst_dir,
                index,
                ..
            } => {
                self.spawn_paste_item(clip, dst_dir, index, policy);
                Ok(())
            }
            ConflictSource::Rename { target, new_name } => {
                self.finish_rename(vfs, browser, target, new_name, policy)
            }
        }
    }

    /// If `index` is not the last item in `clip.paths`, spawns the next item (via
    /// `spawn_paste_item`, with a fresh `ConflictPolicy::Abort`) and returns `None` — the caller
    /// should stop, since the batch is still in flight. Otherwise returns `clip` back so the
    /// caller can finish up (e.g. clearing the clipboard on a completed move).
    fn try_continue_paste(
        &mut self,
        clip: Clipboard,
        dst_dir: PathBuf,
        index: usize,
    ) -> Option<Clipboard> {
        let next_index = index + 1;
        if next_index < clip.paths.len() {
            self.spawn_paste_item(clip, dst_dir, next_index, ConflictPolicy::Abort);
            None
        } else {
            Some(clip)
        }
    }

    /// Spawns a copy or move of `clip.paths[index]` into `dst_dir` on tokio's blocking thread
    /// pool. Progress and the final result arrive later via `poll_bulk` — this call itself never
    /// blocks. `poll_bulk` re-invokes this for the next item once one finishes, so a multi-item
    /// batch (from marks) stays one continuous background operation from the UI's perspective.
    fn spawn_paste_item(
        &mut self,
        clip: Clipboard,
        dst_dir: PathBuf,
        index: usize,
        policy: ConflictPolicy,
    ) {
        let Some(src) = clip.paths.get(index).cloned() else {
            self.status = Some("paste failed: batch index out of range".into());
            return;
        };
        let Some(name) = src.file_name() else {
            self.status = Some("clipboard entry has no file name".into());
            return;
        };
        let dst = dst_dir.join(name);

        let (tx, rx): (UnboundedSender<BulkMsg>, _) = unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_bg = Arc::clone(&cancel);
        let mode = clip.mode;
        let dst_bg = dst.clone();

        self.handle.spawn_blocking(move || {
            let vfs = LocalVfs;
            let progress_tx = tx.clone();
            let mut on_progress = move |path: &Path| -> ControlFlow<()> {
                let _ = progress_tx.send(BulkMsg::Progress {
                    path: display_name(path),
                });
                if cancel_bg.load(Ordering::Relaxed) {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            };
            let result = match mode {
                ClipboardMode::Copy => {
                    file_ops::copy_with_progress(&vfs, &src, &dst_bg, policy, &mut on_progress)
                }
                ClipboardMode::Move => {
                    file_ops::mv_with_progress(&vfs, &src, &dst_bg, policy, &mut on_progress)
                }
            };
            let _ = tx.send(BulkMsg::Done(result));
        });

        self.bulk = Some(BulkOp {
            kind: BulkKind::Paste {
                clip,
                dst_dir,
                index,
                dst,
            },
            items_done: 0,
            current: String::new(),
            cancel: Some(cancel),
            rx,
        });
    }

    /// Spawns a delete of one or more targets on tokio's blocking thread pool, one
    /// `file_ops::delete` call per target in order. Stops at the first failure — the same
    /// abort-not-partial semantics `ConflictPolicy::Abort` gives paste — rather than trying to
    /// press on through the rest of the batch. No per-item cancellation (see `BulkOp` doc), but
    /// it still keeps a large recursive delete from freezing the render loop.
    fn spawn_delete(&mut self, targets: Vec<PathBuf>) {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        let (tx, rx) = unbounded_channel();
        let targets_bg = targets.clone();

        self.handle.spawn_blocking(move || {
            let vfs = LocalVfs;
            let mut result = Ok(Outcome::Completed);
            for target in &targets_bg {
                match file_ops::delete(&vfs, target) {
                    Ok(()) => {
                        let _ = tx.send(BulkMsg::Progress {
                            path: display_name(target),
                        });
                    }
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                }
            }
            let _ = tx.send(BulkMsg::Done(result));
        });

        self.bulk = Some(BulkOp {
            kind: BulkKind::Delete { targets },
            items_done: 0,
            current: String::new(),
            cancel: None,
            rx,
        });
    }

    /// Starts extracting the marked (or selected) archives, each into a new directory named after
    /// it beside the archive's own listing, so a tarball full of loose files cannot litter the
    /// browsed directory.
    fn begin_extract(&mut self, browser: &BrowserState) {
        let archives: Vec<PathBuf> = Self::marked_or_selected(browser)
            .into_iter()
            .filter(|path| Format::of(path).is_some())
            .collect();
        match archives.is_empty() {
            true => self.status = Some("extract: no zip, tar or tar.gz selected".into()),
            false => {
                let jobs = Self::extract_jobs_per_archive(browser.current_dir(), archives);
                self.spawn_extract(jobs, ConflictPolicy::Abort, false);
            }
        }
    }

    /// One job per archive, each into a folder of `dir` named after it.
    fn extract_jobs_per_archive(dir: &Path, archives: Vec<PathBuf>) -> Vec<ExtractJob> {
        archives
            .into_iter()
            .map(|archive| {
                let stem = Format::stem_of(&archive).unwrap_or_else(|| "extracted".into());
                ExtractJob {
                    dest: dir.join(stem),
                    archive,
                }
            })
            .collect()
    }

    /// The extract form for the marked (or selected) archives, or `None` if there are none.
    pub fn begin_extract_form(&mut self, browser: &BrowserState) -> Option<ExtractPopup> {
        let archives: Vec<PathBuf> = Self::marked_or_selected(browser)
            .into_iter()
            .filter(|path| Format::of(path).is_some())
            .collect();
        let form = ExtractPopup::new(archives, browser.current_dir().to_path_buf());
        if form.is_none() {
            self.status = Some("extract: no zip, tar or tar.gz selected".into());
        }
        form
    }

    /// Starts the job a submitted extract form describes.
    ///
    /// # Errors
    ///
    /// A message for the form to show when the job cannot start: another operation is running, or
    /// (with existing files set to stop) a folder the job would create already exists.
    pub fn start_extract(&mut self, request: extract_popup::Request) -> Result<(), String> {
        if self.is_busy() {
            return Err("an operation is already in progress".into());
        }
        let jobs = match request.destination {
            extract_popup::Destination::Here => request
                .archives
                .into_iter()
                .map(|archive| ExtractJob {
                    archive,
                    dest: request.dir.clone(),
                })
                .collect(),
            extract_popup::Destination::Folder(name) => request
                .archives
                .into_iter()
                .map(|archive| ExtractJob {
                    archive,
                    dest: request.dir.join(&name),
                })
                .collect(),
            extract_popup::Destination::PerArchive => {
                Self::extract_jobs_per_archive(&request.dir, request.archives)
            }
        };
        // Extracting into a folder that is already there is what "skip" and "replace" are for;
        // with "stop" it would only fail on the first clashing file, after writing the others.
        if request.policy == ConflictPolicy::Abort
            && let Some(taken) = jobs
                .iter()
                .find(|job| job.dest != request.dir && std::fs::symlink_metadata(&job.dest).is_ok())
        {
            return Err(format!("{} already exists", display_name(&taken.dest)));
        }
        self.spawn_extract(jobs, request.policy, request.delete_archives);
        Ok(())
    }

    /// Starts packing the marked (or selected) entries into `name` in the browsed directory.
    fn begin_compress(&mut self, browser: &BrowserState, name: &str) {
        let Some(format) = Format::of(Path::new(name)) else {
            self.status = Some("compress: name must end in .zip, .tar or .tar.gz".into());
            return;
        };
        let sources = Self::marked_or_selected(browser);
        if sources.is_empty() {
            self.status = Some("compress: nothing selected".into());
            return;
        }
        let out = browser.current_dir().join(name);
        self.spawn_compress(
            vec![CompressJob { sources, out }],
            format,
            Level::default(),
            false,
        );
    }

    /// The channel and cancel flag every archive job reports through. The callback is what
    /// `file_ops::archive` calls after each entry: it forwards the entry's name and asks the job
    /// to stop once `cancel` is set.
    fn archive_channel() -> (
        UnboundedSender<BulkMsg>,
        UnboundedReceiver<BulkMsg>,
        Arc<AtomicBool>,
    ) {
        let (tx, rx) = unbounded_channel();
        (tx, rx, Arc::new(AtomicBool::new(false)))
    }

    /// Starts one extraction per entry of `jobs`, in order, stopping at the first failure. With
    /// `delete_archives`, each archive goes to the trash once it has been extracted, so a failure
    /// never costs an archive its contents.
    fn spawn_extract(
        &mut self,
        jobs: Vec<ExtractJob>,
        policy: ConflictPolicy,
        delete_archives: bool,
    ) {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        let (tx, rx, cancel) = Self::archive_channel();
        let cancel_bg = Arc::clone(&cancel);

        self.handle.spawn_blocking(move || {
            let progress_tx = tx.clone();
            let mut on_progress = move |path: &Path| -> ControlFlow<()> {
                let _ = progress_tx.send(BulkMsg::Progress {
                    path: display_name(path),
                });
                match cancel_bg.load(Ordering::Relaxed) {
                    true => ControlFlow::Break(()),
                    false => ControlFlow::Continue(()),
                }
            };
            let mut result = Ok(Outcome::Completed);
            for job in &jobs {
                if let Err(e) = archive::extract(
                    &job.archive,
                    &job.dest,
                    policy,
                    ExtractLimits::default(),
                    &mut on_progress,
                ) {
                    result = Err(e);
                    break;
                }
                if delete_archives && let Err(e) = file_ops::trash(&job.archive) {
                    result = Err(e);
                    break;
                }
            }
            let _ = tx.send(BulkMsg::Done(result));
        });

        self.bulk = Some(BulkOp {
            kind: BulkKind::Extract,
            items_done: 0,
            current: String::new(),
            cancel: Some(cancel),
            rx,
        });
    }

    /// Starts one archive job per entry of `jobs`, in order, stopping at the first failure. With
    /// `delete_originals`, each job's sources go to the trash once its archive is complete, so a
    /// failure never costs a file its only copy.
    fn spawn_compress(
        &mut self,
        jobs: Vec<CompressJob>,
        format: Format,
        level: Level,
        delete_originals: bool,
    ) {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        let (tx, rx, cancel) = Self::archive_channel();
        let cancel_bg = Arc::clone(&cancel);

        self.handle.spawn_blocking(move || {
            let progress_tx = tx.clone();
            let mut on_progress = move |path: &Path| -> ControlFlow<()> {
                let _ = progress_tx.send(BulkMsg::Progress {
                    path: display_name(path),
                });
                match cancel_bg.load(Ordering::Relaxed) {
                    true => ControlFlow::Break(()),
                    false => ControlFlow::Continue(()),
                }
            };
            let mut result = Ok(Outcome::Completed);
            'jobs: for job in &jobs {
                if let Err(e) = archive::compress_with_level(
                    &job.sources,
                    &job.out,
                    format,
                    level,
                    ConflictPolicy::Abort,
                    &mut on_progress,
                ) {
                    result = Err(e);
                    break;
                }
                if delete_originals {
                    for source in &job.sources {
                        if let Err(e) = file_ops::trash(source) {
                            result = Err(e);
                            break 'jobs;
                        }
                    }
                }
            }
            let _ = tx.send(BulkMsg::Done(result));
        });

        self.bulk = Some(BulkOp {
            kind: BulkKind::Compress,
            items_done: 0,
            current: String::new(),
            cancel: Some(cancel),
            rx,
        });
    }

    /// The compress form for the marked (or selected) entries, or `None` if there are none.
    pub fn begin_compress_form(&mut self, browser: &BrowserState) -> Option<CompressPopup> {
        let form = CompressPopup::new(
            Self::marked_or_selected(browser),
            browser.current_dir().to_path_buf(),
        );
        if form.is_none() {
            self.status = Some("compress: nothing selected".into());
        }
        form
    }

    /// Starts the job a submitted compress form describes.
    ///
    /// # Errors
    ///
    /// A message for the form to show when the job cannot start: another operation is running, or
    /// a file the job would create already exists (nothing is ever overwritten).
    pub fn start_compress(&mut self, request: compress_popup::Request) -> Result<(), String> {
        if self.is_busy() {
            return Err("an operation is already in progress".into());
        }
        let extension = request.format.extension();
        let jobs: Vec<CompressJob> = match request.layout {
            compress_popup::Layout::One { file_name } => vec![CompressJob {
                sources: request.sources,
                out: request.dir.join(file_name),
            }],
            compress_popup::Layout::PerItem => request
                .sources
                .into_iter()
                .filter_map(|source| {
                    // `a.txt` becomes `a.txt.zip`, so `a.txt` and `a.pdf` cannot collide.
                    let mut name = source.file_name()?.to_os_string();
                    name.push(format!(".{extension}"));
                    Some(CompressJob {
                        out: request.dir.join(name),
                        sources: vec![source],
                    })
                })
                .collect(),
        };
        if let Some(taken) = jobs
            .iter()
            .find(|job| std::fs::symlink_metadata(&job.out).is_ok())
        {
            return Err(format!("{} already exists", display_name(&taken.out)));
        }
        self.spawn_compress(
            jobs,
            request.format,
            request.level,
            request.delete_originals,
        );
        Ok(())
    }

    /// Same shape as `spawn_delete`, but sends each target to the desktop trash
    /// (`file_ops::trash`) instead of deleting it — no `Vfs` involved (see that function's docs).
    fn spawn_trash(&mut self, targets: Vec<PathBuf>) {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        let (tx, rx) = unbounded_channel();
        let targets_bg = targets.clone();

        self.handle.spawn_blocking(move || {
            let mut result = Ok(Outcome::Completed);
            for target in &targets_bg {
                match file_ops::trash(target) {
                    Ok(()) => {
                        let _ = tx.send(BulkMsg::Progress {
                            path: display_name(target),
                        });
                    }
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                }
            }
            let _ = tx.send(BulkMsg::Done(result));
        });

        self.bulk = Some(BulkOp {
            kind: BulkKind::Trash { targets },
            items_done: 0,
            current: String::new(),
            cancel: None,
            rx,
        });
    }

    fn finish_rename(
        &mut self,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        target: PathBuf,
        new_name: String,
        policy: ConflictPolicy,
    ) -> Result<()> {
        match file_ops::rename(vfs, &target, &new_name, policy) {
            Ok(Outcome::Completed) => {
                browser.reload(vfs)?;
                self.status = Some(format!("renamed to {new_name}"));
            }
            Ok(Outcome::Skipped) => self.status = Some("rename skipped".into()),
            Err(FileOpsError::Vfs(VfsError::AlreadyExists(_))) => {
                self.prompt = Some(Prompt::Conflict(ConflictSource::Rename {
                    target,
                    new_name,
                }));
            }
            Err(e) => self.status = Some(format!("rename failed: {e}")),
        }
        Ok(())
    }

    fn finish_create(
        &mut self,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        name: String,
    ) -> Result<()> {
        let is_dir = name.ends_with('/');
        let trimmed = name.trim_end_matches('/');
        if trimmed.is_empty() {
            self.status = Some("create cancelled: empty name".into());
            return Ok(());
        }

        let path = browser.current_dir().join(trimmed);
        let result = if is_dir {
            file_ops::create_directory(vfs, &path)
        } else {
            file_ops::create_new_file(vfs, &path)
        };

        match result {
            Ok(()) => {
                browser.reload(vfs)?;
                self.status = Some(format!("created {trimmed}"));
            }
            Err(e) => self.status = Some(format!("create failed: {e}")),
        }
        Ok(())
    }
}

/// One status-line summary of a finished command: its output (lines joined, so `ls` fits on the
/// bar) when it printed any, else just that it ran, and the exit code when it failed.
fn shell_status(command: &str, outcome: &CommandOutcome) -> String {
    let CommandOutcome::Finished(output) = outcome else {
        return format!("cancelled: {command}");
    };
    let lines: Vec<&str> = output
        .text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let mut shown = lines
        .iter()
        .take(STATUS_OUTPUT_LINES)
        .copied()
        .collect::<Vec<_>>()
        .join(" | ");
    if lines.len() > STATUS_OUTPUT_LINES {
        shown.push_str(&format!(
            " (+{} more lines)",
            lines.len() - STATUS_OUTPUT_LINES
        ));
    } else if output.truncated {
        shown.push_str(" (output truncated)");
    }

    match (output.code, shown.is_empty()) {
        (Some(0), true) => format!("ran: {command}"),
        (Some(0), false) => shown,
        (Some(code), true) => format!("exit {code}: {command}"),
        (Some(code), false) => format!("exit {code}: {shown}"),
        (None, _) => format!("killed by a signal: {command}"),
    }
}

/// The status line for a command that had the terminal to itself.
fn handover_status(line: &str, result: &std::io::Result<ExitStatus>) -> String {
    match result {
        Ok(status) if status.success() => format!("{line}: done"),
        Ok(status) => match status.code() {
            Some(code) => format!("{line}: exited with code {code}"),
            None => format!("{line}: ended by a signal"),
        },
        Err(e) => format!("could not run '{line}': {e}"),
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Renders a batch of paths for a status message: the single name, or an item count.
fn batch_label(paths: &[PathBuf]) -> String {
    match paths {
        [single] => display_name(single),
        many => format!("{} items", many.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shell_overlay::CommandOutput;

    fn finished(code: i32, text: &str) -> CommandOutcome {
        CommandOutcome::Finished(CommandOutput {
            code: Some(code),
            text: text.into(),
            truncated: false,
        })
    }

    #[test]
    fn a_quiet_success_says_it_ran_and_a_chatty_one_shows_its_output() {
        assert_eq!(shell_status("true", &finished(0, "")), "ran: true");
        assert_eq!(shell_status("ls", &finished(0, "a\n\nb\n")), "a | b");
    }

    #[test]
    fn a_failure_carries_its_exit_code_and_any_message() {
        assert_eq!(shell_status("false", &finished(1, "")), "exit 1: false");
        assert_eq!(
            shell_status("ls x", &finished(2, "ls: cannot access 'x'\n")),
            "exit 2: ls: cannot access 'x'"
        );
    }

    #[test]
    fn long_output_is_summarized_and_cancellation_and_signals_are_named() {
        let many = (1..=12).map(|n| format!("{n}\n")).collect::<String>();
        assert_eq!(
            shell_status("seq 12", &finished(0, &many)),
            "1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 (+4 more lines)"
        );
        assert_eq!(
            shell_status("sleep 9", &CommandOutcome::Cancelled),
            "cancelled: sleep 9"
        );
        let killed = CommandOutcome::Finished(CommandOutput {
            code: None,
            text: String::new(),
            truncated: false,
        });
        assert_eq!(shell_status("x", &killed), "killed by a signal: x");
    }

    struct Fixture {
        _runtime: tokio::runtime::Runtime,
        root: PathBuf,
        app: App,
        browser: BrowserState,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "minuteman-app-test-{label}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let app =
                App::new(runtime.handle().clone()).with_interactive_commands(vec!["nvim".into()]);
            let browser = BrowserState::new(&LocalVfs, root.clone()).unwrap();
            Self {
                _runtime: runtime,
                root,
                app,
                browser,
            }
        }

        fn flow(&mut self, line: &str) -> ControlFlow<()> {
            self.app
                .run_command(&LocalVfs, &mut self.browser, line)
                .unwrap()
        }

        /// Runs a command that must not ask the app to quit.
        fn run(&mut self, line: &str) {
            assert_eq!(self.flow(line), ControlFlow::Continue(()), "{line}");
        }

        /// Waits for a background `:` command to finish, applying its result the way the render
        /// loop does.
        fn wait_for_idle(&mut self) {
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            while self.app.is_busy() && Instant::now() < deadline {
                self.app.poll_bulk(&mut self.browser, &LocalVfs).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(!self.app.is_busy(), "command did not finish in time");
        }

        /// Presses one key at the open prompt, as the event loop would.
        fn press(&mut self, code: KeyCode) {
            let flow = self
                .app
                .handle_prompt_key(code, &LocalVfs, &mut self.browser)
                .unwrap();
            assert_eq!(flow, ControlFlow::Continue(()));
        }

        fn type_text(&mut self, text: &str) {
            for c in text.chars() {
                self.press(KeyCode::Char(c));
            }
        }

        /// Waits for the `/` prompt's background walk to finish, applying its result the way the
        /// render loop does.
        fn wait_for_search(&mut self) {
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            while self.app.search_job.is_some() && Instant::now() < deadline {
                self.app.poll_search(&mut self.browser, &LocalVfs);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(
                self.app.search_job.is_none(),
                "search did not finish in time"
            );
        }

        fn selected_name(&self) -> String {
            self.browser.selected_entry().unwrap().name.clone()
        }

        fn listed(&self) -> Vec<String> {
            self.browser
                .current_entries()
                .iter()
                .map(|e| e.name.clone())
                .collect()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn select_named(f: &mut Fixture, name: &str) {
        let index = f.listed().iter().position(|n| n == name).unwrap();
        f.browser.select_index(index);
    }

    #[test]
    fn compress_then_extract_round_trips_a_selected_directory() {
        let mut f = Fixture::new("archive-round-trip");
        std::fs::create_dir_all(f.root.join("proj/inner")).unwrap();
        std::fs::write(f.root.join("proj/inner/a.txt"), b"alpha").unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        select_named(&mut f, "proj");

        f.run("compress proj.tar.gz");
        f.wait_for_idle();
        assert_eq!(f.app.status.as_deref(), Some("compress complete"));
        assert!(f.listed().contains(&"proj.tar.gz".to_string()));

        select_named(&mut f, "proj.tar.gz");
        f.run("extract");
        f.wait_for_idle();
        assert_eq!(f.app.status.as_deref(), Some("extract complete"));
        assert_eq!(
            std::fs::read(f.root.join("proj/proj/inner/a.txt")).unwrap(),
            b"alpha"
        );
    }

    #[test]
    fn extract_and_compress_report_what_is_wrong_instead_of_running() {
        let mut f = Fixture::new("archive-errors");
        f.run("touch plain.txt");
        select_named(&mut f, "plain.txt");

        f.run("extract");
        assert_eq!(
            f.app.status.as_deref(),
            Some("extract: no zip, tar or tar.gz selected")
        );
        f.run("compress plain.rar");
        assert_eq!(
            f.app.status.as_deref(),
            Some("compress: name must end in .zip, .tar or .tar.gz")
        );
        assert!(!f.app.is_busy());
    }

    #[test]
    fn compress_will_not_replace_an_existing_archive() {
        let mut f = Fixture::new("archive-exists");
        f.run("touch a.txt out.zip");
        select_named(&mut f, "a.txt");

        f.run("compress out.zip");
        f.wait_for_idle();
        assert!(
            f.app
                .status
                .as_deref()
                .is_some_and(|s| s.starts_with("compress failed:") && s.ends_with("already exists")),
            "{:?}",
            f.app.status
        );
        assert_eq!(std::fs::read(f.root.join("out.zip")).unwrap(), b"");
    }

    fn request(
        f: &Fixture,
        names: &[&str],
        layout: compress_popup::Layout,
    ) -> compress_popup::Request {
        compress_popup::Request {
            sources: names.iter().map(|n| f.root.join(n)).collect(),
            dir: f.root.clone(),
            format: Format::TarGz,
            level: Level::Best,
            layout,
            delete_originals: false,
        }
    }

    /// A fixture holding `proj.tar.gz`, made from a folder that held `inner/a.txt`.
    fn fixture_with_archive(name: &str) -> Fixture {
        let mut f = Fixture::new(name);
        std::fs::create_dir_all(f.root.join("proj/inner")).unwrap();
        std::fs::write(f.root.join("proj/inner/a.txt"), b"alpha").unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        select_named(&mut f, "proj");
        f.run("compress proj.tar.gz");
        f.wait_for_idle();
        std::fs::remove_dir_all(f.root.join("proj")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        select_named(&mut f, "proj.tar.gz");
        f
    }

    fn extract_request(
        f: &Fixture,
        destination: extract_popup::Destination,
        policy: ConflictPolicy,
        delete_archives: bool,
    ) -> extract_popup::Request {
        extract_popup::Request {
            archives: vec![f.root.join("proj.tar.gz")],
            dir: f.root.clone(),
            destination,
            policy,
            delete_archives,
        }
    }

    #[test]
    fn the_extract_form_opens_only_for_archives() {
        let mut f = Fixture::new("extract-form-none");
        f.run("touch plain.txt");
        select_named(&mut f, "plain.txt");
        assert!(f.app.begin_extract_form(&f.browser).is_none());
        assert_eq!(
            f.app.status.as_deref(),
            Some("extract: no zip, tar or tar.gz selected")
        );

        let mut f = fixture_with_archive("extract-form-some");
        assert!(f.app.begin_extract_form(&f.browser).is_some());
    }

    #[test]
    fn the_extract_form_can_unpack_into_a_named_folder() {
        let mut f = fixture_with_archive("extract-named");
        let request = extract_request(
            &f,
            extract_popup::Destination::Folder("out".into()),
            ConflictPolicy::Abort,
            false,
        );
        f.app.start_extract(request).unwrap();
        f.wait_for_idle();
        assert_eq!(f.app.status.as_deref(), Some("extract complete"));
        assert_eq!(
            std::fs::read(f.root.join("out/proj/inner/a.txt")).unwrap(),
            b"alpha"
        );
        // The archive stays unless asked otherwise.
        assert!(f.root.join("proj.tar.gz").exists());
    }

    #[test]
    fn the_extract_form_can_unpack_here_and_trash_the_archive() {
        let mut f = fixture_with_archive("extract-here");
        let request = extract_request(
            &f,
            extract_popup::Destination::Here,
            ConflictPolicy::Abort,
            true,
        );
        f.app.start_extract(request).unwrap();
        f.wait_for_idle();
        assert_eq!(f.app.status.as_deref(), Some("extract complete"));
        assert!(f.root.join("proj/inner/a.txt").exists());
        assert!(!f.root.join("proj.tar.gz").exists());
    }

    #[test]
    fn stopping_on_existing_files_will_not_enter_a_folder_that_is_there() {
        let mut f = fixture_with_archive("extract-taken");
        std::fs::create_dir(f.root.join("out")).unwrap();
        let request = extract_request(
            &f,
            extract_popup::Destination::Folder("out".into()),
            ConflictPolicy::Abort,
            false,
        );
        assert_eq!(
            f.app.start_extract(request),
            Err("out already exists".into())
        );
        assert!(!f.app.is_busy());

        // "Replace" is how a folder that is there gets filled.
        let request = extract_request(
            &f,
            extract_popup::Destination::Folder("out".into()),
            ConflictPolicy::Overwrite,
            false,
        );
        f.app.start_extract(request).unwrap();
        f.wait_for_idle();
        assert!(f.root.join("out/proj/inner/a.txt").exists());
    }

    #[test]
    fn the_compress_form_makes_one_archive_of_the_marked_items() {
        let mut f = Fixture::new("form-one");
        std::fs::write(f.root.join("a.txt"), b"alpha").unwrap();
        std::fs::write(f.root.join("b.txt"), b"beta").unwrap();
        f.browser.reload(&LocalVfs).unwrap();

        let layout = compress_popup::Layout::One {
            file_name: "both.tar.gz".into(),
        };
        f.app
            .start_compress(request(&f, &["a.txt", "b.txt"], layout))
            .unwrap();
        assert!(f.app.is_busy());
        f.wait_for_idle();

        assert_eq!(f.app.status.as_deref(), Some("compress complete"));
        let listing = preview::archive::list(&f.root.join("both.tar.gz")).unwrap();
        let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.txt", "b.txt"]);
        // Nothing was deleted: the originals stay unless asked otherwise.
        assert!(f.root.join("a.txt").exists() && f.root.join("b.txt").exists());
    }

    #[test]
    fn the_compress_form_can_make_one_archive_per_item() {
        let mut f = Fixture::new("form-per-item");
        std::fs::write(f.root.join("a.txt"), b"alpha").unwrap();
        std::fs::create_dir(f.root.join("dir")).unwrap();
        std::fs::write(f.root.join("dir/inner"), b"x").unwrap();
        f.browser.reload(&LocalVfs).unwrap();

        f.app
            .start_compress(request(
                &f,
                &["a.txt", "dir"],
                compress_popup::Layout::PerItem,
            ))
            .unwrap();
        f.wait_for_idle();

        assert_eq!(f.app.status.as_deref(), Some("compress complete"));
        assert!(f.root.join("a.txt.tar.gz").is_file());
        assert!(f.root.join("dir.tar.gz").is_file());
    }

    #[test]
    fn the_compress_form_refuses_a_name_that_is_taken_without_starting_anything() {
        let mut f = Fixture::new("form-taken");
        std::fs::write(f.root.join("a.txt"), b"alpha").unwrap();
        std::fs::write(f.root.join("a.txt.tar.gz"), b"precious").unwrap();

        let error = f
            .app
            .start_compress(request(&f, &["a.txt"], compress_popup::Layout::PerItem))
            .unwrap_err();
        assert_eq!(error, "a.txt.tar.gz already exists");
        assert!(!f.app.is_busy());
        assert_eq!(
            std::fs::read(f.root.join("a.txt.tar.gz")).unwrap(),
            b"precious"
        );
    }

    #[test]
    fn opening_the_compress_form_with_nothing_selected_says_so() {
        let mut f = Fixture::new("form-empty");
        assert!(f.app.begin_compress_form(&f.browser).is_none());
        assert_eq!(f.app.status.as_deref(), Some("compress: nothing selected"));
    }

    #[test]
    fn a_second_compress_cannot_start_while_one_runs() {
        let mut f = Fixture::new("form-busy");
        std::fs::write(f.root.join("a.txt"), b"alpha").unwrap();
        let first = compress_popup::Layout::One {
            file_name: "one.zip".into(),
        };
        let second = compress_popup::Layout::One {
            file_name: "two.zip".into(),
        };
        f.app
            .start_compress(request(&f, &["a.txt"], first))
            .unwrap();
        let refused = f.app.start_compress(request(&f, &["a.txt"], second));
        assert_eq!(refused, Err("an operation is already in progress".into()));
        f.wait_for_idle();
    }

    #[test]
    fn mkdir_and_touch_create_things_and_show_them_at_once() {
        let mut f = Fixture::new("builtins");

        f.run("mkdir -p deep/er");
        f.run("touch notes.txt 'two words.txt'");

        assert!(f.root.join("deep/er").is_dir());
        assert!(f.root.join("two words.txt").is_file());
        assert_eq!(f.listed(), vec!["deep", "notes.txt", "two words.txt"]);
        assert_eq!(f.app.status.as_deref(), Some("touch: 2 done"));
    }

    #[test]
    fn a_failing_name_is_reported_but_does_not_stop_the_rest() {
        let mut f = Fixture::new("partial");
        std::fs::write(f.root.join("taken"), b"x").unwrap();

        f.run("mkdir taken fresh");

        assert!(f.root.join("fresh").is_dir());
        let status = f.app.status.clone().unwrap();
        assert!(status.starts_with("mkdir taken: "), "{status}");
        assert!(status.ends_with("(1 of 2 done)"), "{status}");
    }

    #[test]
    fn cd_and_quit_still_work() {
        let mut f = Fixture::new("cd");
        std::fs::create_dir(f.root.join("sub")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();

        f.run("cd sub");
        assert_eq!(f.browser.current_dir(), f.root.join("sub"));
        assert_eq!(f.flow("quit"), ControlFlow::Break(()));
    }

    #[test]
    fn any_other_command_runs_in_the_shell_and_its_effects_appear() {
        let mut f = Fixture::new("shell");

        f.run("echo hi > made-by-sh.txt && echo done");
        assert!(f.app.is_busy());
        f.wait_for_idle();

        assert_eq!(f.app.status.as_deref(), Some("done"));
        assert_eq!(f.listed(), vec!["made-by-sh.txt"]);
    }

    #[test]
    fn a_failed_shell_command_reports_its_exit_code() {
        let mut f = Fixture::new("shell-fail");

        f.run("exit 4");
        f.wait_for_idle();

        assert_eq!(f.app.status.as_deref(), Some("exit 4: exit 4"));
    }

    #[test]
    fn escape_cancels_a_running_shell_command() {
        let mut f = Fixture::new("shell-cancel");

        f.run("sleep 30");
        assert!(f.app.is_busy());
        f.app.cancel_bulk();
        f.wait_for_idle();

        assert_eq!(f.app.status.as_deref(), Some("cancelled: sleep 30"));
    }

    #[test]
    fn a_listed_program_asks_for_the_terminal_instead_of_running_captured() {
        let mut f = Fixture::new("handover-listed");

        f.run("nvim ROADMAP.md");

        assert!(
            !f.app.is_busy(),
            "nothing runs until main hands the terminal over"
        );
        assert_eq!(
            f.app.take_handover(),
            Some(Handover {
                line: "nvim ROADMAP.md".into(),
                cwd: f.root.clone(),
            })
        );
        assert_eq!(f.app.take_handover(), None, "a handover is taken once");
    }

    #[test]
    fn a_bang_asks_for_the_terminal_for_any_program() {
        let mut f = Fixture::new("handover-bang");

        f.run("!python3 -i");

        let handover = f.app.take_handover().expect("a handover was requested");
        assert_eq!(handover.line, "python3 -i");
    }

    #[test]
    fn a_handover_is_refused_while_another_operation_runs() {
        let mut f = Fixture::new("handover-busy");

        f.run("sleep 30");
        f.run("nvim notes.md");

        assert_eq!(f.app.take_handover(), None);
        assert!(
            f.app
                .status
                .clone()
                .unwrap()
                .contains("already in progress")
        );
        f.app.cancel_bulk();
        f.wait_for_idle();
    }

    #[test]
    fn finishing_a_handover_reports_how_it_ended_and_shows_what_it_changed() {
        use std::os::unix::process::ExitStatusExt;
        let mut f = Fixture::new("handover-finish");
        f.run("nvim x.md");
        let handover = f.app.take_handover().unwrap();
        std::fs::write(f.root.join("x.md"), b"saved by the editor").unwrap();

        f.app
            .finish_handover(
                &mut f.browser,
                &LocalVfs,
                &handover,
                &Ok(ExitStatus::from_raw(0)),
            )
            .unwrap();
        assert_eq!(f.app.status.as_deref(), Some("nvim x.md: done"));
        assert_eq!(f.listed(), vec!["x.md"]);

        f.app
            .finish_handover(
                &mut f.browser,
                &LocalVfs,
                &handover,
                &Ok(ExitStatus::from_raw(3 << 8)),
            )
            .unwrap();
        assert_eq!(
            f.app.status.as_deref(),
            Some("nvim x.md: exited with code 3")
        );

        let missing = Err(std::io::Error::from(std::io::ErrorKind::NotFound));
        f.app
            .finish_handover(&mut f.browser, &LocalVfs, &handover, &missing)
            .unwrap();
        assert!(
            f.app
                .status
                .clone()
                .unwrap()
                .starts_with("could not run 'nvim x.md'")
        );
    }

    #[test]
    fn cancel_forgets_a_cut_and_the_marks_from_any_directory() {
        let mut f = Fixture::new("cancel-all");
        for name in ["a", "b", "c"] {
            std::fs::write(f.root.join(name), b"x").unwrap();
        }
        std::fs::create_dir(f.root.join("elsewhere")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        f.browser.select_index(1);
        f.browser.toggle_mark();
        f.browser.select_index(2);
        f.browser.toggle_mark();
        f.app.cut(&f.browser);
        assert!(f.app.clipboard.is_some());

        // Somewhere else entirely, as the request says: "no matter the directory I am".
        f.browser
            .goto(&LocalVfs, &f.root.join("elsewhere"))
            .unwrap();
        f.app.cancel_all(&mut f.browser);

        assert!(f.app.clipboard.is_none());
        assert!(f.browser.marked_paths().is_empty());
        let status = f.app.status.clone().unwrap();
        assert!(status.starts_with("cancelled: cut of "), "{status}");
        assert!(status.ends_with("2 marks"), "{status}");
        for name in ["a", "b", "c"] {
            assert!(
                f.root.join(name).exists(),
                "a cancelled cut must not touch {name}"
            );
        }
    }

    #[test]
    fn cancel_says_so_when_there_is_nothing_to_cancel() {
        let mut f = Fixture::new("cancel-nothing");
        f.app.cancel_all(&mut f.browser);
        assert_eq!(f.app.status.as_deref(), Some("nothing to cancel"));
    }

    #[test]
    fn cancel_also_stops_a_running_command() {
        let mut f = Fixture::new("cancel-running");
        f.run("sleep 30");
        assert!(f.app.is_busy());

        f.app.cancel_all(&mut f.browser);
        f.wait_for_idle();

        assert_eq!(f.app.status.as_deref(), Some("cancelled: sleep 30"));
    }

    fn deep_tree(f: &Fixture) {
        std::fs::create_dir_all(f.root.join("a").join("b")).unwrap();
        std::fs::write(f.root.join("a").join("b").join("aerend.md"), b"").unwrap();
        std::fs::write(f.root.join("notes.txt"), b"").unwrap();
        std::fs::write(f.root.join("zzz.txt"), b"").unwrap();
    }

    #[test]
    fn a_search_walks_into_subdirectories_and_opens_the_hit() {
        let mut f = Fixture::new("search-deep");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("aerend");
        f.wait_for_search();

        assert_eq!(f.browser.current_dir(), f.root.join("a").join("b"));
        assert_eq!(f.selected_name(), "aerend.md");
    }

    #[test]
    fn a_match_in_the_directory_already_open_is_selected_without_a_walk() {
        let mut f = Fixture::new("search-local");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("notes");

        assert!(f.app.search_job.is_none());
        assert_eq!(f.browser.current_dir(), f.root);
        assert_eq!(f.selected_name(), "notes.txt");
    }

    #[test]
    fn escape_returns_to_the_directory_and_row_the_search_started_from() {
        let mut f = Fixture::new("search-escape");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();
        f.browser.select_index(2);
        let started_on = f.selected_name();

        f.app.begin_search(&f.browser);
        f.type_text("aerend");
        f.wait_for_search();
        assert_ne!(f.browser.current_dir(), f.root);

        f.press(KeyCode::Esc);
        assert_eq!(f.browser.current_dir(), f.root);
        assert_eq!(f.selected_name(), started_on);
        assert_eq!(f.app.status.as_deref(), Some("search cancelled"));
        assert!(f.app.prompt.is_none());
    }

    #[test]
    fn enter_keeps_the_cursor_where_the_search_landed() {
        let mut f = Fixture::new("search-enter");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("aerend");
        f.wait_for_search();
        f.press(KeyCode::Enter);

        assert!(f.app.prompt.is_none());
        assert_eq!(f.browser.current_dir(), f.root.join("a").join("b"));
        assert_eq!(f.selected_name(), "aerend.md");
    }

    #[test]
    fn a_name_that_exists_nowhere_leaves_the_cursor_and_says_no_match() {
        let mut f = Fixture::new("search-miss");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("nothing-like-this");
        f.wait_for_search();

        assert_eq!(f.browser.current_dir(), f.root);
        assert!(
            f.app.status_line().starts_with("no match "),
            "{}",
            f.app.status_line()
        );
    }

    #[test]
    fn emptying_the_query_puts_the_browser_back() {
        let mut f = Fixture::new("search-backspace");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("aerend");
        f.wait_for_search();
        assert_ne!(f.browser.current_dir(), f.root);
        for _ in 0.."aerend".len() {
            f.press(KeyCode::Backspace);
        }

        assert!(f.app.search_job.is_none(), "an empty query starts no walk");
        assert_eq!(f.browser.current_dir(), f.root);
        assert_eq!(f.browser.selected_index(), 0);
        assert!(f.app.prompt.is_some(), "the prompt stays open");
    }

    #[test]
    fn a_new_keystroke_replaces_the_walk_before_it() {
        let mut f = Fixture::new("search-replace");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("aer");
        f.type_text("end");
        f.wait_for_search();

        assert_eq!(f.selected_name(), "aerend.md");
    }

    /// Waits for `record` to hold something, which a detached program writes.
    fn wait_for_record(record: &Path) -> String {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let text = std::fs::read_to_string(record).unwrap_or_default();
            if !text.is_empty() {
                return text;
            }
            assert!(Instant::now() < deadline, "the program never ran");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// A desktop that knows one type, `application/x-zzz` (`*.zzz`), and one program for it: a
    /// script that writes the file it was given to `record`. `extra` goes in its `.desktop`.
    fn install_zzz_opener(f: &mut Fixture, record: &Path, extra: &str) {
        use std::os::unix::fs::PermissionsExt;
        let sys = f.root.join("xdg/sys");
        std::fs::create_dir_all(sys.join("applications")).unwrap();
        std::fs::create_dir_all(sys.join("mime")).unwrap();
        let script = f.root.join("rec.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf %s \"$1\" > {}\n",
                open::shell_quote(&record.to_string_lossy())
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            sys.join("applications/rec.desktop"),
            format!(
                "[Desktop Entry]\nType=Application\nName=Recorder\nExec={} %f\n{extra}\n",
                script.display()
            ),
        )
        .unwrap();
        std::fs::write(
            sys.join("applications/mimeinfo.cache"),
            "[MIME Cache]\napplication/x-zzz=rec.desktop;\n",
        )
        .unwrap();
        std::fs::write(sys.join("mime/globs2"), "50:application/x-zzz:*.zzz\n").unwrap();
        let xdg = crate::mime_type::XdgDirs {
            data_home: f.root.join("xdg/none"),
            data_dirs: vec![sys],
            config_home: f.root.join("xdg/none"),
            ..Default::default()
        };
        assert!(f.app.associations.set(Associations::load(xdg)).is_ok());
    }

    #[test]
    fn opening_a_file_uses_the_program_the_desktop_registered_for_its_type() {
        let mut f = Fixture::new("open-desktop-default");
        let record = f.root.join("record");
        install_zzz_opener(&mut f, &record, "");
        let file = f.root.join("it's a file.zzz");
        std::fs::write(&file, b"").unwrap();

        f.app.open_default(&f.browser, &file);

        assert_eq!(wait_for_record(&record), file.to_string_lossy());
    }

    #[test]
    fn an_open_rule_beats_the_desktops_default() {
        let mut f = Fixture::new("open-rule-wins");
        let (desktop_record, rule_record) = (f.root.join("desktop"), f.root.join("rule"));
        install_zzz_opener(&mut f, &desktop_record, "");
        f.app.open_rules = vec![OpenRule {
            matches: vec!["zzz".into()],
            command: format!(
                "sh -c 'printf %s \"$0\" > \"$1\"' {{}} {}",
                open::shell_quote(&rule_record.to_string_lossy())
            ),
            name: None,
            terminal: false,
        }];
        let file = f.root.join("a.zzz");
        std::fs::write(&file, b"").unwrap();

        f.app.open_default(&f.browser, &file);

        assert_eq!(wait_for_record(&rule_record), file.to_string_lossy());
        assert!(
            !desktop_record.exists(),
            "the desktop's program ran as well"
        );
    }

    #[test]
    fn a_terminal_program_takes_over_the_terminal_whatever_its_name() {
        let mut f = Fixture::new("open-terminal-entry");
        let record = f.root.join("record");
        install_zzz_opener(&mut f, &record, "Terminal=true");
        let file = f.root.join("a.zzz");
        std::fs::write(&file, b"").unwrap();

        f.app.open_default(&f.browser, &file);

        let handover = f
            .app
            .take_handover()
            .expect("Terminal=true asks for the terminal");
        assert!(handover.line.contains("rec.sh"), "{}", handover.line);
        assert!(
            !record.exists(),
            "it must not be started behind the interface"
        );
    }

    #[test]
    fn a_file_the_desktop_knows_nothing_about_falls_back_to_the_system_opener() {
        let mut f = Fixture::new("open-fallback");
        let file = f.root.join("a.unknownkind");
        std::fs::write(&file, [0u8, 1, 2]).unwrap();
        f.app.associations.set(Associations::default()).unwrap();

        f.app.open_default(&f.browser, &file);

        // Nothing registered and nothing ruled: the status names `xdg-open` (or says it is not
        // installed here), rather than opening nothing silently.
        let status = f.app.status.clone().unwrap_or_default();
        assert!(status.contains(open::DEFAULT_OPENER), "{status}");
    }

    #[test]
    fn the_menu_lists_rules_then_configured_then_installed_programs() {
        let mut f = Fixture::new("open-choices");
        let record = f.root.join("record");
        install_zzz_opener(&mut f, &record, "");
        f.app.open_rules = vec![OpenRule {
            matches: vec!["zzz".into()],
            command: "ruled".into(),
            name: Some("Ruled".into()),
            terminal: false,
        }];
        f.app.open_with = vec![OpenWith {
            name: "Configured".into(),
            command: "configured".into(),
            terminal: false,
        }];
        let names: Vec<String> = f
            .app
            .open_choices(&f.root.join("a.zzz"))
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["Ruled", "Configured", "Recorder"]);
        // A `.txt` is not a `.zzz`: the rule stays out of it.
        let plain: Vec<String> = f
            .app
            .open_choices(&f.root.join("a.txt"))
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert!(!plain.contains(&"Ruled".to_string()), "{plain:?}");
    }

    #[test]
    fn other_asks_for_a_command_and_opens_the_file_with_it() {
        let mut f = Fixture::new("open-other");
        let record = f.root.join("record");
        let file = f.root.join("x y.zzz");
        std::fs::write(&file, b"").unwrap();
        f.app.begin_open_other(file.clone());
        assert!(f.app.prompt.as_ref().is_some_and(Prompt::is_text_input));

        let command = format!(
            "sh -c 'printf %s \"$0\" > \"$1\"' {{}} {}",
            open::shell_quote(&record.to_string_lossy())
        );
        for c in command.chars() {
            let _ = f
                .app
                .handle_prompt_key(KeyCode::Char(c), &LocalVfs, &mut f.browser)
                .unwrap();
        }
        let _ = f
            .app
            .handle_prompt_key(KeyCode::Enter, &LocalVfs, &mut f.browser)
            .unwrap();

        assert!(f.app.prompt.is_none());
        assert_eq!(wait_for_record(&record), file.to_string_lossy());
    }

    #[test]
    fn other_can_be_cancelled_or_left_empty_without_running_anything() {
        let mut f = Fixture::new("open-other-cancel");
        let file = f.root.join("a.zzz");
        std::fs::write(&file, b"").unwrap();
        for keys in [vec![KeyCode::Esc], vec![KeyCode::Char(' '), KeyCode::Enter]] {
            f.app.begin_open_other(file.clone());
            for key in keys {
                let _ = f
                    .app
                    .handle_prompt_key(key, &LocalVfs, &mut f.browser)
                    .unwrap();
            }
            assert!(f.app.prompt.is_none());
            assert_eq!(f.app.status.as_deref(), Some("open with cancelled"));
            assert!(f.app.take_handover().is_none());
        }
    }

    #[test]
    fn open_with_an_interactive_program_asks_for_the_terminal_with_the_path_quoted() {
        let mut f = Fixture::new("open-interactive");
        let file = f.root.join("my notes.md");
        std::fs::write(&file, b"").unwrap();

        f.app.open_with(&f.browser, "sh", &file);
        assert_eq!(
            f.app.take_handover(),
            None,
            "sh is not on the interactive list"
        );

        f.app.interactive.push("sh".into());
        f.app.open_with(&f.browser, "sh -e", &file);
        let handover = f.app.take_handover().expect("a handover was requested");
        assert_eq!(handover.line, format!("sh -e '{}'", file.display()));
        assert_eq!(handover.cwd, f.root);
    }

    #[test]
    fn open_with_a_missing_program_says_so_instead_of_failing_silently() {
        let mut f = Fixture::new("open-missing");
        f.app
            .open_with(&f.browser, "minuteman-no-such-program", &f.root);
        assert_eq!(
            f.app.status.as_deref(),
            Some("minuteman-no-such-program: command not found")
        );
        assert_eq!(f.app.take_handover(), None);
    }

    #[test]
    fn open_with_a_window_program_starts_it_detached_on_the_quoted_path() {
        let mut f = Fixture::new("open-detached");
        let file = f.root.join("it's here.txt");
        std::fs::write(&file, b"").unwrap();
        let record = f.root.join("record");

        // `sh` stands in for a viewer: it writes the one file argument it was given (`$0`) to the
        // path it was given after it (`$1`).
        f.app.open_with(
            &f.browser,
            &format!(
                "sh -c 'printf %s \"$0\" > \"$1\"' {{}} {}",
                open::shell_quote(&record.to_string_lossy())
            ),
            &file,
        );

        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while std::fs::read_to_string(&record)
            .unwrap_or_default()
            .is_empty()
        {
            assert!(Instant::now() < deadline, "the program never ran");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            std::fs::read_to_string(&record).unwrap(),
            file.to_string_lossy()
        );
        assert!(!f.app.is_busy(), "a detached program is never waited on");
    }

    #[test]
    fn paste_into_a_folder_puts_the_copy_there_and_not_beside_the_source() {
        let mut f = Fixture::new("paste-into");
        std::fs::write(f.root.join("a.txt"), b"a").unwrap();
        std::fs::create_dir(f.root.join("dest")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        f.browser
            .select_index(f.listed().iter().position(|n| n == "a.txt").unwrap());
        f.app.yank(&f.browser);

        f.app.begin_paste_into(f.root.join("dest"));
        f.wait_for_idle();

        assert!(f.root.join("dest").join("a.txt").exists());
        assert_eq!(f.listed().iter().filter(|n| *n == "a.txt").count(), 1);
    }

    #[test]
    fn a_dropped_move_lands_in_the_folder_and_leaves_the_users_clipboard_alone() {
        let mut f = Fixture::new("drop-move");
        std::fs::write(f.root.join("a.txt"), b"a").unwrap();
        std::fs::write(f.root.join("keep.txt"), b"k").unwrap();
        std::fs::create_dir(f.root.join("dest")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        f.browser
            .select_index(f.listed().iter().position(|n| n == "keep.txt").unwrap());
        f.app.yank(&f.browser);

        f.app.begin_drop(
            vec![f.root.join("a.txt")],
            ClipboardMode::Move,
            f.root.join("dest"),
        );
        f.wait_for_idle();

        assert!(f.root.join("dest").join("a.txt").exists());
        assert!(!f.root.join("a.txt").exists());
        let held = f
            .app
            .clipboard
            .as_ref()
            .expect("the yank survived the drop");
        assert_eq!(held.paths, [f.root.join("keep.txt")]);
    }

    #[test]
    fn a_dropped_copy_keeps_the_original_and_a_moved_mark_is_forgotten() {
        let mut f = Fixture::new("drop-copy");
        std::fs::write(f.root.join("a.txt"), b"a").unwrap();
        std::fs::write(f.root.join("b.txt"), b"b").unwrap();
        std::fs::create_dir(f.root.join("dest")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        f.browser
            .select_index(f.listed().iter().position(|n| n == "b.txt").unwrap());
        f.browser.toggle_mark();

        f.app.begin_drop(
            vec![f.root.join("a.txt")],
            ClipboardMode::Copy,
            f.root.join("dest"),
        );
        f.wait_for_idle();
        assert!(f.root.join("a.txt").exists() && f.root.join("dest/a.txt").exists());

        f.app.begin_drop(
            vec![f.root.join("b.txt")],
            ClipboardMode::Move,
            f.root.join("dest"),
        );
        f.wait_for_idle();
        assert!(!f.root.join("b.txt").exists());
        assert!(
            f.browser.marked_paths().is_empty(),
            "the mark on a moved file outlived it"
        );
    }

    #[test]
    fn a_drop_onto_an_existing_name_asks_before_overwriting() {
        let mut f = Fixture::new("drop-conflict");
        std::fs::write(f.root.join("a.txt"), b"new").unwrap();
        std::fs::create_dir(f.root.join("dest")).unwrap();
        std::fs::write(f.root.join("dest/a.txt"), b"old").unwrap();
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_drop(
            vec![f.root.join("a.txt")],
            ClipboardMode::Move,
            f.root.join("dest"),
        );
        f.wait_for_idle();

        assert!(f.app.prompt.is_some(), "a conflict must raise the prompt");
        assert_eq!(std::fs::read(f.root.join("dest/a.txt")).unwrap(), b"old");
        assert!(f.root.join("a.txt").exists());
    }

    #[test]
    fn caps_lock_starts_off_and_tracks_the_last_key_event() {
        let mut f = Fixture::new("caps-lock");
        assert!(!f.app.caps_lock());

        f.app.set_caps_lock(true);
        assert!(f.app.caps_lock());

        f.app.set_caps_lock(false);
        assert!(!f.app.caps_lock());
    }
}
