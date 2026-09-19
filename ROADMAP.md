# Roadmap: Minuteman

This document outlines the future direction of Minuteman, a Ranger-inspired terminal file
manager written in Rust, focused on fast async bulk file operations, a themeable TUI, and a
stable, multi-language plugin system. Items are organized by priority, not by timeline.

---

## ✅ Completed (Milestones Achieved)

- ✅ **v0.1.0 — bare-bones runnable browser.** Cargo workspace scaffold (10 feature-first
  crates: `shared`, `vfs_ssh`, `browser`, `file_ops`, `preview`, `shell_overlay`, `trash`,
  `plugins`, `theming`, `tui`), each with its own `Cargo.toml`/`src`/`tests`.
- ✅ **Local filesystem browser (miller columns)** – 3-pane Ranger-style navigation (parent |
  current | selection preview) over the local filesystem, backed by a `Vfs` trait + synchronous
  `LocalVfs` implementation in `shared`.
- ✅ **Vim-like keybindings, config-driven from day one** – `theming::Config` loads
  `~/.config/minuteman/config.toml` (action-to-keys mapping), falling back per-field to built-in
  defaults (`j`/`k`/`h`/`l`/`enter`/`q`) when the file is missing or a field is unspecified. A
  malformed config never prevents startup — it logs and falls back to defaults.
- ✅ **Core file operations (sync)** – `Vfs` extended with `copy_file`/`create_dir`/
  `create_file`/`rename`/`remove_file`/`remove_dir_all`/`exists`; `file_ops` orchestrates
  copy/move/delete/create/rename on top, recursing for directories, rejecting a destination
  inside its own source, and never silently overwriting an existing destination. No
  cross-filesystem move fallback yet.
- ✅ **File operations wired into the TUI** – `y`/`m`/`p`/`d`/`r`/`n` (yank, cut, paste, delete,
  rename, create) drive `file_ops` through a new `tui::app::App` (clipboard + prompt state) and
  a bottom status/prompt bar. Delete asks for `y`/N confirmation (no trash yet, so this is
  permanent); a conflicting paste/rename surfaces an overwrite/skip/abort prompt backed by
  `file_ops::ConflictPolicy`. Verified against the real compiled binary via two scripted PTY
  sessions (create/yank/paste/rename/delete, and cut+conflict-abort+conflict-overwrite),
  checking actual filesystem end-state — not just unit tests.
- ✅ **Async bulk file operations with progress** – paste and delete now run on a `tokio`
  blocking thread pool (`tokio::runtime::Handle::spawn_blocking`) instead of inline, so the TUI
  never freezes during a large copy/move/delete. `file_ops` gained `copy_with_progress`/
  `mv_with_progress` (the old `copy`/`mv` are now thin wrappers — zero signature-break for
  existing callers/tests) plus the cross-filesystem move fallback (copy+delete on
  `ErrorKind::CrossesDevices`) promised last cycle. Copy/move report one progress tick per file
  and are cancellable mid-flight (`Esc`); delete has no per-file hook (`Vfs::remove_dir_all` is
  one opaque call) so it just runs off-thread with an indeterminate "deleting…" status and no
  cancel. While an operation is in flight, all other actions (including quit) are blocked except
  `Esc`, so the app can't exit mid-write and leave a partial file. Verified against the real
  compiled binary: a 4000-file copy cancelled after ~1 file (proving the main loop stayed
  responsive during background I/O), the same copy run to completion (all 4000 landed), and a
  4000-file delete via the background path.
- ✅ **Interactive shell overlay** – `s` suspends the TUI (raw mode + alternate screen) and
  spawns `$SHELL` (falling back to `/bin/sh`) as a real, fully interactive child process
  inheriting stdio directly, with its cwd set to the browser's current directory; the user
  controls when it closes (`exit`, not Ranger's auto-close-after-one-command). New `shell_overlay`
  crate owns spawning; `tui` owns suspending/resuming raw mode and forcing a full redraw
  afterward via `Terminal::resize` (not `Terminal::clear`, which depends on a cursor-position
  query that can hang on some terminals — found via the PTY verification below, before it could
  ever hit a real user). Verified against the compiled binary via a scripted PTY session: typed
  real shell commands (`touch`, `pwd`, `exit`), confirmed the marker file and captured `pwd`
  landed in the browsed directory, and confirmed the TUI was still fully interactive afterward
  (navigated and quit normally).
- ✅ **Full theme system** – `theming::Theme` grew from 2 fields to 7 (`selection_bg`/`fg`,
  `border_fg`, `title_fg`, `dir_fg`, `file_fg`, `status_fg`), applied to every pane's border,
  title, per-entry directory/file color, and the status bar. `[theme] name = "..."` selects a
  built-in base palette (`"default"` or `"dracula"`, unknown names fall back to `"default"`
  rather than failing config load); any individually specified color field still overrides that
  palette's value, layered the same way `RawKeyMap` layers keybinding overrides. Verified against
  the real compiled binary: ran with the default theme and with `name = "dracula"` via a scripted
  PTY session, decoded the actual ANSI color codes emitted, and confirmed every themed element
  (border, title/dir color, selection highlight, status bar) genuinely changed between the two —
  not just that the config parsed.
- ✅ **Inline image preview** – the preview pane renders the selected image via `ratatui-image`
  (Kitty/iTerm2/Sixel, falling back to Unicode halfblocks when unsupported), extension-whitelisted
  (`png`/`jpg`/`jpeg`/`gif`/`bmp`/`ico`/`tiff`/`tif`/`webp`) via the new `preview` crate. Decoding
  and resize/encoding both run on `tokio::runtime::Handle::spawn_blocking` via `ratatui-image`'s
  `ThreadProtocol` (new `tui::image_preview` module) — the library's own docs say its adaptive
  widget "will block the UI thread" without this, so it gets the same never-block-the-render-loop
  treatment as bulk file ops. A failed/corrupt decode falls back to a "preview failed" message
  rather than an error. Verified against the real compiled binary via a scripted PTY session: a
  real 4×4 PNG rendered as genuine halfblock output with its actual pixel color present in the
  emitted truecolor escape codes, a mislabeled non-image file correctly showed "preview failed,"
  and navigation kept working throughout (proving the async pipeline never blocked input). This
  verification also surfaced a real (if narrow) finding, written up in `DIARY.md`: the crate's
  `Picker::from_query_stdio()` terminal-capability probe leaves its stdin-reading thread running
  if the terminal never answers *any* escape query — real terminals all answer instantly, so this
  isn't a practical concern for actual users, but it fully explained an early false negative
  during this cycle's own testing.
- ✅ **Command/search bar** – a Ranger/lf-style `:`/`/` bottom input line, reusing the existing
  `tui::app::Prompt` machinery (new `SearchInput`/`CommandInput` variants) rather than a new
  state machine. `/` opens incremental filename search: the selection jumps to the first
  case-insensitive substring match as you type (`browser::BrowserState::find_match`), and `Esc`
  restores the original selection (`select_index`). `:` opens a command prompt supporting
  `:q`/`:quit` (exit) and `:cd <path>` (jump to an arbitrary directory via the new
  `BrowserState::goto`, resolving a relative path against the current directory).
  `handle_prompt_key` now returns `ControlFlow<()>` so a quit command can signal the app to exit
  without `main.rs`'s event loop needing to know about `:`-commands directly. Verified against
  the real compiled binary via a scripted PTY session: typing `/bravo` jumped the selection and
  showed the live search buffer, `:cd alpha` navigated into that subdirectory (confirmed via the
  pane's title showing the real resolved path), and `:q` quit the app on its own. Getting that PTY
  verification right surfaced two harness bugs worth remembering for next time (written up in
  `DIARY.md`): a pty needs an explicit `ioctl(TIOCSWINSZ)` or ratatui lays out every pane as a
  zero-area rect, and a raw keystroke-by-keystroke capture never contains typed text as one
  contiguous string since ratatui only rewrites changed cells — a forced full redraw (resize +
  `SIGWINCH`) is needed before asserting on rendered text.
- ✅ **Text file preview** – the preview pane now renders a selected code/text file's actual
  contents (word-wrapped) instead of just its name, closing out the "text + image preview" scope
  the `preview` crate's module doc has called out since v0.1.0. New `preview::is_text`/`load_text`
  mirror `is_image`/`load_image`'s shape: an extension whitelist (plus a filename whitelist for
  extensionless files like `Makefile`/`.gitignore`) decides eligibility, and `load_text` returns
  `None` — falling back to "preview failed" — for a file over a 1 MiB cap, containing a null byte,
  or not valid UTF-8, so a binary file mislabeled with a text-like name degrades the same way a
  corrupt image does. New `tui::text_preview` module reads the file on
  `tokio::runtime::Handle::spawn_blocking`, the same never-block-the-render-loop treatment
  `image_preview` gives image decoding; `ImagePreview` and `TextPreview` are now constructed and
  driven together via a new `Previews` struct so `run`/`draw` don't grow an unbounded argument
  list as more preview pipelines are added. Verified against the real compiled binary via a
  scripted PTY session (using `pyte` to reconstruct the actual rendered screen rather than
  pattern-matching the raw diffed escape-code stream, which splits a single line of text across
  several writes and made naive substring checks unreliable): a real text file showed its actual
  source content in the preview pane, a `.rs` file containing binary bytes showed "preview
  failed," a file over the 1 MiB cap showed "preview failed," a corrupted `.png` still hit the
  existing image-decode-failure path rather than the new text path, and navigation (including
  back onto a previously-previewed file) and `q`-to-quit kept working throughout. This surfaced a
  harness-only finding written up in `DIARY.md`: `ratatui-image`'s terminal-capability probe
  leaves a background thread reading stdin forever if nothing ever answers its query, which on
  this project's synthetic (non-responding) PTY test harness silently swallowed every keystroke
  sent after startup until the harness was fixed to answer the probe itself — not a bug in
  minuteman.
- ✅ **Shell overlay as an embedded popup terminal emulator** – picked up the parked COA A from
  the previous cycle: `s` now opens a bordered, centered popup (80%×70% of the frame) instead of
  suspending to a full-screen shell. New `shell_overlay::PopupShell` spawns `$SHELL` on its own
  `portable-pty` pty and parses its output into a `vt100::Parser` screen buffer on a background
  reader thread; a new `tui::popup_shell` module owns everything crossterm/ratatui-specific —
  sizing the popup, encoding `KeyEvent`s into the raw escape sequences a real terminal would send
  (arrows, function keys, `Ctrl`/`Alt` combos), and rendering each `vt100::Cell` (color/bold/
  italic/underline/inverse) as a styled `Span`. `main.rs`'s loop forwards every keystroke straight
  to the pty while the popup is open, resizes it on `Event::Resize`, and polls `try_wait`
  non-blockingly each tick to notice the child exiting — the old full-screen `spawn_shell` path
  (raw-mode suspend/resume around inherited stdio) stays in `shell_overlay` unchanged and
  untouched, just no longer wired to any keybinding. Verified against the real compiled binary via
  a scripted PTY session reconstructed through `pyte` (this project's established fix for
  `ratatui-image`'s terminal-probe thread otherwise swallowing keystrokes, documented in the Image
  Preview Concurrency `DIARY.md` entry): pressing `s` showed a genuinely bordered "shell" popup
  with the rest of the browser's panes still visible around it (not a full-screen takeover), a
  real `echo` command's output appeared inside the popup, `Ctrl-C` interrupted a foregrounded
  `sleep 20` almost immediately (proving control-code encoding works, not just plain characters),
  resizing the pty mid-session (`TIOCSWINSZ` + `SIGWINCH`) kept the shell fully responsive
  afterward, and `exit` closed the popup and restored the exact underlying UI with a "shell
  exited" status message.
- ✅ **`Esc` closes the popup shell** – requested right after the popup shell shipped: `Esc` is
  now intercepted before it ever reaches the pty, killing the child (`PopupShell::close`, which
  calls `kill()` then `wait()` so it's reaped immediately rather than left a zombie) and closing
  the popup with a "shell closed" status, instead of forwarding it as input. This is a deliberate
  trade-off against the "vim/less/ssh all still work inside it" full-interactivity goal from the
  original popup design: a program like `vim` running inside the popup that uses `Esc` for its own
  purposes (leaving insert mode) will now have the popup close out from under it instead — see
  `DIARY.md`. Verified against the real compiled binary via a scripted PTY session: `Esc` closed a
  popup with a real shell prompt still active inside it, the underlying UI was restored exactly as
  before opening it, and the status line read "shell closed".
- ✅ **Ranger-style marks (`v`) + inert leader key (`space`) + shipped config example** – `v` now
  toggles a mark on the current entry (`BrowserState`'s new `HashSet<PathBuf>`, persisting across
  navigation, rendered with a `* ` prefix); `d` (delete) acts on every marked entry when any are
  marked, falling back to the single cursor selection otherwise (`Prompt::ConfirmDelete` and
  `spawn_delete` now take `Vec<PathBuf>`, deleting each in order and stopping at the first
  failure), and `BrowserState::prune_marks` drops any mark left pointing at a now-deleted path.
  `space` is a new inert `Action::Leader` — reserved for future chorded commands, dispatched
  through the same flat keymap match as every other key so it never captures or blocks input.
  New `config.example.toml` at the repo root documents every `[keys]`/`[theme]` field with its
  default, to copy to `~/.config/minuteman/config.toml`; a new test parses it against
  `RawConfig::default()` so it can't silently go stale. Verified against the real compiled binary
  via a scripted PTY session: `v` on two files showed both with a `* ` prefix, `d` prompted
  "delete 2 marked items permanently?" rather than the single-file wording, `y` deleted exactly
  those two files while an unmarked file and a directory in the same listing were untouched, a
  `space` keypress had no visible effect and didn't block the movement/mark/delete keys sent right
  after it, and `q` still quit the app cleanly afterward.
- ✅ **Movable, detachable popup shell** – the popup shell can now be repositioned and no longer
  monopolizes every keystroke while it's open. New `Action::ShellFocus` (default `tab`) toggles
  whether keys go to the shell or drive the browser underneath it — the popup stays open and
  visible either way — and `Action::ShellMove` (default `g`, only reachable while the popup is
  open and unfocused) enters a move mode where `hjkl`/arrows nudge its position and `Enter`/`Esc`
  confirms. `popup_shell::popup_area` now takes an `(i32, i32)` offset from center, clamped so the
  popup can never be nudged off-screen; the offset resets to centered on every new `s` spawn.
  Verified against the real compiled binary via a scripted PTY session reconstructed through
  `pyte` (this cycle also needed stripping the `ratatui-image` Kitty-graphics startup query from
  the raw stream before feeding `pyte`, which has no APC handler — see `DIARY.md`): `tab` unfocused
  the shell and the browser's file list stayed visible and responsive to navigation while the
  popup itself stayed open and rendered; `g` plus `hjkl` moved the popup measurably to the right on
  screen; `tab` refocused it and `Esc` closed it, restoring the exact underlying UI.
- ✅ **Tmux-style split-pane shells, with mouse support** – replaced the single floating
  `Option<PopupShell>` with `shell_layout::ShellPanes`, a binary split tree (`Leaf(PopupShell) |
  Split{direction, ratio, first, second}`) tiling the frame's browser area, so multiple shells can
  be open and visible side by side (or stacked) instead of one popup at a time. `%`/`"` split the
  focused pane horizontally/vertically (tmux's own default bindings) and focus the new pane; `o`
  cycles keyboard focus between panes; `Esc` closes just the focused pane, promoting its sibling
  to fill the freed space, and only exits shell mode entirely once it's the last pane. Mouse
  support is new (`EnableMouseCapture` is now on for the session): dragging the shared border
  between two sibling panes live-resizes their `ratio`, and clicking inside a pane focuses it.
  Deliberately **not** in scope: drag-to-reposition (meaningless once panes are tiled — a pane's
  rect is derived from the tree, not an independent offset) and drag-to-reorder/swap panes (real
  tree-surgery complexity for a rarely-used interaction even in mature tiling multiplexers). The
  tree/geometry logic is generic over the leaf payload specifically so it could be unit-tested
  (tree shape, divider hit-testing, focus routing) without spawning real shells — see `DIARY.md`
  for the design decision, the two rejected layout alternatives, and a real ownership bug the
  generic tests caught before it reached the compiled binary. Supersedes the older, vaguer
  "Multi-tab / multi-pane workspaces" idea previously listed under Low Priority. Verified against
  the real compiled binary via a scripted PTY session: `s` then `%` produced two independently
  interactive side-by-side panes (marker commands typed into each landed only on their own side);
  `o` and a real mouse click each correctly moved keyboard focus between panes; a real mouse
  divider-drag measurably moved the pane boundary; `Esc` closed the focused pane and resized the
  remaining one to fill the freed width, and a second `Esc` closed the last pane entirely with the
  browser still responsive afterward.
- ✅ **Shell panes contained to a mini floating box, not the full screen** – `shell_area` no
  longer returns the entire browser region; it now returns an 80%-width/70%-height box centered
  within it (the same sizing the original single popup used), so the browser stays visible around
  the shell panes instead of them tiling across the whole screen. Splits/tiling still work exactly
  as before, just confined to that smaller box. `draw` now calls `shell_area` directly instead of
  duplicating the layout math, so rendering and pty sizing can never drift apart. Verified against
  the real compiled binary: the same split/focus/divider-drag/close PTY session as above, with the
  shell box now visibly starting a few rows/columns in from the frame's edge instead of at (0, 0).
- ✅ **Mini-shell box movable by mouse, like a floating window's title bar** – the whole shell box
  (not individual panes, which stay tiled) can now be dragged around the screen. `shell_area`
  gained a `(dx, dy)` offset from centered, applied then clamped so the box can never be dragged
  off screen; a mouse-down on the box's top border (where the "shell" title renders) starts the
  drag, `Drag` accumulates the delta, `Up` ends it, and every fresh `s` spawn resets the offset to
  centered. New `ShellView` struct bundles the pane tree with its offset so `draw`'s argument list
  doesn't grow with every new piece of shell-overlay state. Verified against the real compiled
  binary via a scripted PTY session (real SGR mouse escape sequences, not just unit tests):
  grabbing the title bar and dragging moved the box by exactly the dragged delta, an extreme
  off-screen drag clamped the box to the frame's edge instead of vanishing or panicking, and the
  browser underneath stayed visible and `Esc`/`q` still closed/quit cleanly afterward.
- ✅ **Batch file operations — multi-select for yank/cut/paste** – `Clipboard.path: PathBuf`
  became `Clipboard.paths: Vec<PathBuf>`; `yank`/`cut` now snapshot every currently marked path
  (falling back to the single cursor entry when nothing's marked), the same
  marks-win-over-cursor convention `begin_delete` already established, via a new shared
  `App::marked_or_selected` helper. `spawn_paste` became `spawn_paste_item(clip, dst_dir, index,
  policy)`, pasting one item of the batch at a time; `poll_bulk`'s completion handling grew a
  continuation step — on `Outcome::Completed` or `Outcome::Skipped`, if items remain it
  re-spawns the next one directly instead of ending the operation, so an N-item batch stays one
  continuous "busy" operation from the UI's perspective rather than N separate ones. A conflict
  still pauses the whole batch on the existing overwrite/skip/abort prompt — `o`/`s` resume the
  batch afterward (retrying the conflicting item or moving past it), closing out the exact gap
  called out in the previous cycle's roadmap note: `file_ops::ConflictPolicy::Skip` "continue
  past one conflicting item instead of stopping" was implemented but unobservable in the TUI
  since there was never more than one item in flight. The status line grew a `[i/N]` hint for
  batches of more than one item. Verified against the real compiled binary via a scripted PTY
  session driving actual marks/yank/cut/paste keystrokes (answering `ratatui-image`'s startup
  terminal-capability probe first — the same synthetic-PTY gap documented in the Image Preview
  Concurrency `DIARY.md` entry, which otherwise silently swallows every keystroke sent
  afterward): marking a directory plus two files and yanking, then pasting into an empty
  directory, landed all three with correct contents including the recursed subdirectory; cutting
  two files into a directory where one name already existed moved the first one, correctly
  paused on the real conflict prompt for the second, and — after pressing `s` — left the
  conflicting source file physically untouched (not moved, destination not overwritten) while
  still finishing the batch and clearing marks/clipboard state correctly at the end.
- ✅ **Keyboard resize/move for shell panes, i3-style** – `space` (`Action::Leader`, previously
  inert) now starts a chord whenever a shell pane is open: `r` enters a resize mode where
  `hjkl`/arrows nudge the focused pane's nearest ancestor divider by 5% per press
  (`shell_layout::ShellPanes::resize_focused`, walking up the tree for the first `Split` whose
  axis matches the pressed direction and growing/shrinking the focused pane's share regardless of
  which side of that split it's actually on), and `m` enters a move mode where the same keys nudge
  the whole box's offset by 2 cells per press — the keyboard equivalent of the existing
  mouse-drag-the-divider and drag-the-title-bar interactions, not a new capability. When there's
  no divider along the pressed axis to adjust — a lone pane, or a tree only ever split the other
  way, e.g. only stacked (`"`) panes when `h`/`l` is pressed — resize mode falls back to growing or
  shrinking the box's own width/height instead (`resize_focused` now returns whether it actually
  found a divider, so `apply_shell_chord` knows when to fall back), via a new `size_adjust: (i32,
  i32)` parameter on `shell_area` alongside its existing `offset`. `Esc` leaves either mode; any
  other key leaves it too but still gets dispatched normally afterward (e.g. `q` still quits from
  inside the chord) rather than being silently swallowed. Deliberately **not** a modifier chord
  (e.g. `Alt+hjkl`): `KeyMap::resolve` has no modifier awareness at all today, and Alt-prefixed
  hjkl is real, commonly-configured `tmux`/`vim-tmux-navigator` pane-navigation input that a shell
  running *inside* one of these panes could legitimately be using — this needed to cost nothing
  from the pty's own keyspace, which a `space`-prefixed chord dispatched entirely at the app layer
  accomplishes for free. The chord's own `r`/`m`/`hjkl` keys are hardcoded rather than
  user-configurable, the same precedent already set by the shell-pane `Esc` handling. Verified
  against the real compiled binary via scripted PTY sessions (reconstructed through `pyte`):
  confirmed a space typed while a real shell had focus still landed in it unchanged (proving the
  chord can't steal input from a focused pty), `space r` plus three `l` presses moved the shared
  divider between two horizontally split panes measurably left without moving the box itself,
  `space m` plus `lll`/`jj` moved the whole box by exactly 6 columns and 4 rows (3×2 and 2×2, the
  configured step), pressing `q` while still inside an active chord (no `Esc` first) both left the
  mode and quit the app, and — separately — a lone unsplit pane and a vertically-split (`"`) pane
  each grew the box's own width by exactly 6 columns (3×2) on `space r` plus three `l` presses,
  confirming the fallback triggers exactly when there's no horizontal divider to adjust instead.
- ✅ **Mouse box-width resize + keyboard split-orientation toggle** – two follow-ups to the
  resize/move chord above, both requested directly from using it. First, the box's own right
  border (distinct from any internal pane divider) is now mouse-draggable, resizing its width the
  same way the resize chord's fallback already does — `shell_area`'s `size_adjust.0` tracks the
  drag, with a mouse-column delta of `d` mapping to a `size_adjust` delta of `2d` since the box
  grows symmetrically from its centered position (the dragged edge only tracks the mouse 1:1 if
  the *opposite* edge also moves by the same amount to stay centered). The right-border hit-test
  is checked before the existing title-bar hit-test, so the top-right corner (where both would
  otherwise match) prefers resize over move. Second, `space t` — a third, one-shot (not
  repeatable, unlike `r`/`m`) branch of the existing leader chord — flips the orientation
  (side-by-side <-> stacked) of the split the focused pane is immediately part of, via new
  `shell_layout::ShellPanes::toggle_focused_orientation`, which mutates a `Split` node's
  `direction` in place without touching which panes are on which side or their ratio — still not
  the pane-reordering this project has already deliberately ruled out (see the Multi-Shell
  Layout Model entry), just how the same two panes are arranged. Deliberately **not** a bare `t`
  keybinding intercepted while the shell is focused: that would silently steal an extremely
  common shell-command letter (`touch`, `top`, `tar`, `test`, `git`, …) from real typing, the
  exact category of problem the leader chord was already built to avoid for resize/move — see
  `DIARY.md` for a design decision this cycle caught and reverted before it shipped. Verified
  against the real compiled binary via scripted PTY sessions: dragging the box's right border by
  5 columns grew its actual width by 10 (2× the drag delta, matching the symmetric-growth model);
  `space t` on a side-by-side split removed the internal vertical divider entirely (confirming a
  stacked layout) and a second `space t` restored it; and `toggle_focused_orientation` on a lone
  pane correctly reports nothing to toggle rather than panicking.
- ✅ **Quit into the current directory (`Q`), ranger-style** – capital `Q` quits and leaves
  the parent shell in the directory being browsed; lowercase `q` still quits in place. A process
  can't change its parent's working directory, so the feature is two halves: `Q`
  (`Action::QuitToCwd`) writes the directory to the file given by the new `--cwd-file` option,
  and `minuteman init <bash|zsh|fish>` prints an `mm` wrapper function that passes the flag and
  `cd`s afterward (one `eval` line in the rc file). Single-character config keys became
  case-sensitive so `"Q"` doesn't collapse into `"q"`. Verified with a real zsh in a PTY: after
  three keypresses down into `root/alpha/beta`, `Q` left the shell in `root/alpha/beta` and `q`
  left it in `root`.

---

## 🔥 High Priority (Critical)

- **Richer status line** – expand the bottom status bar beyond the current prompt/progress text
  to surface per-selection info at a glance: file count, cumulative size, permissions, and (where
  applicable) git status.
- **Bookmarks / marks** – jump-to-directory bookmarks (Ranger-style `` ` ``/`m` register) so
  frequently visited paths don't require re-navigating the miller columns each time.

---

## 🟡 Medium Priority (Important)

- **Built-in trash + undo history** – safe delete-to-trash and an undo stack for recent file
  operations, with no plugin required.
- **VFS abstraction hardening** – a `Filesystem`/`Vfs` trait consumed uniformly by browser,
  file_ops, preview, and trash, so backends can be swapped without touching feature code.

---

## 🟢 Low Priority (Nice-to-Have)

- **Native remote filesystem browsing (SSH/SFTP)** – browse and operate on `ssh://`/`sftp://`
  paths directly through the VFS abstraction, no FUSE mount required.
- **WASM plugin host (Extism)** – expose a versioned host API (`read_dir`, `get_selection`,
  `spawn_preview`, keybind registration, lifecycle hooks) so plugins can be written in Rust, Go,
  Python, JS, etc., sandboxed by default.
- **Optional Lua scripting tier** – lightweight `mlua`-based scripting for config/keybindings/
  simple commands, layered alongside the WASM plugin system.
- **Fuzzy find / search within the browser.**
- **Byte-level/percentage progress for large single files** – current progress is one tick per
  *file*, so a single huge file shows no movement until it's done. Needs `Vfs::copy_file` to
  support a streaming copy with periodic callbacks instead of one atomic `std::fs::copy` call.

---

## 🌌 Long-Term Vision

- **Stable, semver'd plugin API commitment** – additive-only changes and a documented
  deprecation policy from v1 onward, positioning Minuteman as the dependable "go-to" daily
  driver where competing tools are still pre-1.0 and shipping breaking plugin-API changes.
- **Broader remote backend support** – extend the VFS abstraction beyond SSH/SFTP to backends
  like S3 or rclone-backed remotes.
- **Community plugin ecosystem** – a registry/index of Extism-based plugins across languages,
  sandboxed by default.

---

## 🎯 Next Actions (Immediate)

0. Add `eval "$(minuteman init zsh)"` to `~/.zshrc`, reopen the terminal, run `mm`, navigate
   somewhere, and press `Q` — your shell should follow; `q` should not.
1. `cargo run -p tui` — press `s` to open a shell, `%` to split it. Drag its own right border
   with the mouse to resize the box's width directly. `tab` to unfocus, then `space t` to flip
   the split to stacked and back, or `space r`/`space m` plus `hjkl` for the keyboard resize/move
   chord (`Esc` leaves either mode).
2. `cargo test --workspace` (all tests) to verify everything still passes.
3. Commit this cycle (step 8 of the dev loop).
4. Pick the next roadmap item from 🔥 High Priority: richer status line or bookmarks/marks
   (directory bookmarks — distinct from the file marks added earlier) are the remaining
   candidates.
