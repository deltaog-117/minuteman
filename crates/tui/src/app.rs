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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use browser::BrowserState;
use crossterm::event::KeyCode;
use file_ops::{ConflictPolicy, FileOpsError, Outcome};
use shared::{LocalVfs, Vfs, VfsError};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

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
    /// Incremental filename search (`/`). `origin` is the selection index to restore on `Esc`.
    SearchInput {
        buffer: String,
        origin: usize,
    },
    /// The `:`-command prompt (`:q`, `:cd <path>`, ...).
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
}

impl BulkKind {
    fn progressing_label(&self) -> &'static str {
        match self {
            BulkKind::Paste { clip, .. } => match clip.mode {
                ClipboardMode::Copy => "copying",
                ClipboardMode::Move => "moving",
            },
            BulkKind::Delete { .. } => "deleting",
        }
    }

    fn past_label(&self) -> &'static str {
        match self {
            BulkKind::Paste { clip, .. } => match clip.mode {
                ClipboardMode::Copy => "copy",
                ClipboardMode::Move => "move",
            },
            BulkKind::Delete { .. } => "delete",
        }
    }
}

enum BulkMsg {
    Progress { path: String },
    Done(Result<Outcome, FileOpsError>),
}

/// A background operation in flight. `cancel` is `None` for `Delete`, since
/// `Vfs::remove_dir_all` is one opaque blocking call with no per-item hook to check against.
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
    handle: tokio::runtime::Handle,
}

impl App {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self {
            clipboard: None,
            prompt: None,
            status: None,
            bulk: None,
            handle,
        }
    }

    pub fn status_line(&self) -> String {
        if let Some(bulk) = &self.bulk {
            return match &bulk.kind {
                BulkKind::Delete { targets } => {
                    format!("{}… {}", bulk.kind.progressing_label(), batch_label(targets))
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
        Some(Progress {
            label: bulk.kind.progressing_label(),
            done: bulk.items_done,
            batch,
        })
    }

    /// Drains any progress/completion messages from the running background operation, if any.
    /// Call once per render tick.
    pub fn poll_bulk(&mut self, browser: &mut BrowserState, vfs: &dyn Vfs) -> Result<()> {
        let Some(bulk) = self.bulk.as_mut() else {
            return Ok(());
        };

        let mut done = None;
        while let Ok(msg) = bulk.rx.try_recv() {
            match msg {
                BulkMsg::Progress { path } => {
                    bulk.items_done += 1;
                    bulk.current = path;
                }
                BulkMsg::Done(result) => done = Some(result),
            }
        }

        let Some(result) = done else {
            return Ok(());
        };
        let bulk = self.bulk.take().expect("checked Some above");
        let past_label = bulk.kind.past_label();

        let is_delete = matches!(bulk.kind, BulkKind::Delete { .. });

        match result {
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
                BulkKind::Delete { .. } => {
                    self.status = Some("delete failed: unexpected conflict".into());
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
        self.prompt = Some(Prompt::SearchInput {
            buffer: String::new(),
            origin: browser.selected_index(),
        });
    }

    pub fn begin_command(&mut self) {
        self.prompt = Some(Prompt::CommandInput {
            buffer: String::new(),
        });
    }

    pub fn begin_paste(&mut self, browser: &BrowserState) {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        let Some(clip) = self.clipboard.clone() else {
            self.status = Some("clipboard is empty".into());
            return;
        };
        let dst_dir = browser.current_dir().to_path_buf();
        self.spawn_paste_item(clip, dst_dir, 0, ConflictPolicy::Abort);
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
                    browser.select_index(origin);
                    self.status = Some("search cancelled".into());
                }
                KeyCode::Enter => {}
                KeyCode::Backspace => {
                    buffer.pop();
                    match browser.find_match(&buffer) {
                        Some(idx) => browser.select_index(idx),
                        None if buffer.is_empty() => browser.select_index(origin),
                        None => {}
                    }
                    self.prompt = Some(Prompt::SearchInput { buffer, origin });
                }
                KeyCode::Char(c) => {
                    buffer.push(c);
                    if let Some(idx) = browser.find_match(&buffer) {
                        browser.select_index(idx);
                    }
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

    /// Parses and runs a `:`-command buffer (without the leading `:`). Unknown commands surface
    /// as a status message rather than an error — a typo shouldn't need a `Result` unwind.
    fn run_command(
        &mut self,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        buffer: &str,
    ) -> Result<ControlFlow<()>> {
        let mut parts = buffer.split_whitespace();
        let Some(cmd) = parts.next() else {
            return Ok(ControlFlow::Continue(()));
        };
        let rest = parts.collect::<Vec<_>>().join(" ");

        match cmd {
            "q" | "quit" => return Ok(ControlFlow::Break(())),
            "cd" if !rest.is_empty() => match browser.goto(vfs, Path::new(&rest)) {
                Ok(()) => self.status = Some(format!("cd {rest}")),
                Err(e) => self.status = Some(format!("cd failed: {e}")),
            },
            "cd" => self.status = Some("cd: missing path".into()),
            other => self.status = Some(format!("unknown command: {other}")),
        }
        Ok(ControlFlow::Continue(()))
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
