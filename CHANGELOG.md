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
- `theming`: `Theme` grew from 2 fields to 7 — `selection_bg`/`fg`, `border_fg`, `title_fg`,
  `dir_fg`, `file_fg`, `status_fg`. New `RawTheme` (`name: Option<String>` plus every color field
  optional) resolves via `Theme::named` (built-in `"default"`/`"dracula"` palettes, unrecognized
  names fall back to `"default"`) as a base, with individually specified fields overriding it.
- `tui`: every pane's border and title, each directory/file entry, and the status bar are now
  themed (previously only the selection highlight was). New `themed_block`/`entry_item` helpers
  in `main.rs`.
- `preview`: `is_image(path) -> bool` (extension whitelist: png/jpg/jpeg/gif/bmp/ico/tiff/tif/
  webp) and `load_image(path) -> Option<DynamicImage>` (returns `None` rather than erroring on any
  I/O or decode failure) — pure, terminal-agnostic building blocks for image preview.
- `tui`: new `image_preview` module (`ImagePreview`, `PreviewStatus`) — renders the selected
  image inline via `ratatui-image` (Kitty/iTerm2/Sixel, falling back to Unicode halfblocks).
  Decoding and resize/encoding both run on `tokio::runtime::Handle::spawn_blocking` via
  `ratatui-image`'s `ThreadProtocol`, so a large or slow-to-encode image never blocks the render
  loop. The preview pane shows "loading preview…" while decoding and "preview failed" if the
  file doesn't actually decode as an image despite its extension.
- `browser`: `BrowserState::select_index` (clamped jump to an arbitrary entry), `find_match`
  (case-insensitive substring search over the current directory's entries, always from the top),
  and `goto` (jump directly to an arbitrary directory, resolving a relative path against the
  current one) — backing the new search/command bar.
- `theming`: two new keybindable actions — `Search` (default `/`) and `Command` (default `:`).
- `tui`: `Prompt` gained `SearchInput`/`CommandInput` variants — `/` opens an incremental
  filename search that jumps the selection to the first case-insensitive match as you type and
  restores the original selection on `Esc`; `:` opens a command prompt supporting `:q`/`:quit`
  (exit the app) and `:cd <path>` (jump to a directory). `handle_prompt_key` now returns
  `ControlFlow<()>` so a `:q`/`:quit` command can signal the app to exit.
- `preview`: `is_text(path) -> bool` (extension whitelist covering common source/config/doc
  formats, plus a filename whitelist for extensionless files like `Makefile`/`.gitignore`) and
  `load_text(path) -> Option<String>` (returns `None` for a file over 1 MiB, containing a null
  byte, or not valid UTF-8) — pure, terminal-agnostic building blocks for text/code preview,
  mirroring `is_image`/`load_image`'s shape.
- `tui`: new `text_preview` module (`TextPreview`, `PreviewStatus`) — reads the selected text/code
  file off `tokio::runtime::Handle::spawn_blocking` and renders it, word-wrapped, in the preview
  pane, the same never-block-the-render-loop treatment `image_preview` gives image decoding.
  Shows "loading preview…" while reading and "preview failed" for anything over the size cap or
  not valid UTF-8 despite a text-like name.
- `tui`: `ImagePreview` and `TextPreview` are now constructed and driven together via a new
  `Previews` struct, keeping `run`/`draw`'s argument counts down now that there are two preview
  pipelines instead of one.
- `shell_overlay`: new `PopupShell` type — spawns `$SHELL` on its own `portable-pty` pty and
  parses its output into a `vt100::Parser` screen buffer on a background thread. New
  `write_input`/`resize`/`with_screen`/`try_wait` methods; `try_wait` returns a local
  `ExitOutcome` rather than a `portable_pty` type, keeping that dependency contained to this
  crate.
- `tui`: new `popup_shell` module — sizes a centered popup (80%×70% of the frame), encodes
  `KeyEvent`s into the raw escape sequences a real terminal sends (arrows, function keys,
  `Ctrl`/`Alt` combos), and renders the `vt100` screen buffer cell-by-cell as styled `Span`s
  (color/bold/italic/underline/inverse), including cursor positioning.
- `tui`: `s` now opens the popup shell instead of suspending to a full-screen shell — every
  keystroke forwards straight to the popup's pty while it's open, `Event::Resize` resizes the
  pty to match, and the popup closes itself (with a "shell exited" status) the moment the child
  exits. The old full-screen `spawn_shell` path is unchanged in `shell_overlay`, just no longer
  bound to any keybinding.

- `shell_overlay`: `PopupShell::close` — kills and reaps the child shell immediately, for a
  caller-initiated close rather than waiting for it to exit on its own.
- `tui`: `Esc` now closes the popup shell (kills the child, restores the underlying UI, sets a
  "shell closed" status) instead of being forwarded to it as input.

### Removed
- `tui`: `TerminalGuard::suspend`/`resume`, now dead code after the popup shell replaced their
  only caller.

## [0.1.0] - 2026-09-16

### Added
- Initial release: bare-bones, runnable local filesystem browser establishing the workspace
  architecture as a north star for the rest of `ROADMAP.md`.
