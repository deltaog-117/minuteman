# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Cargo workspace scaffold: 10 feature-first crates (`shared`, `vfs_ssh`, `browser`,
  `file_ops`, `preview`, `shell_overlay`, `trash`, `plugins`, `theming`, `tui`).
- `shared`: `Vfs` trait and a synchronous `LocalVfs` implementation over `std::fs`, with
  directories sorted before files.
- `browser`: miller-column navigation state (`BrowserState`) — enter/leave a directory, move
  selection up/down, restores the previous selection when leaving a directory (like Ranger).
- `theming`: `Config` loading from `~/.config/minuteman/config.toml` (TOML), with an
  action-to-keys keybinding schema and per-field fallback to built-in defaults when the file is
  missing, partial, or fails to parse.
- `tui`: the `minuteman` binary — a 3-pane (parent / current / selection preview) ratatui TUI
  over the local filesystem, with vim-style navigation (`j`/`k`/`h`/`l`/Enter/`q`) resolved
  through the config-driven keymap. Terminal state (raw mode, alternate screen) is always
  restored on exit via an RAII guard.
- `shared`: `Vfs` trait extended with `exists`, `create_dir`, `create_file`, `copy_file`,
  `rename`, `remove_file`, `remove_dir_all`; `LocalVfs` implements all of them over `std::fs`,
  never silently truncating an existing file or overwriting an existing copy/rename destination.
  New `VfsError::NotFound` / `VfsError::AlreadyExists` variants.
- `file_ops`: `copy`/`mv`/`delete`/`create_directory`/`create_new_file`/`rename` orchestration on
  top of `Vfs` — recursive directory copy, a guard against a destination nested inside its own
  source, and a guard against a no-op same-path operation.
- `file_ops`: `ConflictPolicy` (`Abort`/`Skip`/`Overwrite`) and `Outcome` (`Completed`/`Skipped`),
  threaded through `copy`/`mv`/`rename` so callers can resolve an existing-destination conflict
  instead of just erroring.
- `theming`: six new keybindable actions — `Yank`, `Cut`, `Paste`, `Delete`, `Rename`, `Create` —
  with defaults `y`/`m`/`p`/`d`/`r`/`n`.
- `browser`: `BrowserState::reload` — re-lists the current/parent directories after a filesystem
  mutation made outside `BrowserState` (copy/move/delete/create/rename).
- `tui`: new `app` module (`App`, `Clipboard`, `Prompt`) driving `file_ops` from the keyboard —
  yank/cut mark a clipboard entry, paste copies or moves it into the current directory, delete
  asks for `y`/N confirmation (permanent — no trash yet), rename opens a prompt pre-filled with
  the current name, and create prompts for a name (trailing `/` makes a directory). A conflicting
  paste/rename surfaces an overwrite/skip/abort prompt. A new bottom status/prompt bar in the TUI
  shows the active prompt or the last operation's result.
- `file_ops`: `copy_with_progress`/`mv_with_progress` — the same copy/move logic with a
  `&mut dyn FnMut(&Path) -> ControlFlow<()>` callback fired once per file/directory processed;
  returning `ControlFlow::Break(())` aborts with the new `FileOpsError::Cancelled`. `copy`/`mv`
  are now thin wrappers over these with a no-op callback (unchanged signatures/behavior).
- `file_ops`: `mv`/`mv_with_progress` now fall back to copy + delete when `Vfs::rename` fails
  with `ErrorKind::CrossesDevices` (moving across filesystems), instead of just erroring.
- `tui`: paste and delete now run on a `tokio::runtime::Handle::spawn_blocking` thread pool
  instead of inline, so a large copy/move/delete no longer freezes the render loop. The event
  loop switched from a blocking `event::read()` to a 100ms `event::poll` so it keeps redrawing
  progress even without a keypress. Copy/move show live "N done (name)" progress and are
  cancellable with `Esc`; delete shows an indeterminate "deleting…" status (no per-file hook to
  report through). While an operation is in flight, all other actions — including quit — are
  blocked except `Esc`, so the process can't exit mid-write.
- `shell_overlay`: new crate — `spawn_shell(cwd: &Path) -> io::Result<ExitStatus>` spawns
  `$SHELL` (falling back to `/bin/sh`) inheriting stdio directly, blocking until it exits.
- `theming`: new `Shell` keybindable action, default `s`.
- `tui`: `s` suspends raw mode/the alternate screen, drops into a real interactive shell in the
  browser's current directory, then resumes and forces a full redraw via `Terminal::resize`
  (not `Terminal::clear`, which depends on a cursor-position query that can hang — see
  `DIARY.md`). `TerminalGuard` gained `suspend`/`resume` methods for this.

## [0.1.0] - 2026-09-16

### Added
- Initial release: bare-bones, runnable local filesystem browser establishing the workspace
  architecture as a north star for the rest of `ROADMAP.md`.
