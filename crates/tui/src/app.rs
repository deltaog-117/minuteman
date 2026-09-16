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

//! Clipboard + prompt state driving `file_ops` from the TUI. Keeps `main.rs`'s event loop a
//! thin `Action -> App method` dispatcher, the same way `browser::BrowserState` keeps it thin
//! for navigation.

use std::path::{Path, PathBuf};

use anyhow::Result;
use browser::BrowserState;
use crossterm::event::KeyCode;
use file_ops::{ConflictPolicy, FileOpsError, Outcome};
use shared::{Vfs, VfsError};

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
    RenameInput { target: PathBuf, buffer: String },
    CreateInput { buffer: String },
    ConfirmDelete { target: PathBuf },
    Conflict(ConflictSource),
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
        }
    }
}

#[derive(Debug, Default)]
pub struct App {
    pub clipboard: Option<Clipboard>,
    pub prompt: Option<Prompt>,
    pub status: Option<String>,
}

impl App {
    pub fn status_line(&self) -> String {
        match &self.prompt {
            Some(prompt) => prompt.display(),
            None => self.status.clone().unwrap_or_default(),
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

    pub fn begin_paste(&mut self, vfs: &dyn Vfs, browser: &mut BrowserState) -> Result<()> {
        let Some(clip) = self.clipboard.clone() else {
            self.status = Some("clipboard is empty".into());
            return Ok(());
        };
        let Some(name) = clip.path.file_name() else {
            self.status = Some("clipboard entry has no file name".into());
            return Ok(());
        };
        let dst = browser.current_dir().join(name);
        self.perform_paste(vfs, browser, clip, dst, ConflictPolicy::Abort)
    }

    /// Routes a raw key to the active prompt. No-op if there is no active prompt.
    pub fn handle_prompt_key(
        &mut self,
        code: KeyCode,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
    ) -> Result<()> {
        let Some(prompt) = self.prompt.take() else {
            return Ok(());
        };

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
                KeyCode::Char('y') => match file_ops::delete(vfs, &target) {
                    Ok(()) => {
                        browser.reload(vfs)?;
                        self.status = Some(format!("deleted {}", display_name(&target)));
                    }
                    Err(e) => self.status = Some(format!("delete failed: {e}")),
                },
                _ => self.status = Some("delete cancelled".into()),
            },
            Prompt::Conflict(source) => match code {
                KeyCode::Char('o') => {
                    self.resolve_conflict(vfs, browser, source, ConflictPolicy::Overwrite)?;
                }
                KeyCode::Char('s') => {
                    self.resolve_conflict(vfs, browser, source, ConflictPolicy::Skip)?;
                }
                _ => self.status = Some("aborted".into()),
            },
        }
        Ok(())
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
                self.perform_paste(vfs, browser, clip, dst, policy)
            }
            ConflictSource::Rename { target, new_name } => {
                self.finish_rename(vfs, browser, target, new_name, policy)
            }
        }
    }

    fn perform_paste(
        &mut self,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        clip: Clipboard,
        dst: PathBuf,
        policy: ConflictPolicy,
    ) -> Result<()> {
        let result = match clip.mode {
            ClipboardMode::Copy => file_ops::copy(vfs, &clip.path, &dst, policy),
            ClipboardMode::Move => file_ops::mv(vfs, &clip.path, &dst, policy),
        };

        match result {
            Ok(Outcome::Completed) => {
                browser.reload(vfs)?;
                self.status = Some(format!("pasted {}", display_name(&dst)));
                if clip.mode == ClipboardMode::Move {
                    self.clipboard = None;
                }
            }
            Ok(Outcome::Skipped) => self.status = Some("paste skipped".into()),
            Err(FileOpsError::Vfs(VfsError::AlreadyExists(_))) => {
                self.prompt = Some(Prompt::Conflict(ConflictSource::Paste { clip, dst }));
            }
            Err(e) => self.status = Some(format!("paste failed: {e}")),
        }
        Ok(())
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
