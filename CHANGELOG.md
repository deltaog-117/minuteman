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
  source, and a guard against a no-op same-path operation. Not yet wired into the TUI.

## [0.1.0] - 2026-09-16

### Added
- Initial release: bare-bones, runnable local filesystem browser establishing the workspace
  architecture as a north star for the rest of `ROADMAP.md`.
