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
use file_ops::{ConflictPolicy, FileOpsError, Outcome};
use shared::{LocalVfs, Vfs, VfsError};
use shell_overlay::CommandOutcome;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::command::{self, Command};
use crate::inspect::InspectView;
use crate::live_refresh::LiveRefresh;
use crate::open;
use crate::search_job::{SearchJob, SearchState};

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
    Rename { target: PathBuf, new_name: String },
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
}

impl Prompt {
    /// The status bar's mode label for this prompt.
    pub fn label(&self) -> &'static str {
        match self {
            Prompt::RenameInput { .. } => "RENAME",
            Prompt::CreateInput { .. } => "CREATE",
            Prompt::ConfirmDelete { .. } => "DELETE",
            Prompt::Conflict(_) => "CONFLICT",
            Prompt::SearchInput { .. } => "SEARCH",
            Prompt::CommandInput { .. } => "COMMAND",
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
            Prompt::Conflict(ConflictSource::Paste { dst, .. }) => format!(
                "'{}' already exists — overwrite / skip / abort? (o/s/a)",
                display_name(dst)
            ),
            Prompt::Conflict(ConflictSource::Rename { new_name, .. }) => {
                format!("'{new_name}' already exists — overwrite / skip / abort? (o/s/a)")
            }
            Prompt::SearchInput { buffer, .. } => format!("/{buffer}"),
            Prompt::CommandInput { buffer } => format!(":{buffer}"),
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
    Delete { targets: Vec<PathBuf> },
    /// A `:` command line running under `sh -c`, from `started`.
    Shell { command: String, started: Instant },
}

impl BulkKind {
    fn progressing_label(&self) -> &'static str {
        match self {
            BulkKind::Paste { clip, .. } => match clip.mode {
                ClipboardMode::Copy => "copying",
                ClipboardMode::Move => "moving",
            },
            BulkKind::Delete { .. } => "deleting",
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
            BulkKind::Shell { .. } => "command",
        }
    }
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
    handle: tokio::runtime::Handle,
    /// Program names a `:` command hands the terminal to (see `command::parse`).
    interactive: Vec<String>,
    handover: Option<Handover>,
    /// The `/` prompt's search in flight, if any. Only ever `Some` while that prompt is open.
    search_job: Option<SearchJob>,
    search_state: SearchState,
}

impl App {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self {
            clipboard: None,
            prompt: None,
            status: None,
            bulk: None,
            live: LiveRefresh::new(handle.clone()),
            handle,
            interactive: Vec::new(),
            handover: None,
            search_job: None,
            search_state: SearchState::Idle,
        }
    }

    /// Sets which program names `:` hands the terminal to.
    pub fn with_interactive_commands(mut self, interactive: Vec<String>) -> Self {
        self.interactive = interactive;
        self
    }

    pub fn status_line(&self) -> String {
        if let Some(bulk) = &self.bulk {
            return match &bulk.kind {
                BulkKind::Delete { targets } => {
                    format!("{}… {}", bulk.kind.progressing_label(), batch_label(targets))
                }
                BulkKind::Shell { command, .. } => {
                    format!("{}… {command} — Esc to cancel", bulk.kind.progressing_label())
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

        let is_delete = matches!(bulk.kind, BulkKind::Delete { .. });
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
                if let BulkKind::Paste { clip, dst_dir, index, .. } = bulk.kind {
                    let Some(clip) = self.try_continue_paste(clip, dst_dir, index) else {
                        return Ok(());
                    };
                    if clip.mode == ClipboardMode::Move {
                        self.clipboard = None;
                    }
                } else if is_delete {
                    browser.prune_marks(vfs);
                }
                self.status = Some(format!("{past_label} complete"));
            }
            Ok(Outcome::Skipped) => {
                if let BulkKind::Paste { clip, dst_dir, index, .. } = bulk.kind
                    && self.try_continue_paste(clip, dst_dir, index).is_none()
                {
                    return Ok(());
                }
                self.status = Some(format!("{past_label} skipped"));
            }
            Err(FileOpsError::Cancelled) => self.status = Some(format!("{past_label} cancelled")),
            Err(FileOpsError::Vfs(VfsError::AlreadyExists(_))) => match bulk.kind {
                BulkKind::Paste { clip, dst_dir, index, dst } => {
                    self.prompt = Some(Prompt::Conflict(ConflictSource::Paste {
                        clip,
                        dst_dir,
                        index,
                        dst,
                    }));
                }
                BulkKind::Delete { .. } | BulkKind::Shell { .. } => {
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

    /// Marks (via `Select`) win over the cursor: if any entries are marked, delete confirms
    /// against the whole marked set; otherwise it falls back to the single entry under the
    /// cursor, matching Ranger's "act on marks if any, else the current file" convention.
    pub fn begin_delete(&mut self, browser: &BrowserState) {
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

    /// Opens `path` with whatever the desktop has registered for its type.
    pub fn open_default(&mut self, browser: &BrowserState, path: &Path) {
        self.open_with(browser, open::DEFAULT_OPENER, path);
    }

    /// Opens `path` with `command` (see `open::command_line` for where the path goes). A program
    /// in the `interactive_commands` list takes over the terminal like a `:` command does; any
    /// other is started on its own, since it opens a window and must not be waited on.
    pub fn open_with(&mut self, browser: &BrowserState, command: &str, path: &Path) {
        let Some(program) = open::program_of(command) else {
            self.status = Some("open with: the command is empty".into());
            return;
        };
        // `VAR=value prog` is shell syntax, not a program name to look for.
        if !program.contains('=') && !open::program_exists(program) {
            self.status = Some(format!("{program}: command not found"));
            return;
        }
        let line = open::command_line(command, path);
        if command::names_interactive_program(program, &self.interactive) {
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
                Ok(()) => format!("opening {} with {program}", display_name(path)),
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
            Ok(Some(Command::Mkdir { parents: true, names })) => {
                self.run_builtin(vfs, browser, "mkdir", &names, file_ops::create_directory_all)?;
            }
            Ok(Some(Command::Mkdir { parents: false, names })) => {
                self.run_builtin(vfs, browser, "mkdir", &names, file_ops::create_directory)?;
            }
            Ok(Some(Command::Touch { names })) => {
                self.run_builtin(vfs, browser, "touch", &names, file_ops::touch)?;
            }
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
        shown.push_str(&format!(" (+{} more lines)", lines.len() - STATUS_OUTPUT_LINES));
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
}
