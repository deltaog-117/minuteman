# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `tui`: header row with a breadcrumb path (`~` for home, `…` for elided middle segments) and
  pills for marked entries, the clipboard (`⧉ N yanked` / `✂ N cut`), and a gauge for a running
  copy/move/delete.
- `tui`: right-aligned size and relative-age columns in the current pane (`14K`, `2h`), dropping
  age and then size when the pane is too narrow to keep names readable.
- `tui`: scrollbar over the current pane's right border when the list overflows.
- `tui`: powerline-style status bar: a colored mode pill (`NORMAL`, `SHELL`, `LEADER`, `RESIZE`,
  `MOVE`, `BUSY`, or the prompt's label), the selection's name, permissions, size or item count,
  and type, the position (`3/49`), and key hints drawn from the user's own bindings that change
  with the mode. Text prompts show a cursor.
- `theming`: `[theme]` fields `bar_bg`, `danger_fg` and `separator` (`"flat"` or `"arrow"`, the
  latter needing a Powerline/Nerd Font), and `KeyMap::keys_for`.
- `shared`: `DirEntryInfo` gained `size`, `modified` and `mode`, filled from the `stat` the local
  listing already made per entry.
- `tui`: new neon look. Rounded frames; the active pane (the current directory's column, or the
  focused shell) gets a bright border and an accent-colored bold title while the others dim; the
  selected row gets an accent stripe (`▌`) over a background highlight and keeps its own color;
  marked rows show an accent `*`. Entries are colored by kind (directory, source, config, doc,
  archive, media) from their extension, dotfiles counting as config.
- `theming`: `#rrggbb` / `#rgb` hex colors in `[theme]`. Hex is drawn in true color when
  `COLORTERM` is `truecolor`/`24bit` and quantized to the nearest xterm-256 color otherwise.
- `theming`: new `[theme]` fields `border_focused_fg`, `accent_fg`, `source_fg`, `config_fg`,
  `doc_fg`, `archive_fg`, `media_fg` and `border_type` (`rounded`/`plain`/`double`/`thick`);
  `selection_fg = "keep"` leaves the selected entry in its file-type color. New built-in
  palettes `neon` and `classic`.
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
- `theming`: two new keybindable actions — `Select` (default `v`, Ranger-style toggle-mark) and
  `Leader` (default `space`, an inert placeholder reserved for future chorded commands).
- `browser`: `BrowserState` gained a mark set (`toggle_mark`/`is_marked`/`marked_paths`/
  `prune_marks`) — marks persist across directory navigation until toggled off or consumed by a
  bulk action.
- `tui`: `v` toggles a mark on the current entry (shown with a `* ` prefix in the current pane);
  `d` (delete) now acts on every marked entry when any are marked, falling back to the single
  cursor selection otherwise, with a pluralized confirmation prompt and per-item progress.
  `space` is bound to the new inert `Leader` action and never blocks or delays any other key.
- Added `config.example.toml` at the repo root — every `[keys]`/`[theme]` field spelled out with
  its built-in default, to copy to `~/.config/minuteman/config.toml` as a starting point. A new
  `theming` test parses it and asserts it resolves to exactly `RawConfig::default()`, so it can't
  silently drift out of sync with a future default change.

- `theming`: two new keybindable actions — `ShellFocus` (default `tab`) and `ShellMove` (default
  `g`).
- `tui`: while the popup shell is open, `tab` toggles whether keystrokes go to the shell or drive
  the browser underneath it, so the shell can stay open and visible while you keep browsing.
  `g` (only reachable while the popup is open and unfocused) enters move mode — `h`/`j`/`k`/`l`
  or arrow keys reposition the popup, `enter`/`esc` confirms. `popup_shell::popup_area` now takes
  an `(i32, i32)` offset, clamped so the popup can never be nudged off-screen.

- `tui`: new `shell_layout` module — a binary split-pane tree (`ShellPanes`) replacing the single
  floating popup shell with tmux-style tiled panes. The tree/geometry logic (`split_rect`,
  `Divider`, tree search/mutation) is generic over the leaf payload so it's unit-testable without
  spawning real shells; `ShellPanes` specializes it to `shell_overlay::PopupShell` and owns
  spawning, resizing, rendering, and reaping exited panes.
- `theming`: `Action::ShellMove` replaced with three new actions — `ShellSplitHorizontal`
  (default `%`), `ShellSplitVertical` (default `"`), and `ShellPaneNext` (default `o`), matching
  tmux's own default split bindings for muscle memory.
- `tui`: `s` opens the first shell pane tiled into the frame (docked below the status bar, not
  floating); `%`/`"` split the focused pane side by side or stacked, spawning a new shell in the
  current directory and focusing it; `o` cycles keyboard focus to the next pane. `Esc` now closes
  just the focused pane, promoting its sibling to fill the freed space, and only exits shell mode
  entirely once it's the last pane open.
- `tui`: mouse support — `EnableMouseCapture` is now on for the whole session. Dragging a
  divider (the shared border between two sibling panes) live-resizes their split ratio; clicking
  inside a pane focuses it (and refocuses keyboard input to the shell if it was browsing).
  Dragging a pane to reposition it and reordering panes are explicitly out of scope (see
  `ROADMAP.md` and `DIARY.md`) — panes are tiled, not floating, so there's no independent
  position to drag.
- `tui`: the focused pane's border is now highlighted with the theme's `selection_bg` color, so
  which pane keystrokes route to is visible at a glance once more than one is open.
- `tui`: `shell_area` now returns a centered 80%-width/70%-height box within the browser region
  instead of the whole thing, so shell panes stay contained to a mini floating area (matching the
  original single popup's sizing) instead of tiling across the entire screen. `draw` now calls
  `shell_area` directly rather than duplicating its layout math.
- `tui`: the shell box itself can now be dragged around the screen by its title bar, like a
  floating window — `shell_area` takes a `(dx, dy)` offset from centered, clamped so the box can
  never be dragged off screen. A mouse-down on the box's top border (where the "shell" title
  renders) starts the drag; `Drag` events accumulate into the offset, `Up` ends it. The offset
  resets to `(0, 0)` on every new `s` spawn. This moves the whole box, not individual panes inside
  it, which stay tiled exactly as before (see `ROADMAP.md`/`DIARY.md` for why per-pane dragging is
  still out of scope). New `ShellView` struct bundles the pane tree with its offset for `draw`,
  keeping its argument count from growing.
- `tui`: batch file operations — `y`/`m` now yank/cut every currently marked path (falling back
  to the cursor entry if nothing's marked), and `p` pastes the whole batch one item at a time,
  re-spawning the next item itself as each one finishes so a multi-item paste stays one
  continuous background operation. A conflict on any item still pauses the batch on the existing
  overwrite/skip/abort prompt and resumes afterward — including `Skip`, which now actually
  continues a batch past a conflicting item instead of ending the whole paste. The status line
  shows an `[i/N]` progress hint for batches of more than one item.
- `tui`: keyboard resize/move for shell panes. `space` (`Action::Leader`) now starts a chord
  while any shell pane is open — `r` enters a resize mode where `hjkl`/arrows nudge the focused
  pane's nearest divider by 5% per press, falling back to growing/shrinking the box's own
  width/height by 2 cells per press when there's no divider along that axis (a lone pane, or a
  tree only ever split the other way); `m` enters a move mode where the same keys nudge the
  whole box's offset by 2 cells per press. `Esc` leaves either mode; any other key leaves it too
  but is still dispatched normally afterward. New `shell_layout::ShellPanes::resize_focused`
  (now returning whether it actually adjusted a divider) and `shell_layout::NudgeDir`; `shell_area`
  gained a `size_adjust: (i32, i32)` parameter alongside its existing `offset`.
- `tui`: the shell box's own right border is now mouse-draggable, resizing its width directly
  (checked before the title-bar drag hit-test, so the shared top-right corner prefers resize).
  `space t` — a third, one-shot branch of the leader chord — flips the focused pane's split
  between side-by-side and stacked, keeping the same two panes and their ratio. New
  `shell_layout::ShellPanes::toggle_focused_orientation`.
- `theming`: new `QuitToCwd` keybindable action (`quit_to_cwd`), default `Q`. Single-character
  key names in `[keys]` are now case-sensitive (`"Q"` is Shift+q, distinct from `"q"`); named keys
  such as `"Enter"`/`"space"` still match case-insensitively.
- `tui`: `Q` quits and records the directory being browsed into the file named by the new
  `--cwd-file <path>` option; `q` and `:q` never write it. Without `--cwd-file`, `Q` is a plain
  quit.
- `tui`: new `minuteman init <bash|zsh|fish>` subcommand prints an `mm` shell wrapper that runs
  minuteman with `--cwd-file` and `cd`s the parent shell afterward, since a child process cannot
  change its parent's directory. Unknown flags and extra positional arguments are now rejected
  with exit code 2 instead of being taken as the start directory.

### Changed
- `tui`: the current pane's frame is titled with the directory's own name; the full path lives
  in the new header. Rows in that pane now start with a two-cell gutter (selection stripe, mark).
- `tui`: `Esc`/`space`/resize/typing states are shown by the status bar's mode pill and hints
  instead of one-off messages ("resize mode — ...", "shell focused — ..."). Status messages now
  clear after five seconds instead of staying until replaced.
- `theming`: the default theme is now `neon` (true color). The previous 16-color look is
  `name = "classic"`; `name = "default"` and unknown names resolve to `neon`.
- `tui`: the mini-shell frame now uses the shared pane frame, so a focused shell is outlined in
  `border_focused_fg` (previously `selection_bg`).
- `tui`: `space` is now the single leader key for all mini-shell commands: `space space` goes
  back to typing in the shell, `space h`/`j`/`k`/`l` (or arrows) moves focus between panes,
  `space |` and `space -` split side by side / stacked, `space x` closes the focused pane, next to
  the existing `space r`/`m`/`t`. Pressing `space` lists them in the status bar.
- `tui`: while typing in a shell, `Esc` now leaves typing mode instead of closing the pane (close
  with `space x`), and every other key — `Space`, `Tab`, `o`, `%`, `"` included — is sent to the
  shell.
- `tui`: `Clipboard.path: PathBuf` is now `Clipboard.paths: Vec<PathBuf>`.

### Removed
- `theming`: `Action::ShellFocus`/`ShellSplitHorizontal`/`ShellSplitVertical`/`ShellPaneNext` and
  their `shell_focus`/`shell_split_horizontal`/`shell_split_vertical`/`shell_pane_next` config keys
  (`Tab`, `%`, `"`, `o`), replaced by the `space` leader commands above. Old config files that
  still set them keep loading; the keys are ignored.
- `tui`: `TerminalGuard::suspend`/`resume`, now dead code after the popup shell replaced their
  only caller.
- `tui`: `popup_shell::popup_area` and `theming::Action::ShellMove`/`RawKeyMap::shell_move` —
  floating-popup positioning is gone now that shell panes are tiled by `shell_layout::ShellPanes`
  instead.

## [0.1.0] - 2026-09-16

### Added
- Initial release: bare-bones, runnable local filesystem browser establishing the workspace
  architecture as a north star for the rest of `ROADMAP.md`.
