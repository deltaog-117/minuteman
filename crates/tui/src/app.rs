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
    pub path: PathBuf,
    pub mode: ClipboardMode,
}

#[derive(Debug)]
pub enum ConflictSource {
    Paste { clip: Clipboard, dst: PathBuf },
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
    ConfirmDelete {
        target: PathBuf,
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
    pub fn display(&self) -> String {
        match self {
            Prompt::RenameInput { buffer, .. } => format!("rename: {buffer}"),
            Prompt::CreateInput { buffer } => {
                format!("create (end with / for a directory): {buffer}")
            }
            Prompt::ConfirmDelete { target } => {
                format!("delete '{}' permanently? (y/N)", display_name(target))
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
#[derive(Debug)]
enum BulkKind {
    Paste { clip: Clipboard, dst: PathBuf },
    Delete { target: PathBuf },
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
                BulkKind::Delete { target } => format!(
                    "{}… {}",
                    bulk.kind.progressing_label(),
                    display_name(target)
                ),
                BulkKind::Paste { .. } => {
                    let cancel_hint = if bulk.cancel.is_some() {
                        " — Esc to cancel"
                    } else {
                        ""
                    };
                    format!(
                        "{}… {} done ({}){cancel_hint}",
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

        match result {
            Ok(Outcome::Completed) => {
                browser.reload(vfs)?;
                if let BulkKind::Paste { clip, .. } = &bulk.kind
                    && clip.mode == ClipboardMode::Move
                {
                    self.clipboard = None;
                }
                self.status = Some(format!("{past_label} complete"));
            }
            Ok(Outcome::Skipped) => self.status = Some(format!("{past_label} skipped")),
            Err(FileOpsError::Cancelled) => self.status = Some(format!("{past_label} cancelled")),
            Err(FileOpsError::Vfs(VfsError::AlreadyExists(_))) => match bulk.kind {
                BulkKind::Paste { clip, dst } => {
                    self.prompt = Some(Prompt::Conflict(ConflictSource::Paste { clip, dst }));
                }
                BulkKind::Delete { .. } => {
                    self.status = Some("delete failed: unexpected conflict".into());
                }
            },
            Err(e) => self.status = Some(format!("{past_label} failed: {e}")),
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

    pub fn yank(&mut self, browser: &BrowserState) {
        match browser.selected_entry() {
            Some(entry) => {
                self.clipboard = Some(Clipboard {
                    path: entry.path.clone(),
                    mode: ClipboardMode::Copy,
                });
                self.status = Some(format!("yanked {}", entry.name));
            }
            None => self.status = Some("nothing selected".into()),
        }
    }

    pub fn cut(&mut self, browser: &BrowserState) {
        match browser.selected_entry() {
            Some(entry) => {
                self.clipboard = Some(Clipboard {
                    path: entry.path.clone(),
                    mode: ClipboardMode::Move,
                });
                self.status = Some(format!("marked {} to move", entry.name));
            }
            None => self.status = Some("nothing selected".into()),
        }
    }

    pub fn begin_delete(&mut self, browser: &BrowserState) {
        match browser.selected_entry() {
            Some(entry) => {
                self.prompt = Some(Prompt::ConfirmDelete {
                    target: entry.path.clone(),
                });
            }
            None => self.status = Some("nothing selected".into()),
        }
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
        let Some(name) = clip.path.file_name() else {
            self.status = Some("clipboard entry has no file name".into());
            return;
        };
        let dst = browser.current_dir().join(name);
        self.spawn_paste(clip, dst, ConflictPolicy::Abort);
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
            Prompt::ConfirmDelete { target } => match code {
                KeyCode::Char('y') => self.spawn_delete(target),
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
            ConflictSource::Paste { clip, dst } => {
                self.spawn_paste(clip, dst, policy);
                Ok(())
            }
            ConflictSource::Rename { target, new_name } => {
                self.finish_rename(vfs, browser, target, new_name, policy)
            }
        }
    }

    /// Spawns a copy or move on tokio's blocking thread pool. Progress and the final result
    /// arrive later via `poll_bulk` — this call itself never blocks.
    fn spawn_paste(&mut self, clip: Clipboard, dst: PathBuf, policy: ConflictPolicy) {
        let (tx, rx): (UnboundedSender<BulkMsg>, _) = unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_bg = Arc::clone(&cancel);
        let src = clip.path.clone();
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
            kind: BulkKind::Paste { clip, dst },
            items_done: 0,
            current: String::new(),
            cancel: Some(cancel),
            rx,
        });
    }

    /// Spawns a delete on tokio's blocking thread pool. No per-item progress (see `BulkOp`
    /// doc), but it still keeps a large recursive delete from freezing the render loop.
    fn spawn_delete(&mut self, target: PathBuf) {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        let (tx, rx) = unbounded_channel();
        let target_bg = target.clone();

        self.handle.spawn_blocking(move || {
            let vfs = LocalVfs;
            let result = file_ops::delete(&vfs, &target_bg).map(|()| Outcome::Completed);
            let _ = tx.send(BulkMsg::Done(result));
        });

        self.bulk = Some(BulkOp {
            kind: BulkKind::Delete { target },
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
