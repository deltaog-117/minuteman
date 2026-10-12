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
- ✅ **Single-leader mini-shell keys** – replaced the `Tab` focus toggle and the `%`/`"`/`o`
  pane keys with one leader, `space`, for everything: `space space` (back to typing),
  `space h`/`j`/`k`/`l` (directional pane focus, via new `shell_layout::neighbor`), `space |`/`-`
  (split), `space x` (close), plus the existing `r`/`m`/`t`. Typing mode now forwards every key to
  the shell except `Esc`, which leaves it. This also fixes a bug where `Tab` (completion), `o`, `%`
  and `"` couldn't be typed into a focused shell. Verified with a real bash in a PTY read through
  a terminal emulator: those keys and multi-space input reached the shell, and `space |`,
  `space h`, `space space` and `space x` each did what they say.
- ✅ **Neon theme engine (UI overhaul, phase A)** – the default look is now a cyberpunk
  true-color palette: rounded frames, an active pane that glows cyan against dim indigo
  neighbours, a magenta stripe on the selected row, and per-kind entry colors. `[theme]` gained
  hex colors (drawn as truecolor, or quantized to 256 colors when `COLORTERM` doesn't advertise
  it), seven new color fields, `border_type`, and `selection_fg = "keep"`; the old look survives as
  `name = "classic"`. Verified against the real binary with a terminal emulator reading per-cell
  colors, with and without `COLORTERM=truecolor`. Phases B (HUD layout: header breadcrumb,
  powerline status bar, size/date columns, scrollbars, optional Nerd Font icons) and C
  (animation and effects) are queued below.
- ✅ **HUD layout (UI overhaul, phase B)** – a header row with a breadcrumb path (`~` for home,
  the middle elided with `…` when narrow) and pills for marks, the clipboard (`⧉ 1 yanked`,
  `✂ 3 cut`) and a background-job gauge; right-aligned size and relative-age columns that drop
  age, then size, on narrow panes and never squeeze a name below 12 cells; a scrollbar drawn over
  the current pane's right border when the list overflows; and a powerline-style status bar: a
  colored mode pill (NORMAL, SHELL, LEADER, RESIZE, MOVE, BUSY, or the prompt's own), then the
  selection's name, permissions, size (or item count for a directory) and type, the position
  `12/48`, and key hints built from *your* bindings that change with the mode. This replaces the
  ad-hoc status strings for leader/resize/typing states, and status messages now clear themselves
  after five seconds. `DirEntryInfo` gained `size`, `modified` and `mode`, read from the same
  `stat` the listing already did. `[theme] separator = "arrow"` switches the segment edges to
  Powerline arrows (needs a Powerline/Nerd Font; the default is flat). Verified against the real
  binary through a terminal emulator at 120 and 60 columns, in every mode. Directories show `—`
  in the size column and their item count in the status bar once selected, rather than counting
  every directory's entries on every listing.
- ✅ **Glyph sets, Nerd Font icons, and terminal snippets (UI overhaul, typography)** – the
  font is the terminal's, so the UI now matches what the font can draw. `[ui] glyphs` selects
  `unicode` (default), `nerd` or `ascii`: `nerd` adds file-type icons in each kind's color and
  real Powerline arrows (`[theme] separator` defaults to `auto`, meaning arrows exactly when
  the set is `nerd`); `ascii` draws every frame, stripe, scrollbar, gauge and separator with
  plain ASCII. Every hardcoded symbol in the header, list, scrollbar and status bar moved into
  one `Glyphs` table. `minuteman glyphs` prints all three sets to see what your font shows, and
  `minuteman init-terminal <kitty|alacritty|wezterm>` prints a matching font (JetBrainsMono Nerd
  Font Mono) and neon 16-color palette, with a Symbols Nerd Font fallback for kitty. Verified
  against the real binary through a terminal emulator: the Nerd set showed icons, arrows and pill
  symbols with columns still aligned, and the ASCII set put no non-ASCII character anywhere on
  screen; the alacritty snippet parses as TOML and the wezterm one as Lua.
- ✅ **Appearance file with text styles and font (UI overhaul, typography)** – all of the look
  now lives in `~/.config/minuteman/appearance.toml`: `[theme]` colors, `[ui]` glyphs, a new
  `[style]` table giving each of 22 elements a list of bold/italic/dim/underline/reverse/
  strikethrough, and a `[font]` table. Prompted by Ranger's bold directories: Minuteman's were
  plain, and terminals brighten bold only for the basic 16 colors, so hex colors needed the
  attribute set explicitly — directories and executables are now bold by default, and every other
  hardcoded bold became a `[style]` default. The font can't be applied by the TUI (the terminal
  owns it), so `[font]` feeds `minuteman init-terminal`. `minuteman init-appearance` prints the
  commented default; a `[theme]`/`[ui]` still in `config.toml` keeps working, with the
  appearance file winning per field. Verified against the real binary: attributes read off the
  screen for the defaults, a custom file, and legacy layering; a malformed file falls back.
- ✅ **`Alt` layer for the mini-shell box** – holding `Alt` now drives the box directly, in every
  mode, without touching the `space` leader chord (which still owns the explicit `|`/`-` splits,
  flipping a split, and the divider-resize and `r`/`m` modes). `Alt+hjkl` moves the box, `Alt+a`/`Alt+d` grow its
  left/top edge outward and `Alt+f`/`Alt+s` shrink it horizontally/vertically (top-left fixed), `Alt+zxcv` focuses the pane on that
  side, `Alt+n` splits the focused pane (picking side-by-side or stacked from its shape,
  and opening the first shell when none is open), `Alt+t`/`Alt+b` snap the box to the top or
  bottom centre, `Alt+m` closes the pane under the pointer (the focused one if the pointer isn't
  over any) and `Alt+q` closes every shell. With the mouse, `Alt`+left-drag grabs the box
  anywhere and `Alt`+right-drag resizes it from the bottom-right corner (growing or shrinking).
  Chosen as COA A — the existing single box with tiled panes — over independent floating
  windows, so "the shell" in a move or resize is the whole box, not one pane. **Reverses** the
  earlier decision not to use `Alt`-prefixed keys (see the chord entry above): a shell in the box
  no longer receives `Alt+b/f/d/t` (readline word motions) or `Alt+hjkl` (tmux navigation), by
  request. Split started as `Alt+Shift+S` (`Alt+s` was taken by resize) and was then moved to
  `Alt+n`, with close-pane moving from `Alt+e` to `Alt+m`. New pure `tui::alt_keys` module parses the keys; new `shell_params_for` inverts
  `shell_area`, which is what makes one-sided edge growth and exact snapping possible;
  `ShellPanes` gained `focused_rect` and `close_all` (a dropped `ShellPanes` would leave its
  child shells running). `Alt+q` used to be a plain `q` and quit from browse mode; it no longer
  does. Verified against the real binary through a PTY read with `pyte`: every key above moved,
  grew, focused, split, snapped or closed exactly as described, plain letters still reached a
  focused shell.
- ✅ **Tapping `Alt` alone switches between the mini-shell and the file browser** – pressing and
  releasing `Alt` with nothing in between toggles whether keys go to the shell or to the
  browser, the same switch `Esc` and `space space` already make, but in both directions from
  one key. A terminal sends nothing at all for a modifier on its own, so this uses the kitty
  keyboard protocol: at startup Minuteman asks the terminal whether it supports it and, if so,
  turns it on (and off again on exit). A tap is an `Alt` release that follows an `Alt` press
  with no other key or mouse press between, so `Alt+n`, an `Alt`-drag, or `Alt` then `x` never
  toggle. Turning the protocol on makes the terminal report every key as an escape code, so key
  handling now treats a held key's repeat events as typing and ignores releases, and requests
  alternate keys so `Shift`+letter still arrives as a capital. The same change moved the
  new-shell key to `Alt+n` and close-pane to `Alt+m`, retiring `Alt+Shift+S` and `Alt+e`. New
  top-level `alt_tap` option in `config.toml` (default `true`) turns the protocol off, because
  it can plausibly break composed characters (dead keys, `AltGr`) typed into a shell. On a
  terminal without the protocol nothing changes and the tap does nothing. Verified against the
  real binary on a PTY that answered the protocol query like kitty and sent real protocol
  events; it could not be tried in a real kitty.
- ✅ **Mouse support in the file browser** – the three columns now answer the mouse. A click on a
  row in the middle column selects it; a double-click on a directory opens it (on a file it only
  selects, since there is nowhere to send a file to be opened yet — see *Open-with* below). A
  click on a row in the left column goes up a directory and selects that entry, and a
  double-click there is ignored, because the columns have already shifted under the first click.
  The wheel over the left or middle column moves the selection three entries at a time; over the
  preview column it does nothing yet. Clicking anywhere in the browser hands the keyboard back
  from a mini-shell, the same switch as tapping `Alt`. The shell box keeps priority over the
  pointer, and a drag that begins on it still finishes on it. Behind the scenes `draw` and the
  mouse handler now take their rectangles from one function (`browser_mouse::BrowserLayout`),
  and the middle column's scroll position is kept between frames so a click can be mapped back to
  the entry it hit. That changes keyboard scrolling slightly: the list only scrolls once the
  selection reaches an edge of the view, instead of following it from the top each frame. A new
  top-level `browser_mouse` option in `config.toml` (default `true`) turns the browser's mouse
  off without affecting the shell box. Hit-testing, the double-click clock and the wheel step are
  pure code with property tests. Verified against the real binary on a PTY through `pyte`; not
  tried with a real mouse in a real terminal.
- ✅ **`mman`, the command that launches Minuteman** – the project keeps its name, but the
  executable Cargo builds and installs is now `mman`, so `cargo install --path crates/tui` puts
  one short command on the user's `PATH`. The `init` shell wrapper is a function of the same name,
  so there is one command to remember: run `mman`, and `Q` leaves the shell in the directory being
  browsed. The wrapper reaches the real binary through `command mman`. Nothing else was renamed:
  the config folder is still `~/.config/minuteman/`, and code comments that mention Minuteman by
  name were left as they were. The old `minuteman` executable and the `mm` wrapper no longer exist.
  Verified in a real zsh with only `mman` on `PATH`: `Q` moved the shell into the browsed
  directory and `q` did not.
- ✅ **Hidden-file toggle, `:` commands and live refresh** – `.` shows or hides dot-files in every
  column (hidden by default; `show_hidden` in `config.toml` sets the starting state), filtered
  where a listing is stored so the cursor, `/` search and mouse clicks all index the list on
  screen. The `:` prompt now runs `mkdir [-p]` and `touch` in-process through `Vfs` and hands
  everything else to `sh -c` in the browsed directory on the blocking pool, with output on the
  status line and `Esc` to kill it. The lists refresh on their own: about twice a second the
  directory is re-listed off-thread and compared with the screen, which catches creates,
  removals, renames and in-place writes (a directory's own mtime would miss the last), and
  re-reads the selected file's preview when it changed. No new dependency. Verified against the
  real binary on a PTY through `pyte`: toggling, both built-ins and the shell path, `Esc`
  killing `sleep 30`, files created, edited and deleted by another program and from inside a
  mini-shell appearing untouched, and a new hidden file staying hidden until toggled.
- ✅ **`Esc` belongs to the mini-shell** – `Esc` used to leave typing mode, so a program inside a
  pane that needs it (`vim`, `fzf`) lost focus mid-use. It is now sent to the shell like any
  other key (`0x1b`). Typing mode is left by tapping `Alt`, clicking the browser, or closing the
  pane with `Alt+m`; `Esc` is unchanged in the browser's own prompts, busy state and resize/move
  chord. Docs, `config.example.toml` and the status bar hint updated to match.

- ✅ **`Q` under the keyboard protocol** – a terminal may report Shift+q as `q` plus the Shift
  flag, and the key lookup ignores modifiers, so `Q` quit without recording the directory. Shift
  plus a lowercase letter is now turned into the capital once, before any lookup. Reproduced on a
  PTY through the real `mman` wrapper in zsh: the shell stayed put with the shifted-`q` report and
  moved into the browsed directory after the fix, and with the plain and the alternate-key reports.

- ✅ **`Q` with Caps Lock on** – a probe in the real kitty showed Caps Lock arrives as a flag
  (modifier mask 65) with the letter still lowercase, so a capital typed with Caps Lock was read
  as `q`. Caps Lock now flips a letter's case after the `Alt` layer has run, so `Alt` commands are
  unaffected. Checked through the real `mman` wrapper in zsh with the exact sequences kitty sent:
  Caps Lock+`q` and Shift+`q` moved the shell, plain `q` and Caps Lock+Shift+`q` did not.

- ✅ **`:nvim ROADMAP.md` — a `:` command can take over the terminal** – a command whose program
  name is in the new `interactive_commands` list in `config.toml` (editors, pagers, `htop`, `mpv`,
  `ssh`, `tmux`, `fzf` by default), or any command prefixed with `!` (`:!python3`), now suspends
  the interface instead of running with its output captured. `App` only records the request
  (`Handover`); `main` owns the terminal, so `TerminalGuard::run_foreground` leaves the alternate
  screen, mouse capture, raw mode and the kitty keyboard flags, calls the new
  `shell_overlay::run_foreground` (`sh -c` with inherited stdio, in the browsed directory), then
  restores all four and forces a repaint with `Terminal::resize`, not `clear`, for the reason the
  post-shell-redraw decision in `DIARY.md` gives. The listing is re-read afterwards and the exit
  status shown. While the child runs, Minuteman swaps `SIGINT` for a do-nothing handler (a
  handler, not `SIG_IGN`, because only a handler resets to the default across `exec`), so
  `Ctrl-C` stops the program and not the browser. Chosen as COA A over running the command in a
  mini-shell pane and over auto-detecting full-screen programs; this is the terminal handover
  *Open-with* needs. Verified against the real binary on a PTY through `pyte`, with the real
  `nvim`: it showed the file, an `Esc` reached it, `:wq` saved to disk, and the browser came back
  repainted and responsive with `nvim ROADMAP.md: done`; `:!cat > file` received typed input
  and Ctrl-D; `Ctrl-C` killed `:!sleep` while Minuteman survived; and with the terminal
  answering the kitty protocol query the keyboard flags were popped before leaving the
  alternate screen and pushed again after re-entering it. Not tried in a real kitty.
- ✅ **`c` cancels everything pending** – a new `Action::Cancel` (`[keys] cancel`, default `c`)
  clears the yanked or cut clipboard, every `v` mark, and cancels a running copy, move or `:`
  command, all in one press and from any directory (the clipboard lives on `App`, not on a
  directory, so this needed no new state). It works in the busy state too. A running delete has
  no cancel hook and the status line says so. Chosen as COA B: cancel means everything. Doing this
  exposed a bug worth fixing: each item of a batch paste gets its own cancel flag, so a cancel
  landing as an item finished let the batch start the next one; `poll_bulk` now checks the
  finished item's flag. Verified against the real binary: two marked files cut and taken into
  another directory lost their pill and marks on `c`, the files were untouched on disk, `p` then
  reported an empty clipboard, and `c` killed a running `sleep` (checked with `pgrep`).
- ✅ **Arrow keys for browsing** – `down`/`up`/`right`/`left` are now key names the config
  accepts and are in the default bindings for move down, move up, enter and leave. Arrows still
  go to a focused mini-shell untouched. Verified against the real binary: down, up, right (into
  a directory), left (back up, cursor on the directory left) and `q`.
- ✅ **Smart `/` search** – `/aerend` typed in `~` now finds `~/Desktop/work/aerend`. The new
  `browser::search::find_below` walks breadth-first through `Vfs::list_dir` only, so the nearest
  match wins and a name in the current directory is found before anything deeper, and it takes a
  cancel flag and never opens or `stat`s a path itself, which is what a remote backend will
  need. `tui::search_job` runs it on the blocking pool, one job per keystroke, dropping (and so
  cancelling) the previous one. The hit's directory opens with the cursor on it
  (`BrowserState::reveal`); `Esc` or an emptied query returns to the directory and row the search
  started from; `Enter` keeps the cursor; `searching…` and `no match` show in front of the query.
  Bounded at 16 levels and 200,000 entries, hidden entries skipped unless shown. Chosen as COA A
  over a background index with fuzzy scoring and over shelling out to `fd`. Tested with a
  property test over random trees (the hit matches, nothing nearer matches, and "not found"
  means nothing does) on an in-memory `Vfs` that implements only listing. Verified against the
  real binary: the nearest of two same-named entries won, a deeper name was found, `Esc` and
  `Enter` behaved as above, and a missing name said `no match`.
- ✅ **Right-click context menu, Inspect panel, Open / Open with** – right-clicking a file or folder
  selects it and opens a menu at the pointer (Open, Open with ▸, Cut, Copy, Paste into folder,
  Rename, Delete, Mark, Copy path, Inspect); empty space opens a directory menu (New, Paste,
  Show/Hide hidden, Refresh, Copy path, Inspect this folder). `tui::context_menu` holds the
  contents, geometry and hit-testing as pure code (the `browser_mouse` pattern), so the rectangle
  `overlay_view` draws and the one a click is tested against are one function; every item calls
  the `App` or `BrowserState` method its key already calls. `tui::inspect` reads one `lstat` and,
  for a folder, counts what is below it on the blocking pool (cancelled when the panel closes,
  capped at 500,000 entries); owner, on-disk size and link count are local-only, as `Vfs` has no
  such fields. Open runs `xdg-open`; Open with reads `[[open_with]]` from `config.toml` (falling
  back to `$VISUAL`/`$EDITOR`), hands the terminal over for a program in `interactive_commands`
  and otherwise starts it detached (`shell_overlay::spawn_detached`). Double-clicking a file now
  opens it too. Copy path uses OSC 52. Chosen as COA B (menu with submenus and a modal Inspect
  panel inside `tui`, config-driven Open with) over a flat menu (A) and a multi-crate build with
  `.desktop` discovery, drag and drop and multi-select clicks (C), whose extras are queued
  below. Property-tested: shell quoting round-trips through a real `sh` for any file name,
  a menu and its submenu stay on screen for any pointer position, and no sequence of moves,
  clicks and keys can highlight a missing or disabled row. Verified against the real binary
  over a PTY with SGR mouse sequences: the file and directory menus opened, the submenu opened on
  hover, Open with ran a program on a file, Inspect showed a file and a folder, Delete reached
  its confirmation prompt, and Cut then Paste into folder moved a file on disk.
- ✅ **Richer status line — marked-set size and git status** – the two gaps left in the HUD.
  The header's `◆ 3 marked` pill now adds what the marks total (`◆ 3 marked │ 1.4 GiB`,
  `at least …` past the 500,000-entry walk limit): new `tui::marked_size` sums files by `lstat` and
  folders through the same `tally_dir` walk Inspect uses, on the blocking pool, restarting when the
  marks change and keeping the previous total on screen until the new one lands (no flicker); paths
  under a marked folder are skipped, so a folder and a marked child count once. The status bar
  gains a git segment on the right (`⎇ main ↑2 ↓1 +3 ~2 ?1`) and the selected entry's state on the
  left (`modified`, `staged`, `untracked`, `staged+modified`, `conflict`; a folder shows the merged
  state of what is inside). New `tui::git_status` runs `git --no-optional-locks status
  --porcelain=v2 --branch -z` on the blocking pool — no index lock, cancelled when the directory
  leaves the repository, killed after 10 s, re-run 3 s after the last run finished — with a pure
  parser (`parse`) over its bytes; whether a directory is in a repository is decided in-process by
  looking for `.git`, so browsing elsewhere spawns nothing. Chosen as COA A (shell out to `git`,
  no new dependency) over the `gix` crate (B) and marked-size only (C). Both segments drop first on
  a narrow bar; the ASCII glyph set draws them in ASCII. A top-level `git_status` option in
  `config.toml` (default `true`) is the off switch. Property-tested: the parser round-trips any
  ordinary records (names with spaces, counts add up) and never panics on arbitrary bytes, the
  state merge is commutative, associative and idempotent, and marked totals equal the sum of the
  file sizes whatever their order, nesting or repeats. Tests against the real `git` confirm a
  status run leaves `.git/index` untouched (with a control showing a plain `git status` does not).
  Verified against the real binary in a PTY read through `pyte`, in a scratch repository: the
  branch and `+1 ~2 ?1` appeared, each file showed its own state and a folder showed `modified`,
  a clean file showed the branch and no state word, marking one file then two changed the pill from
  `2.0K` to `2.2K`, a marked folder showed its 500 B, `c` cleared the pill, `:cd` out of the
  repository dropped the segment at once and back restored it, and with `git_status = false` no
  segment and no `git` process appeared.
- ✅ **Preview extras, stage 1 — scrolling, hex view, archive listing** – the preview column now
  scrolls (`J`/`K`, new `[keys] preview_down`/`preview_up`, half a screen each; the wheel over the
  column, three rows), and it shows something for every regular file. New `tui::preview_view` draws
  text, hex and archive content with a scrollbar; new `tui::text_preview::Scroll` holds the
  position (clamped when drawn, since the pane's height and the content's length are only known
  then; reset when the selection changes, kept when the file changes on disk). The `preview` crate
  gained `hex` (first 64 KiB, 16/8/4 bytes a row by pane width, formatted only for the rows on
  screen), `archive` (zip, tar and tar.gz listings) and `load`, which picks text, bytes or an
  archive by name and content. Chosen as COA A (in-process `zip` and `tar` crates, `flate2` for
  gzip) over shelling out to `bsdtar` (B) and hand-parsing the formats (C). Archives are untrusted
  input opened by moving the cursor over them, so each read is bounded: 5,000 entries, 256 MiB
  inflated (cut-off listings say so), plain tars skipped by seeking, zips with a central directory
  over 8 MiB or in zip64 refused before the crate allocates, and names cleaned of control and
  direction-changing characters. Text is now told from binary by content, so an extensionless text
  file shows as text and a `.txt` full of binary falls back to the hex view instead of `preview
  failed`. Property-tested: hex rows round-trip every byte at any width, archives of random names
  and sizes list back exactly in all three formats, the scroll offset is always within the content,
  the scrollbar thumb spans the bar, a cleaned name never holds an unsafe character, and no pane
  size or scroll position panics for any content. A test that made the zip guard's first version
  fail (it looked only at the last end-of-directory record, which a reader may skip) led to it
  checking all of them. A worst-case frame (1 MiB of text scrolled to the end) takes about 43 ms in
  release, inside the 100 ms tick (an ignored benchmark test). Verified against the real binary in
  a PTY read through `pyte`: `J`, `K` and the wheel scrolled a long text and stopped with the last
  line as the last row, a binary showed a hex dump and a 200 KiB one said `first 64K of 200K
  shown`, a zip and a tar.gz showed their summaries and entries, an archive holding an
  OSC-title escape in a name showed it defused and sent nothing to the terminal, a `.txt` of binary
  showed hex, a file with no extension showed text, and a named pipe showed only its name with the
  browser still responsive.
- ✅ **Disk usage view** – `u` (or "Disk usage" in the right-click menu of a folder or of empty
  space) opens a modal, full-screen view of what takes the space in the browsed folder, biggest
  first, with a size, a share and a bar per entry; `enter` opens a folder, `h` goes up (past the
  starting folder too, keeping the cursor on the one left), `a` switches between size on disk and
  apparent size, `r` rescans, `q`/`Esc` close it. New `tui::disk_usage` scans one folder at a time
  on the blocking pool (`scan_level`: files first, then each subfolder as its total is known),
  cancelled when the view moves on or closes, and keeps a stack of the levels on the way down, so
  going back is instant and memory never holds a whole tree; new `tui::disk_usage_view` draws it
  from pure layout, hit-test, scroll and bar functions. Chosen as COA A (a `du`-style modal view)
  over a recursive-size column in the browser (B) and an ncdu-style cached tree with delete
  built in (C). Sizes are allocated blocks by default (both sizes are recorded in one scan, so the
  switch is free), hard links count once, the scan stays on its filesystem and never follows a
  symlink, an unreadable subfolder marks its parent `!`, a folder's 20,000 biggest entries get
  rows and the rest fold into one, and at most 10,000,000 entries are looked at. Property-tested:
  folder totals equal the sum of the files below them on random trees, folding many files keeps
  every byte and every one of the largest, the cursor stays on its row while rows arrive and are
  re-sorted, the selected row is always on screen and the view scrolls only when it must, shares
  and bars stay in range, and a click maps to the row drawn at that spot. Tests also cover hard
  links, a symlink loop, another filesystem (by pretending the root is on a device nothing below it
  is on), a folder mode 000, cancellation, the entry limit and the ASCII glyph set. A warm scan of
  `/usr` (580,370 entries) takes 2.85 s against `du`'s 2.9 to 3.0 s (an ignored benchmark test; the
  first, cold run took 20 s, which is the disk). Verified against the real binary in a PTY read
  through `pyte`, in a folder with nested folders, a hard link, a sparse 10 MB file and a symlink
  to itself: the view opened with the biggest folder first, a hard-linked pair showed one size and
  one `0B`, the sparse file took almost nothing on disk and jumped to the top at `9.5M` after `a`,
  `enter` opened a folder and `h` came back with the cursor on it, `enter` on a file did nothing, a
  click selected a row, `r` rescanned, `q` closed the view without quitting, and the folder's
  right-click menu offered Disk usage and opened it.
- ✅ **Configurable panels and a live settings popup** – a new `[panels]` table in `config.toml`
  picks the miller-column layout (`columns = "three"`, the default parent | current | preview, or
  `"two"`, current | preview with the parent column removed and its width given to the rest) and
  whether the header row (`show_hud`) and the status bar's idle chrome (`show_command_bar`) are
  drawn; an unrecognised `columns` value falls back to `"three"`, like every other config field.
  `space` then `t` with no shell pane open (previously a no-op — `t` only means anything
  mid-leader-chord when a shell pane exists, to flip its orientation) opens a settings popup that
  cycles all of these plus the theme (`neon`/`classic`/`dracula`) live, for the running session;
  new `tui::settings_popup` is pure state, mirroring `context_menu`, and
  `overlay_view::render_settings` draws it. `BrowserLayout::split` now takes the column layout and
  the HUD flag: two-pane gives the parent column a zero-width `Rect` so it is skipped rather than
  drawn empty, and hiding the HUD reclaims its row's height; the status row is never reclaimed the
  same way and always renders in full during a prompt, a non-idle mode or a transient message, so
  hiding the command bar only ever suppresses its passive display. Chosen as COA C — a Rust-native,
  config-driven panel registry compiled in — over a declarative-only popup with no add/remove
  capability (A) and pulling the WASM/Extism plugin host (see Low Priority) forward into this
  cycle (B); C is a stepping stone toward B, not a replacement for it. Popup changes are
  session-only: there is no precedent anywhere in this codebase for writing TOML back out, and
  doing it losslessly (both config files are heavily commented) is a real feature of its own — see
  *persist the settings popup's changes to disk* under Medium Priority.
- ✅ **Adaptive default theme, plus Catppuccin and Nord** – the built-in default is no longer
  always the neon palette: with `[theme]` left out of both config files, Minuteman queries the
  terminal's background color over OSC 11 (bundled into `ImagePreview::new`'s existing
  graphics-capability probe, via `ratatui-image`'s `terminal_background_color_osc` option — no
  hand-rolled stdio parsing needed) and picks Catppuccin Mocha or Latte to match it, falling back
  to the original neon-cyberpunk look if the terminal never answers. Any explicit `[theme]` —
  a `name`, or even a single overridden field — always wins over this; the new
  `Config::theme_is_customized` flag (from comparing the merged `RawTheme` against its `Default`)
  is what tells `main` it's safe to auto-detect. Two new built-in palettes, `catppuccin` (Mocha)
  and `nord`, join `neon`/`classic`/`dracula` in the settings popup's Theme cycle;
  `catppuccin-latte` is reachable by name in `appearance.toml` but stays out of that cycle, which
  sticks to dark-background palettes. Chosen as COA A (an OSC 11 background query) over reading
  the OS/desktop-environment's own light/dark setting (B — platform-specific, needs a new
  dependency on Linux, and doesn't reflect a terminal deliberately themed differently from the
  desktop) or a `$COLORFGBG`-only heuristic (C — many modern terminals don't set it).
- ✅ **Appearance popup: live color/border/separator/glyph editing, mouse and keyboard (COA C)** –
  `a` (or "Appearance…" on blank space's right-click menu) opens a new modal popup, styled like
  the settings popup but genuinely usable by mouse: a click on a row acts on it exactly like
  `Enter` would (new `appearance_popup::hit`/`click_row`, mirroring `ContextMenu`'s hit-testing
  rather than the settings/Inspect popups' click-anywhere-to-dismiss). Six colors (Accent, Focused
  border, Selection, Directory, Status bar, Danger) are edited as free text — a name or
  `#rrggbb`/`#rgb` hex, exactly what `appearance.toml` already accepts, with a live preview as you
  type before `Enter` commits it; Border style, Separator and Glyphs cycle through their fixed
  choices; Reset to defaults clears every change this popup made this session, back to whatever
  theme/glyphs were already in effect (a settings-popup palette pick included) rather than forcing
  the built-in neon look specifically. `Theme::overlay_raw` (also now what `RawTheme`'s own
  `From` impl is built on, removing a duplicated field list) layers these field-level overrides
  onto whichever theme is otherwise active. New `theming::Action::Appearance`
  (default key `a`) and `context_menu::MenuCommand::Appearance` (blank space only). Chosen as
  COA C — six curated colors plus the three cycle fields, real text entry, real mouse support —
  over extending the settings popup's cycle-only rows to every field (A: not real customizing for
  colors) and a full category-submenu editor covering every field (B: sound direction, but too
  much for one cycle — see *Full appearance editor* under Medium Priority). Session-only, same
  caveat as the settings popup.
- ✅ **Persist the settings and appearance popups to `local.toml`, and move Theme into the
  appearance popup (COA B)** – every commit from either popup (a settings-popup toggle, a
  color/border/separator/glyph edit, a Theme pick, Reset to defaults) is now saved at once to a
  new, program-owned `~/.config/minuteman/local.toml`, layered highest of three files
  (`config.toml` < `appearance.toml` < `local.toml`) so it survives a restart without ever
  touching — or risking the comments in — the two hand-edited files. Chosen as COA B, a third
  file nothing but Minuteman writes, over an in-place `toml_edit` rewrite of the hand-edited files
  themselves (A: the eventual "real" answer, but the first time this program has ever written a
  config file at all is a bad place to also introduce a new dependency and edit files a bug could
  corrupt) or appending a marked, stripped-and-rewritten block to those files (C: TOML forbids a
  duplicate `[theme]` table, so this needs fragile string surgery to avoid corrupting a hand-edit
  near the marker). `local.toml` stays sparse — only ever the fields a popup actually touched,
  this session or a saved one from before — via new `RawTheme`/`RawUi`/`RawPanels` `Serialize`
  impls (the `toml` crate already skips a `None` field, no `skip_serializing_if` needed) and new
  `RawPanels::overlay`/`PanelsConfig::overlay_raw` (mirroring `Theme`'s), so a later hand-edit to
  `appearance.toml` is never silently masked by a stale saved value. `Config` exposes `local.toml`'s
  three tables unmerged (`local_theme`/`local_ui`/`local_panels`) alongside the fully resolved
  `theme`/`ui`/`panels`, so `main` can seed a popup's live state and re-save the union rather than
  overwriting a previous session's save with just this session's delta. The settings popup's
  Theme row moved into the appearance popup (now its first row, before the six colors) — it
  belongs with the rest of the look it picks a base for, and having it in two popups invited them
  to drift on what "the theme" currently is; a new `appearance_popup::theme_name` reads the
  Theme row's own display value back from either an explicit pick or, absent one, whichever named
  palette the resolved theme's colors exactly match (fixing a pre-existing quirk where the row
  used to always start labeled "neon" regardless of what was actually active, e.g. an
  `appearance.toml` pin or the adaptive auto-detected default).
- ✅ **Plugin system, stage 1 — out-of-process JSON-RPC over stdio** – a new `plugins` crate spawns
  each `[[plugin]]` entry from `config.toml` as its own process and talks a small, versioned,
  line-delimited JSON-RPC protocol over its stdin/stdout: an `init` event at spawn, a `key` event
  when the plugin's own `on_key` is pressed, and requests back into `read_dir`/`copy`/`mv`/
  `delete`/`create_dir`/`create_file`/`touch`/`rename` — the same `file_ops` orchestration the
  built-in keys use — plus a `log` notification that surfaces as the status bar message. Any
  language that can read a line and print one works, with no compile step and no per-language host
  bindings. Chosen as COA B (out-of-process JSON-RPC) over a sandboxed WASM/Extism host (A, still
  below, as the sandboxed tier to sit alongside this rather than replace it) and an embedded
  Lua-only tier (C, rejected as too narrow — "any language" was the point). Deliberately
  unsandboxed for now: a plugin runs with Minuteman's own OS permissions, an accepted trade until
  a community plugin registry (see Long-Term Vision) means running code nobody local wrote.
- ✅ **Appearance popup: color picker and saved custom themes** – any color row (Accent, Focused
  border, Selection, Directory, Status bar, Danger) can now be edited as HSV sliders instead of
  only typed hex: `Tab` mid-edit toggles between text and picker mode, `Up`/`Down` pick which of
  H/S/V a subsequent `Left`/`Right` nudges, `Enter` commits the resulting hex, and every color row
  shows a live two-cell swatch of its own value. A new "Save theme" row saves the live look under a
  name — its own value reads "new theme" or "updates '⟨name⟩'" depending on whether the live look
  still traces back to a saved theme; saving when it's drifted from that theme offers `u`pdate or
  `n`ew, saving when it matches exactly is a no-op reported in the status line, and saving with
  nothing active yet just asks for a name. New `theming::color` (`Hsv`, `hex_to_hsv`,
  property-tested RGB↔HSV round-tripping) and `theming::CustomTheme`; saved themes are full color
  snapshots (`RawTheme::from_theme`, every field set, not an overlay) in `local.toml`'s new
  `[[custom_themes]]` array alongside a new `active_custom_theme` field, so a saved theme keeps
  looking the same regardless of later `config.toml`/`appearance.toml` edits and survives restarts.
  Chosen as COA A — extend the existing popup and `local.toml` — over a separate Theme Manager
  popup plus a standalone picker overlay (B) and a swatch-grid-only version with no automatic
  update/new detection (C); reselecting, renaming or deleting a saved theme from the popup is
  deferred (see Medium Priority), since the Theme row's cycle still only knows the fixed built-in
  palettes. Verified against the real compiled binary via a scripted PTY session reconstructed
  through `pyte` (stripping the kitty-graphics APC startup query first, the same gotcha this
  project's PTY sessions have hit before): the swatch and readout tracked a live hue/saturation
  nudge in real time and `Enter` committed the exact resulting hex; saving a theme for the first
  time asked for a name, and after saving, the same row's value flipped from "new theme" to
  "updates 'verify-theme'" while `local.toml` held both the existing field-level override and a
  correct `[[custom_themes]]` snapshot. This also surfaced a harness-only finding written up in
  `DIARY.md`: the startup capability probes can still be mid-flight after the first frame renders,
  so a keystroke sent right after can be swallowed — not a bug in `mman`.
- ✅ **Built-in trash — `d`/`D`, `:trash`, OS-integrated (COA B)** – `d` (and the right-click
  "Delete") now sends the marked-or-selected entries to the real desktop trash instead of deleting
  them outright — the same trash Nautilus, Dolphin, Explorer and Finder use — confirming with `y`
  or `Enter` since it's reversible; `D` (Shift+d) keeps the old permanent, trash-bypassing delete,
  confirming with `y` only. `:trash` at the `:` prompt does the same as `d`, deferring to the shell
  when given any argument (`:trash --empty`, `:trash foo`), the same "bare form only" convention
  `mkdir`/`touch` already use for a flag they don't recognize. New `trash::send` (in the `trash`
  crate stub the workspace scaffolded from day one for this) wraps the `trash` crate — renamed
  `os_trash` in its own `Cargo.toml`, since `cargo add`/`cargo add --rename` both refuse a
  dependency sharing the local package's own name outright, though a manually written manifest
  entry compiles and resolves from the registry fine — and `file_ops::trash` exposes it alongside
  `copy`/`mv`/`delete`. Deliberately not `Vfs`-based like the rest of `file_ops`: the desktop trash
  is a local-filesystem concept with no analog over a remote backend (there is no "SSH trash"), a
  question this cycle raised directly and answered by having a future non-local `Vfs` fall back to
  a permanent delete instead of teaching `Vfs` a trash primitive or building a project-private
  trash folder — moot in practice today, since every real call site is still `LocalVfs`. Chosen as
  COA B (the real OS/desktop trash) over a private minuteman-only trash folder (A) and a remote
  per-connection trash convention (C), both raised and rejected before coding. New
  `Prompt::ConfirmTrash` and `BulkKind::Trash` mirror `ConfirmDelete`/`BulkKind::Delete` exactly,
  so the busy-state bulk machinery (progress, cancel-on-partial-failure, mark-pruning) needed only
  a few match arms widened rather than new logic. Verified against the real compiled binary via a
  scripted PTY session (answering the Device Status Report/Device Attributes startup probes per
  this project's established fix): pressing `d` then `Enter` on a real file moved it out of its
  directory and into the actual freedesktop trash (`$topdir/.Trash-<uid>/files/`, since the scratch
  directory was on a different filesystem than `$HOME`), with a correct `.trashinfo` sidecar
  recording its original path and deletion time.
- ✅ **Preview extras, stage 2 — syntax highlighting** – the text preview colors source and config
  files instead of showing one flat color. New `preview::highlight` tokenizes with `syntect`
  (parsing only — no theme/HTML machinery), classifying each token's scope into a small, curated
  set (`Keyword`/`String`/`Comment`/`Number`/`Function`/`Type`/`Plain`) rather than one theme field
  per possible scope. Every bundled theme (default, Dracula, Catppuccin Mocha/Latte, Nord) gained
  six `syntax_*_fg` colors, using each theme's own published mapping where one exists. Tokenizing
  runs off the render thread alongside the read itself (`text_preview::spawn_read`), the same
  never-block-the-render-loop treatment already given image decoding and archive listing — a
  slow-to-parse file can't stall input. `preview_view::draw_text` turns the tokens into styled
  `ratatui` spans, falling back to the preview's ordinary text color for `Plain` and for any
  language `syntect` doesn't recognize (which tokenizes as all-`Plain` already, so no separate
  code path is needed for "unknown language"). Verified against the real compiled binary via a
  scripted tmux session previewing this project's own `highlight.rs`: captured the raw truecolor
  escape codes and confirmed `let`/`mut`/`for`/`if`/`continue`/`else` rendered in the active
  theme's exact `syntax_keyword_fg`, function calls in `syntax_function_fg`, `Vec`/`Ok` in
  `syntax_type_fg`, comments in `syntax_comment_fg`, and a string literal and a number in
  `syntax_string_fg`/`syntax_number_fg` — six-for-six, byte-for-byte against the theme's own hex
  values, not just that the preview rendered without panicking.
- ✅ **Preview extras, stage 3 — external previewers for PDF and video (COA A)** – a new
  `[[preview_hook]]` config table (mirroring `[[open_with]]`'s style) names a command per
  extension and a `kind` (`image` or `text`); `tui::preview_hook::run` substitutes `{in}`/`{out}`
  into it and spawns it under `sh -c` with the same spawn/poll/timeout/kill shape `git_status`
  already uses for `git`, since a user-chosen tool is exactly as capable of hanging. An `image`
  result is decoded with the same `preview::load_image` a real image file uses (now content-
  sniffed rather than trusted from its extension, so a scratch file's arbitrary name still
  decodes) and fed straight into `ImagePreview`'s existing `ratatui-image` pipeline; a `text`
  result is read back bounded (the same 1 MiB cap a browsed text file gets) and fed into
  `TextPreview` exactly like any other text file, tokenized for syntax the same way. Chosen as
  COA A (a new hook layer in `tui`, reusing both existing rendering pipelines) over pushing hook
  config into the `preview` crate (B, which would give it a `theming` dependency and process-
  spawning it has never needed) and an image-only stage that deferred text hooks (C, which
  wouldn't honor the roadmap's own "image *or* text" wording) — both raised and rejected before
  coding. A missing program, a non-zero exit, a timeout, or unreadable output all fall back to
  quietly showing just the file's name: `ImagePreview`/`TextPreview` now distinguish a hook's own
  failure (→ `Empty`, the same status an unsupported file already reads back as) from a *real*
  image file's decode failure (→ `Failed`, unchanged, still "preview failed"), and the preview
  pane's `is_selected_image`/`is_selected_file` routing in `main.rs` switched from asking "is this
  path an image by extension" to "does the pipeline actually have something," so a hook-covered
  file with nothing to show falls through to the ordinary filename placeholder rather than being
  stuck inside a branch with no room for one. Verified against the real compiled binary via a
  scripted tmux session: a real one-page PDF (rendered from a solid-red test image via
  ImageMagick) thumbnailed through the actual `pdftoppm` binary showed up as genuine red halfblock
  output (`38;2;254;0;0` in the captured escape codes, matching the source color), while a second
  extension configured with a nonexistent program's hook showed nothing but its own file name —
  both exactly as designed, not just that neither one crashed.
- ✅ **UI overhaul, phase C (start) — animated shell-pane focus transitions (COA A)** – a new
  `tui::anim` module (pure `Duration` in, `f64` out — no ratatui or theme types, so its transition
  math is tested directly rather than through a render) drives a shell pane's border fading between
  `border_fg` and `border_focused_fg` over 180ms instead of switching instantly the moment its
  focus changes (`shell_layout::FocusAnim`, tracked per `ShellPanes` via a `Cell`-based "did focus
  just move" check inside `render`, since `render` only ever borrows `&self`). Falls back to an
  instant, unanimated switch (`style::blend_rgb` returning `None`) for any theme whose border
  colors aren't both resolved hex — a named ANSI color, or a hex color quantized to a 256-color
  index on a non-truecolor terminal, has no shade to blend through, so guessing one was rejected in
  favor of the plain pre-animation behavior. A shell pane's own poll was already 16ms whenever any
  shell is open (faster than the 180ms transition needs), so this needed no poll-timeout change at
  all. Chosen as COA A (the tick plus this one payload) over building the boot splash alone (B,
  self-contained but doesn't establish reusable tick infrastructure) or the full cinematic-layer
  bullet in one cycle (C, too large a blast radius for one bounded cycle — see the
  appearance-editor deferral above for the same reasoning) — all three raised and rejected before
  coding, at the user's direction. A pulsing selection highlight was also built and verified this
  cycle, then removed before committing — flagged as distracting in practice — so `style::pulse`
  and the pulse half of `tui::anim` never shipped; see DIARY.md. Verified against the real compiled
  binary via a scripted tmux session: opened and split a shell pane and captured a frame
  mid-transition, where both the newly-focused and newly-unfocused pane's border escape codes sat
  at the *same* blend fraction (≈0.267) between `border_fg` and `border_focused_fg`, confirming the
  two panes crossfade in lockstep rather than just snapping.
- ✅ **UI overhaul, phase C (remainder) — cinematic layer, minus the pulsing selection for good** –
  closes out the cinematic layer bullet with the four pieces last cycle deferred; the pulsing
  selection stays dropped for good (tried and cut on taste alone last cycle — see `DIARY.md` —
  rather than revisited). A brief boot splash (`boot_splash`, default on) shows the app name over
  the browser for under a second, dismissed by the first key or click. The focused browser pane's
  border and title now paint as a gradient between the theme's accent and focused-border colors
  (`gradient_borders`, default on) via a new `tui::gradient`, which walks a rect's perimeter in one
  continuous sweep and recolors every cell — border glyph or title text alike — with
  `style::blend_rgb`, falling back to the existing flat look wherever the two colors can't be
  blended; scoped to just the current pane, since shell panes already animate their own border a
  different way (the focus-fade from last cycle) and layering a second competing animation on it
  was rejected outright. The text preview streams in from the top, line by line, the first time a
  file is selected (`typewriter_preview`, default on) at 90 lines/sec — fast enough that even a
  tall pane finishes well under a second — but only for that first, unscrolled view; scrolling, or
  the same file changing on disk while still selected (`reload`), never triggers or restarts it.
  The header gained an optional system segment (`system_hud`, default off, since it is Linux-only
  and unrelated to the files being browsed) showing session uptime, load average and memory
  used/total, read straight from `/proc/loadavg`/`/proc/meminfo` — no new dependency, chosen after
  actually measuring the alternative: linking the `sysinfo` crate (with a real call, not just
  declaring it) grew the release binary by 276 KiB (~2.9%) and pulled in eight new transitive
  crates including `rayon`, a second concurrency idiom this project has never otherwise needed;
  `tui::system_hud`'s own module doc says to revisit that trade-off if Minuteman ever needs to run
  somewhere without a `/proc`. Verified against the real compiled binary via four scripted PTY
  sessions reconstructed through `pyte`: the splash showed real "MINUTEMAN" text and vanished the
  instant a key was sent; the header showed a real `up now  load 1.45 1.48 1.13  mem 5.0G/15G`
  segment sourced from the actual machine's `/proc`; a 40-line file's preview showed only its first
  line 30ms after selection and the rest a second later; one frame carried 139 distinct truecolor
  border/title colors with the gradient on against 9 with it off; and setting all four switches to
  `false` reproduced the old, un-cinematic behavior exactly — full content immediately, no system
  segment, and the same 9-color flat palette.
- ✅ **Caps Lock header pill** – a "caps lock" pill now appears in the header (danger-colored,
  next to the yanked/cut/marked pills) for as long as Caps Lock is on, and disappears the moment
  it's off. `App` tracks the state itself (`caps_lock`/`set_caps_lock`) rather than reading it only
  where `with_caps_lock_applied` already did, updated from every key event's `KeyEventState` —
  modal views and prompts included, since those still read Caps Lock for typing even while they
  own the keyboard — so the pill can never fall out of sync with what the next letter key does. No
  blinking or pulsing: this project's own diary records a pulsing selection highlight built,
  verified and then cut for being distracting, so a static, always-correct pill was chosen instead
  of reviving that effect for a different indicator.
- ✅ **Bookmarks / marks** – Ranger-style jump-to-directory bookmarks, so a frequently visited path
  is one chord away instead of re-navigating the miller columns. `` ` `` then a letter or digit
  jumps to whatever directory is saved under that register; `B` then a letter or digit saves the
  browsed directory there, overwriting whatever it held before (`m` was already `cut`, so the
  bookmark-set default landed on `B` — "Bookmark" — instead of Ranger's own `m`). Both chords are
  one-shot: any key that isn't a letter or digit cancels without acting, `Esc` cancels silently,
  and anything else still dispatches normally afterward (e.g. `q` still quits mid-chord), the same
  fallback `Alt`+leader's shell-pane chords already use. Registers persist to `local.toml`'s new
  `[bookmarks]` table (a `BTreeMap<char, PathBuf>`, mirroring how saved custom themes already live
  there) so they survive a restart. A miss (jumping to a register nothing was ever saved under)
  just reports that in the status line rather than erroring.
- ✅ **Full appearance editor with a real color picker (COA A from the appearance-popup cycle)** –
  the appearance popup is now two levels deep: a root list of five categories (Theme & colors,
  Border & separator, Glyphs, Text styles, Font) plus "Save theme"/"Reset to defaults", and each
  category drills into its own flat row list with a "‹ Back" row on top. "Theme & colors" now
  covers all 22 `Theme` color fields, not just six. Any color row toggles (`Tab`) into a
  VS Code-style picker: a saturation/value gradient square and a hue strip, painted in truecolor
  cells and set by click or drag, alongside the existing typed hex entry. "Text styles" edits every
  `[style]` element's modifier list (bold, italic, dim, underline, reverse, strikethrough) with a
  live preview, and "Font" edits the family and size. Style and font edits persist to
  `local.toml`'s new `[style]` and `[font]` tables; the font can't be applied by the TUI (the
  terminal owns it), so it only feeds a later `minuteman init-terminal`.
- ✅ **Saved custom themes: reselect, rename, delete (COA A)** – the appearance popup has a new
  "Saved themes" category on its root screen, listing every theme saved from "Save theme" by its
  own name, with the one the live look started from marked `active`. `Enter` (or a click) applies
  a theme, replacing the live colors wholesale with its snapshot; `r` renames it (the name field
  starts as the current one, and an empty name or another saved theme's name is refused); `d`
  deletes it after a `y`/`n` confirmation. Renaming the active theme keeps tracking it, and
  deleting it leaves the live look as it is but stops tracking a saved theme. Every change is
  written to `local.toml` straight away. The Theme row keeps cycling the five built-in palettes.
- ✅ **Compress and extract archives — `zip`, `tar`, `tar.gz`** – `:extract` unpacks every marked
  archive (or the selected one) into a new directory named after it, so a tarball of loose files
  cannot litter the browsed directory, and `:compress <name>` packs the marked entries (or the
  selection) into `<name>`, its `.zip`, `.tar` or `.tar.gz` ending picking the format. Both run on
  the blocking pool with the header's progress gauge and cancel with `Esc`, like a paste. New
  `file_ops::archive` does the work in-process with the `zip`, `tar` and `flate2` crates already
  used by the preview (COA A over shelling out to system tools, B, and a hybrid, C); `xz`, `zstd`
  and `bzip2` are left out to keep the binary small. An archive is untrusted input that gets
  written to disk, so every entry goes through a `SafePath` newtype (no absolute paths, `..` or
  control characters), nothing is written through a symlinked directory, only downward symlinks
  are created (hard links, devices and escaping links are skipped and counted), permissions are
  clamped (no setuid, no group/other write), and `ExtractLimits` caps entries (1,000,000) and the
  bytes actually written (32 GiB) against zip bombs, deleting a file the cap cut short. Output
  goes to a hidden sibling file and replaces the target only when complete, so a cancel leaves
  nothing behind. An existing file is an error, not a prompt.
- ✅ **Compress form on the right-click menu** – "Compress…" on a file or folder's right-click menu
  (with a count when several are marked) opens a modal form: the archive **name** (the extension
  is fixed after it, and typing one such as `backup.tar.gz` also picks the format), the
  **format** (zip, tar.gz, tar), the **level** (fast, normal, smallest; not offered for a plain
  tar), **one archive per item** (shown only for several items; each is named `<item>.<ext>`) and
  **delete originals** (to the trash, and only once that item's archive is complete). Focus moves
  with the arrows or `Tab`, values change with the side arrows, `Space` or a click, `Enter`
  starts it and `Esc` or a click outside cancels; an empty or reserved name, or one that already
  exists, is shown inside the form rather than closing it. The pure state lives in
  `tui::compress_popup` (a `Request` can only describe a well-formed job) and `file_ops::archive`
  gained `Level` and `compress_with_level`. Chosen as COA A over a format submenu feeding the
  prompt bar (B) and a chain of prompts (C).
- ✅ **Extract form on the right-click menu** – "Extract…" on a zip, tar or tar.gz opens a modal
  form shaped like the compress one: the **folder** to extract into (named after the archive by
  default, typed freely, and empty to extract straight into the browsed directory; unused with
  several archives, which each go to a folder named after them), **existing files** (stop, skip
  them or replace them) and **delete archives** (to the trash, and only once that archive has been
  extracted). With "stop", a folder that is already there is refused inside the form. The pure
  state lives in `tui::extract_popup` (a `Destination` is `Here`, one plain folder name, or a
  folder per archive) and `:extract` is unchanged. Chosen as COA C over a single "Extract" item
  (A) and an "Extract here / to…" submenu (B).

- ✅ **Mouse, stage 2 — marks by click, range select, breadcrumb jumps, middle-click** –
  `Ctrl`-click on a middle-column row toggles its mark; a range click marks everything from the last
  entry clicked (the range anchor, a path on `BrowserState`, set by a plain, `Ctrl` or range click)
  to the one under the pointer, in either direction, keeping earlier marks; a click on a segment
  of the header's path jumps to that directory (the `…` that stands for dropped segments and the
  last segment do nothing); a middle-click opens the entry, a folder in place and a file with its
  default program. Chosen as COA B: because most terminals keep `Shift`+mouse for their own text
  selection, the range modifier is the new `mouse_range_modifier` option (`"either"`, the default,
  accepting `Shift` or `Alt`; `"shift"`; `"alt"`) over the roadmap's `Shift` alone (A) and over a
  two-cycle split (C). A modified click resets the double-click clock, so two quick `Ctrl`-clicks
  toggle twice instead of opening the folder. `hud::breadcrumb` became a wrapper over `crumbs`,
  which keeps each segment's directory; `hud::crumb_rects` lays them out from the same widths
  `render_header` draws with, through one shared `header_view`. Property-tested: a range marks
  exactly the span between anchor and click plus earlier marks, and every clickable header
  rectangle sits over the text drawn for it at any width. Verified against the real binary in a
  PTY read through `pyte`: header clicks jumped to `sub`, `proj` and `~`, `Ctrl`-, `Alt`- and
  `Shift`-click marked, spanned and unmarked as described, and a middle-click entered a folder.
  Not tried in a real terminal, so whether `Shift`-click reaches the program there is unchecked.

- ✅ **Mouse, stage 3 — drag and drop** – press a middle-column row and move the pointer a couple
  of cells to start a drag; a label follows the pointer (`move 3 items to sub/`, `copy …` while
  `Ctrl` is held at the drop, `paste … into the shell`, or `… - drop on a folder` over nothing), and
  the folder a drop would land in is drawn inverted. Dropping on a folder row in either file column
  moves the entry there, or the whole marked set when the pressed row is one of the marked entries
  (an unmarked row carries only itself and leaves the marks alone); dropping on blank space in the
  left column moves up a level; dropping on a file, blank middle space or the preview does
  nothing; dropping onto a mini-shell pane types the paths, shell-quoted and space-separated and
  without a newline, into the pane under the pointer and focuses it. Holding the pointer at the
  middle column's top or bottom border scrolls the list a row every 80 ms so a folder out of view
  can be reached, and `Esc` abandons the drag. Chosen as COA C — every surface the roadmap item
  listed, with feedback — over a release-only drop with no feedback (A) and the feedback without
  the shell drop and auto-scroll (B). The drop is the same background paste as `p`
  (`App::begin_drop`), so progress, cancel, and the overwrite/skip/abort prompt on a conflict come
  free; it never touches the yank/cut clipboard, and a completed move now forgets marks on the
  files that moved. A drop is refused when any dragged folder contains the target, and entries
  already in the target folder are left out of the batch. The pure `tui::browser_drag` module
  holds the press-move-release state machine, `resolve` (what a drop at a spot does), `edge_scroll`
  and the label text; `draw` and the release both call the same `drop_target`, so the row lit is
  the drop that happens. Property-tested: a folder target is never one of its sources or inside
  one and nothing already in it travels, and the auto-scroll target is always one row outside the
  view. Verified against the real binary in a PTY read through `pyte`: the ghost and the inverted
  row while hovering; a move onto a folder; `Ctrl` at the drop copying; a marked pair moving
  together and an unmarked row moving alone; drops onto a sibling folder in the left column and on
  its blank space (up a level); no change when dropped on a file, on the preview or a folder on
  itself; `Esc` cancelling; a one-cell wobble staying a click; the yank surviving a drop; a path
  typed into a real shell pane with the shell still focused; and holding at the top border
  scrolling a list of 80 files back to a folder that started out of view and dropping into it.
  Not tried in a real terminal or with a real mouse.

- ✅ **Open-with, remainder: associations, discovery, `enter` and "Other…"** – `enter` (and `l`,
  `→`, a double-click, a middle-click) on a file now opens it: the first `[[open_rule]]` in
  `config.toml` that matches (`match` is an extension such as `md` or `tar.gz`, or a MIME pattern
  such as `image/*`; a string or a list), else the program the desktop has as the default for the
  file's type, else `xdg-open` as before. The right-click "Open with" submenu lists, best first,
  the matching rules, the `[[open_with]]` entries, then every program installed for the type, and
  ends with **Other…**, a prompt for a command (`{}` is the file, otherwise it is appended). Chosen
  as COA A — everything read in-process, no new crate — over shelling out to `xdg-mime`/`gio` (B)
  and a hybrid that launches through `gio` (C). New `tui::mime_type` reads the shared MIME
  database (`globs2` with weight and longest-pattern precedence and case-sensitive flags,
  `subclasses`, `aliases`) with a built-in extension table when it is missing, and calls a
  name-less file text or not by its first 512 bytes; new `tui::desktop_entry` parses a `.desktop`
  file (localized `Name`, `Hidden`, `NoDisplay`, `TryExec`, `Terminal`) and turns its `Exec` — the
  quoting, escapes and `%f %F %u %U %c %k %i %%` field codes — into the same `{}` command line
  `[[open_with]]` uses; new `tui::associations` merges `mimeapps.list` (desktop-specific before
  general, user before system: defaults, added, removed) with each `mimeinfo.cache`, follows a
  type's aliases and parents, finds a desktop file ID's file (including `kde-foo.desktop` as
  `kde/foo.desktop`), and skips a program that is not installed. `Terminal=true` apps and rules
  with `terminal = true` take the terminal through the existing handover. The associations are
  read on first use, not at startup. Property-tested: an `Exec` line launched under a real `sh`
  hands the program any file name (spaces, quotes, `$`, backticks, `{}`) exactly, a quoted program
  path reads back as itself, a rule for an extension covers exactly the names ending in it, the
  offered apps never repeat and never include a removed one, and every glob resolves to a
  well-formed type. Writing the tests found a real bug — a program path that needs quoting (a
  space or a parenthesis, as under `/opt/My App/`) reached the launcher still quoted and was
  reported as "command not found" — fixed with `open::program_word`, which reads the first word
  the way the shell does. Verified against the real binary in a PTY read through `pyte`, on a
  scratch desktop: `enter` ran the registered default with the exact path, the menu listed the
  installed programs in order with **Other…** last, choosing one and typing a command into
  **Other…** each opened the file, a rule won for its files, a `Terminal=true` program took the
  screen and the browser came back, and `enter` on a folder still entered it. Not tried against
  the real desktop's files on a machine with a full set of applications.

---

- ✅ **VFS abstraction hardening (COA A: complete the trait, move the readers)** – `Vfs` is now
  `Send + Sync` and gained `metadata` (following links) and `symlink_metadata` (not), returning a
  `Metadata` with kind, length, allocated size, mode, owner, link count, three times and device and
  inode, every field but kind and length optional so a backend reports what it has; `read_link`;
  `open_read`, a seekable `Read`; `scan_dir`, one call listing a folder with each entry described as
  itself, for scans; and `local_path`, which says whether a path is on this machine's disk, so a
  feature that can only work on a real local file (handing it to another program, an archive
  listing, a preview hook) skips it elsewhere instead of failing oddly. `VfsError::Unsupported` lets
  a backend decline. Preview (text, hex, images), Inspect, the disk usage view, the marked-size
  pill, the compress form's default name, and every background job in `App` (copy, move, delete,
  the `/` search, the live refresh, undo) now read through an `Arc<dyn Vfs>` the app is built with.
  A field a backend does not report is left out of Inspect, and disk usage falls back to length
  when nothing is allocated. A second backend, `shared::MemVfs` (in memory, with symlinks and a
  `sparse()` mode that reports only kind, length and times), and `shared::conformance::check_backend`
  (twelve scenarios every backend must pass) keep the trait honest: both the local disk and memory
  pass it, and each migrated feature is also tested on memory. `scripts/vfs-gate`, run by
  `scripts/check`, fails when non-test code outside an allow-list (each entry with its reason)
  touches the disk directly, and when a listed file stops doing so. Chosen over also moving
  archives onto the trait (B) and a guardrails-only change (C).

- ✅ **Native remote browsing over SSH/SFTP (COA C: the system `ssh`)** – a path beginning
  `ssh://[user@]host[:port]/` is a place on another machine, handled by the `vfs_ssh` crate:
  `Remote::locate` parses it into validated `Host`, `User` and `Target` types (nothing that could
  be read as an `ssh` option gets through), `RoutedVfs` is the `Vfs` the application is built with
  and sends a path to the disk or to that machine's `Link`, and a `Link` runs `ssh -s sftp` through
  the `openssh` and `openssh-sftp-client` crates, so host keys, `~/.ssh/config`, agents and
  `ProxyJump` stay OpenSSH's. A `Link` connects on first use, keeps one session for every thread,
  re-makes it after a failure, and retries a read once; its control process exits ten seconds after
  the application stops using it, so a crash leaves nothing running. `:cd ssh://host` opens the
  login's home. Reads and writes are blocking `Read`/`Write`/`Seek` streams, so zip listing seeks
  over the network. A copy between the disk and a machine is streamed and a move falls back to
  copy and delete, so yank and paste work across them. The backend passes the same sixteen
  conformance scenarios as the disk and memory, against a real `sshd` started on a loopback port
  for the test. Shell panes, `:` commands, opening with a local program and the trash are refused
  in a remote folder, and the live refresh slows to five seconds there. Chosen over `ssh2`
  (libssh2, a C dependency) and `russh` (a second SSH implementation to keep correct).

- ✅ **Archives through `Vfs`** – creating, extracting, checking the password of and listing an
  archive now go through the trait, so none of it needs the local disk. `Vfs` gained
  `create_write` (a seekable writer that creates new files only and never follows a link),
  `create_symlink`, `set_mode` (a backend without permissions declines, which extraction accepts)
  and `replace` (a finished archive appears under its name in one step); each has a conformance
  scenario, and `MemVfs` implements them. The extraction safeguards are unchanged. The archive
  files left `scripts/vfs-gate`'s allow-list; a line that only asks an archive entry whether it is
  a folder carries a `// vfs-gate: archive entry` marker instead.

---

- ✅ **Archives, remainder (COA A: UX and robustness, no new dependencies)** – `C` opens the
  compress form and `E` the extract form for the marked entries (`compress`/`extract` under
  `[keys]`, marks-only like yank and delete). The compress form has a **Save in** row, a folder
  relative to the browsed one or an absolute path that must already exist, with the name and
  folder checked live: as you type, a line under the rows warns when the archive (or any per-item
  archive) is already there or the folder is not, and the extract form warns about a folder that
  is already there unless Existing files is set to skip or replace. `:compress` and `:extract` no
  longer fail on an existing destination; they ask overwrite / skip / abort (`o`/`s`/`a`) the way a
  paste does, and `:compress -p name.zip` (or `--password`) seals the zip with a password asked
  for twice in a masked prompt, never typed on the command line. A zip entry now records its
  file's modified time (UTC, within a zip's 1980-2107 range). Extraction refuses a zip whose
  end-of-central-directory record claims more entries than the limit or a directory over 256 MiB,
  following zip64 records and checking every record in the file's tail, before the `zip` crate
  reads the whole directory into memory. `Prompt::Conflict` gained compress and extract sources and
  the form-building code a pure `build`, so the warning and the submit share one check. Property
  tested: every day from 1970 to 2150 converts to a date that converts back.

---

- ✅ **Encrypted archives (COA A: zip AES-256)** – the compress form has **Password** and
  **Confirm password** rows (typed as dots; greyed for tar; a note under the form says the names
  stay visible), and an encrypted zip is sealed with AES-256 through the `zip` crate's `aes-crypto`
  feature (a handful of RustCrypto crates: `aes`, `hmac`, `pbkdf2`, `sha1`, `zeroize`). Chosen over
  7z with header encryption (B) and a standalone `age` encrypt/decrypt action (C). A mismatched
  confirmation, or a password with a tar format (including a typed `.tar.gz` name), is an error in
  the form rather than a locked archive. Extracting an encrypted zip, from `:extract` or the
  right-click form, opens a masked **PASSWORD** prompt before anything is written; the password is
  checked against every encrypted archive of the batch first, a wrong one is refused with the
  archive's name and asked for again, and `Esc` cancels. New in `file_ops::archive`: `Password`
  (wiped on drop, redacted in `Debug`, capped at 256 characters in a buffer that never
  reallocates), `compress_with_options`, `extract_with_password`, `needs_password` and
  `check_password`, and the errors `PasswordRequired`, `WrongPassword`, `EmptyPassword` and
  `EncryptionNeedsZip`. Legacy ZipCrypto is never written. Property-tested: any printable password
  seals and opens arbitrary content, and a different one never does.

---

- ✅ **Undo history (COA A: inverse-operation journal)** – `z` takes back the newest create, rename,
  move, copy or trash and `Z` does it again (`undo`/`redo` under `[keys]`). Chosen over an
  app-owned holding area for deleted data (B) and a reverse-only popup (C). A new
  `file_ops::history` module records each finished operation as a `Change` (`Created`, `Moved`,
  `Copied`, `Trashed`) that can `apply` itself again and `revert` itself, and groups a batch — a
  paste or drop of several files, a trash of a marked set, a `:mkdir a b c` — into one entry, kept
  to the last 100. Reverting is conservative: a created file or folder goes only while it is still
  empty, a copy is sent to the trash rather than deleted, a move is moved back (failing instead
  of overwriting if something is in the way), and a trash is restored from the desktop trash. The
  `trash` crate gained `snapshot`/`claim`/`restore` (freedesktop trash through the `trash`
  crate's `os_limited` API) so each trashed item is matched to the path it came from, including
  when the parent path has symlinks. A step that fails partway leaves the rest on the stack and
  makes what did succeed redoable. Overwrites, permanent deletes and shell commands are not
  recorded, and the status line says so after an overwrite. Property-tested: any sequence of
  creates and moves, undone entirely, leaves the folder as it began, and redone entirely leaves it
  as it ended.

---

- ✅ **Key remap: `d` cuts, `x`/`X` delete, `s` no longer opens a shell, `y` needs marks** – the
  defaults are now `d` cut (was `m`), `x` delete to the trash (was `d`), `X` delete permanently
  (was `D`), and `shell` ships unbound (`Alt+n` still opens one). `y`, `d`, `x` and `X` act only on
  what was marked with `v`; with nothing marked they say so instead of using the entry under the
  cursor. The right-click menu still acts on the clicked entry. `B` stays bookmark-set.

---

- ✅ **Browse inside an archive as a folder (COA A)** – `enter` (or `l`, a double-click, or "Open" in
  the right-click menu) on a `.zip`, `.jar`, `.tar`, `.tar.gz` or `.tgz` opens it as a folder, and
  `h` steps back out onto the archive's own row. An archive's root is its path plus a `!`
  (`/x/a.zip!`, a file in it `/x/a.zip!/dir/f`; the spelling lives in `shared::mount`). New crate
  `vfs_archive` holds `ArchiveVfs`, a `Vfs` that wraps another and serves those paths out of the
  archive, so previews, search, marks, the disk usage view and copying a file or a whole folder out
  all work with no code of their own, and an archive on a remote machine opens through the same
  wrapper. Chosen as COA A (a read-only `Vfs` routed by path) over extracting to a temporary folder
  (B) and a zip-only first cut (C). Read-only: every way of changing an archive reports
  `Unsupported`, and `x`, a `:` command, a shell pane and opening with a program are refused inside
  one with a plain message. The table of contents is read once and kept (eight archives, reread when
  the file's size or time changes); a zip is read from its central directory and a plain tar by its
  headers, so listing costs no inflation, while a `.tar.gz` is inflated once to index it. Archives
  are untrusted input opened by moving the cursor onto them, so reading is bounded: 200,000
  entries, 512 MiB inflated while indexing, a zip central directory over 8 MiB or in zip64 is
  refused, and an entry over 256 MiB is not opened (it is held in memory, since deflate cannot
  seek); going over a limit is an error, never a quietly shorter listing. A name is cleaned of
  control and direction-changing characters, and an entry climbing out with `..` is left out.
  Links inside a tar are followed only within the archive (an absolute or escaping target leads
  nowhere, a loop ends after 16 hops). Property-tested: walking a random tree of files in zip, tar
  and tar.gz returns exactly those paths and bytes, a window onto a buffer reads exactly its
  range, an archive path round-trips through the parser, and parsing never panics on any text.
  A 50,000-entry zip is indexed cold in about 90 ms and listed warm in about 0.2 ms (an ignored
  benchmark test). Verified against the real binary in a PTY read through `pyte`: entering a zip
  and a tar.gz, a nested folder, a text preview of a file inside, stepping out with the cursor on
  the archive, yanking a file and a folder out into a real folder (contents checked on disk), cut,
  paste, delete, `:mkdir`, `:touch` and `:ls` inside an archive all refused with the archive file
  byte-for-byte unchanged, `Alt+n` refused, a damaged archive saying so without leaving the
  browser, and the git segment surviving a repository around the archive.

---

- ✅ **Filmstrip view, switched with `V` (view modes, stage 1)** – `V` (new `[keys] cycle_view`,
  `Action::CycleView`) cycles the browser between views; the first one added is the filmstrip: the
  selected entry's preview as large as the window allows, with a strip of thumbnails of its
  neighbours underneath (the cursor's slot highlighted, a mark shown as `*`, `n/N` in the strip's
  frame). `V` again returns to the name list. The big area is the ordinary preview pane, so an image
  is scaled *up* to fill it (`Resize::Scale`, bilinear; the narrow preview column still only
  shrinks), and a text file, archive listing, hex dump or folder shows exactly as it does there. In
  the strip, `j`/`k` and up/down move the cursor, a click on a thumbnail selects it, the wheel
  over the strip moves three entries and over the big preview scrolls it. New `tui::view_mode` holds
  `ViewMode` (`cycled`, `label`) and `FilmstripLayout` as pure geometry in the `browser_mouse`
  pattern (the rectangles `draw` paints and the ones a click is tested against are one function):
  the strip's height, how many thumbnails fit (always an odd count, so the cursor stays central),
  the `window` of entries shown (sliding to the ends of the folder instead of leaving empty slots)
  and `entry_at` for a click. New `tui::thumbnails` is the cache: a bounded (64) map filled on the
  blocking pool, each picture decoded *and encoded for its final cell size* there
  (`Picker::new_protocol`) so the render loop only draws a finished `Protocol` with the stateless
  `Image` widget; the cache is emptied when the slot size changes and when leaving the view.
  New `tui::filmstrip_view` draws the strip, with a `dir`/extension placeholder for anything that
  is not an image. Chosen as COA B (a view cycle, filmstrip first and grid second) over a grid only
  (A) and a bigger preview only (C). Cost: the release binary grew by 85,568 bytes (13,511,080 to
  13,596,648, +0.63%), all of it this crate's own code, as the decoders and `ratatui-image` were
  already linked. Property-tested: the layout stays inside the body and its cells never overlap for
  any terminal size, the window always holds the cursor and never runs past the folder, and a click
  never resolves to an entry outside the window; unit tests cover the cache (a ready thumbnail, a
  broken image reporting `Failed`, a slot-size change, the capacity bound with oldest-first
  eviction). Verified against the real binary in a PTY read through `pyte` with the terminal probe
  answered in full (cell size included), reading per-cell colors: a 60×40 image filled over half of
  the big area (the same image shows at its native size in the list view), four differently
  colored images each showed in their own thumbnail, the selected name was highlighted, `j j` moved
  the cursor and the big preview changed color with it, a click on a thumbnail selected exactly that
  entry, the wheel over the strip moved 2/9 to 5/9, a text file showed its content above a `TXT`
  placeholder, `V` returned to the list, and a 60×9 terminal dropped the strip and kept running.
  Not tried in a real kitty or with photographs.

---

- ✅ **Grid and details views, and a redone filmstrip strip (view modes, stage 2)** – `V` now cycles
  four views: the columns, a **grid**, a **details** table and the filmstrip. The grid is framed
  tiles over the whole window, each a thumbnail over the entry's name (a folder or a non-image shows
  `dir` or its extension), like a graphical file manager's icon view: as many columns as fit, the
  block centred, scrolling by rows only when the cursor leaves the screen. Up/down (whatever
  `move_down`/`move_up` are bound to) move a whole row, the left/right arrows one tile (they are
  `Leave`/`Enter` in the other views; `h`, `l` and `enter` still go up and in), the wheel a row, a
  click selects, a double-click opens, and marks, right-click menus, `y`/`d`/`p` and the rest work
  because a tile answers the mouse as a `Hit::CurrentRow`, so the list's existing handling runs
  unchanged. Details is one flat, full-width table with a title row (Name, Size, Modified,
  Mode) and no parent or preview column, dropping Mode, then Modified, then Size on a narrow
  terminal so a name always keeps 12 cells. It reuses the list pane, so clicks, marks, drag and drop
  and the context menu already work, through one new function, `view_mode::browser_layout`, that
  drawing and every mouse path ask for the rectangles. The filmstrip's strip was reworked after it
  looked messy (tiny pictures floating in a tall, mostly empty frame): it now holds the same
  bordered tiles as the grid, sized from one shared `tile_size`, and thumbnails are scaled up to fill
  their tile (`Resize::Scale`, bilinear) instead of staying at their native size. New modules
  `tile_view` (one tile), `grid_view`, and an extended `view_mode`; `hud::Columns` gained `modified`
  and `mode`, `themed_block` no longer adds two blanks to an unnamed frame, and `cycle_view` in
  `config.toml` documents the keys per view. Chosen as COA A's grid and details with the filmstrip
  kept (the choice made when it was presented), over a grid with a side preview (B) and four
  separate view systems (C). Cost: the release binary grew by 17,240 bytes (13,596,648 to
  13,613,888, +0.13%). Property-tested: grid tiles stay inside the body and never overlap for any
  size, the cursor stays on screen after `follow`, cursor steps never leave the listing and a
  down-then-up returns, a click resolves only inside a visible tile, a tile's picture and name stay
  inside it and apart, and the details plan and header agree. A property test also caught that two
  empty rectangles compare unequal, which was a mistake in the test, not the drawing. Verified
  against the real binary in a PTY read through `pyte` with the terminal probe answered in full:
  the grid drew five tiles across and four rows, a colored image filled its tile, `Right` moved
  one, `j`/`k` a row, six `j` scrolled the first row off with the cursor's name still shown, the
  wheel moved a row, a click selected a tile, a double-click entered a folder (an empty folder said
  so) and `h` returned; details showed the title row, an exact date, `rw-r--r--`, the size and a
  table spanning the full width, with `j` and a click selecting; the filmstrip's strip drew framed
  tiles with colored pictures; and the fourth `V` returned to the columns. Not tried in a real
  kitty or with photographs.

---
- ✅ **Default theme follows the terminal's own colors** – with `[theme]` left out of both config
  files, Minuteman now asks the terminal for its foreground and background (OSC 10/11) and its 16
  ANSI colors (OSC 4), then builds the whole theme from them (`Theme::from_palette`), so it matches
  whatever the user's wallpaper or theming tool (pywal, matugen, a hand-made scheme) already pushed
  into the terminal, on any window manager, desktop or OS. Panel and bar backgrounds stay `reset`
  so a translucent terminal shows through; only the selection row gets a solid color. The accent is
  the most colorful non-red ANSI slot, and every text color is lifted toward the foreground until
  it reaches a readable contrast against the background, so a dim palette entry never becomes
  unreadable. If the terminal answers only the background, the Catppuccin Mocha/Latte pick stays;
  if it answers nothing, the neon look stays. The probe (`terminal_palette.rs`) runs after the
  graphics probe, ends on a device-attributes reply so it never waits for a terminal that ignores
  OSC, and is capped at 300 ms. Unix only for now; other platforms keep the old behavior. Chosen
  over a hard-coded palette (A, goes stale when the wallpaper changes) and a user palette file
  (C, needs manual wiring per user), and over reading one tool's cache or the compositor's config,
  which would only work for people using that tool. It is also a named palette, `terminal`, last
  in the appearance popup's Theme cycle and valid as `name = "terminal"`, so it can be picked and
  kept even when another `[theme]` field (or an earlier popup save) has turned the automatic
  default off; the probe therefore runs on every start. `default` is the same adaptive look as a
  name (terminal colors, else Catppuccin from the background, else neon), first in the cycle and
  the base for a `[theme]` that sets fields without a name; `terminal` stays as the strict one.
- ✅ **Folders open in the background** – entering, leaving, `:cd`, a bookmark jump, a breadcrumb
  click, a click in either column and the menu's Open no longer wait for the listing on the render
  thread, so a folder on another machine (where connecting alone can take ten seconds) or a large
  archive no longer freezes the interface. A move is now planned (`BrowserState::plan_enter`,
  `plan_leave`, `plan_goto`), loaded anywhere (`MovePlan::load`, which touches nothing) and applied
  in one step (`apply_move`); the sync `enter`, `leave` and `goto` run that same path, so there is
  one implementation. `App::begin_move` waits up to 40 ms for the result, so a local folder is open
  before the key press returns and nothing typed right after is lost; a slower one makes the app
  busy (the BUSY pill, `opening <folder>…` in the status line), keeps the old folder on screen with
  its cursor and marks valid, and holds other keys back, with `Esc` abandoning the move. A result
  for a folder the browser has since left is dropped. A folder that cannot be read now leaves the
  browser where it was and says why; before, a failed step up exited the program and a failed
  `goto` left the browser on the folder it could not list. Chosen as COA A (the move itself made
  asynchronous, old folder kept) over also showing the target as an empty listing at once (B,
  leaves the cursor, marks and preview pointing at a folder with no entries) and moving the whole
  browser onto a worker thread (C, a rewrite of every read of the listing).
- ✅ **The chosen view is remembered** – `V` now saves the view (`list`, `grid`, `details` or
  `filmstrip`) to `local.toml` and the next launch starts there. `view` in `config.toml` sets the
  first one, and the saved choice wins once `V` has been used. `Config::save_view` rewrites the
  file with everything else as it is on disk, and `save_local` keeps a stored `view` when its
  caller (the popups) does not set one, so a popup save never erases it.
- ✅ **Sorting, local time, grid icons, page keys and prompt editing** – five small gaps closed in
  one cycle. *Sorting:* `,` cycles the order (name, size largest first, modified newest first,
  permissions) and `<` reverses it; in the details view the title of the ordering column wears an
  arrow, and clicking a title orders by it and clicking again flips it. The order lives in
  `BrowserState` (`browser::sort`) and is applied whenever a listing is stored, the same moment
  hidden entries are dropped, so the cursor, `/` search, marks and every mouse hit all index the
  list the user sees; folders always come first, equal keys fall back to the name, and the cursor
  follows its entry when the order changes. It applies in every view and lasts for the session.
  *Local time:* Inspect and the details column show the modified time in the machine's time zone
  (`chrono`, already built for the trash crate; Inspect writes the offset out, the column says
  just `Modified`), replacing the UTC-only formatting and its hand-written calendar code. *Grid
  icons:* with the Nerd glyph set a folder or non-image tile draws its file-type icon, in the type's
  color, above the extension label; other sets keep the text label. *Page keys:* `PageUp`,
  `PageDown`, `Home` and `End` move a screenful (the list's rows, the grid's tiles, the strip's
  thumbnails) or to the first or last entry in every view; fixed, like the disk usage view's.
  *Prompt editing:* rename, create, `/`, `:` and Open with → Other… now edit a line with a cursor
  (`line_edit::LineEdit`): arrows, Home/End, Delete, `Ctrl`-A/E/B/F/D/W/U/K, `Alt`-B/F and `Ctrl`
  or `Alt` with the arrows for words; the cursor is drawn where typing lands, a `/` search only
  re-runs when the text changes, and `Ctrl`+`Alt` characters (AltGr layouts) still type. Chosen as
  COA A for the time crate (`chrono`, over `jiff` and a direct `localtime_r`) and for the editor
  (a shared `LineEdit`, over a cursor field per prompt and an end-of-line-only Ctrl-W/U). The sort
  keys are `,` and `<`: `o` and `s`, the natural picks, are deliberately unbound by an existing
  test of freed keys.
- ✅ **Fuzzy `/` search, ranked, with stepping** – `/` now matches a name that holds the typed
  letters in order, not necessarily side by side (`aernd` finds `aerend`), and ranks the matches.
  A name's score (`Query::score`) is the best way of laying the letters over it: a run side by side,
  a letter at the start of a name or after `.`, `_`, `-` or a space (or a capital after a
  lowercase letter), and a matching capital score higher, gaps and leading gaps lower, and a name
  that holds the whole text as one piece beats every name that does not; among equal fits the
  shorter name wins. It is a dynamic program filled with a running best, so a name costs one pass
  of letters times characters, and the "letters in order" check runs first without allocating
  (about 0.1 µs for a name that fails and under 1 µs for one that matches, in a release build, so
  200,000 entries cost well under half a second). The walk keeps its nearest-first rule but now
  finishes the nearest level that has any match before answering (`find_below`), so the best of
  that level wins rather than the first met; if the entry limit stops it partway through such a
  level, what was found so far is the answer. The answer is the best 20 (`Outcome::Found` carries a
  list). The first level is the folder already on screen, ranked at once from the loaded listing
  (`rank_entries`) with no walk. While the prompt is open `Down`/`Ctrl-N` and `Up`/`Ctrl-P` step
  through the kept matches, wrapping, crossing folders if they are in different ones, and the
  status line shows `2/14` (a `+` when more were found than kept); `Esc` still returns to where
  the search began. Chosen as COA A (nearest level first, ranked within it) over ranking across the
  whole walk with a depth penalty (B, always scans up to the cap so answers are slower and move
  while typing) and a matcher crate (C, a new dependency that is not available offline).
- ✅ **Instant start** – measured with a terminal that answers every query the way kitty does:
  the first frame is drawn about 14 ms after the process starts, so the wait people felt was the
  900 ms boot splash, plus the splash swallowing the first key pressed. `boot_splash` now defaults
  to off, and with it on the browser is drawn underneath from the first frame and a key press goes
  on to do its job after dismissing the splash (a click only dismisses it, since it would land on a
  row hidden behind). Not changed, and the next place to look: a terminal that does not answer
  the startup queries (some tmux and ssh setups) costs 2 s for each query left unanswered (4.3 s in
  a test with a silent terminal), because the keyboard query and the graphics probe each wait for
  their timeout; cutting both to a few hundred milliseconds would fix that at the risk of a slow
  terminal missing its image support.
- ✅ **Marks are the only selection, and spent when used** – `v` is now the one way to choose
  entries. Before, the keys already refused to act on nothing marked, but the right-click menu's
  Copy, Cut, Delete, Compress and Extract and the `:trash`, `:compress` and `:extract` commands fell
  back to the row under the cursor, and the marks stayed after being used, so a later `x` could hit
  what a `y` had just taken. Now every operation reads the marks alone (`App::marked_only`) and
  spends them: `yank` and `cut` clear them as they fill the clipboard, a confirmed trash or delete
  clears them (declining keeps them), and a compress or extract clears them when it starts. A
  plugin key still receives the marks or else the cursor row, since that is the plugin API's
  documented meaning, and dragging an unmarked row still drags it, since that is a pointer gesture
  rather than an operation on a selection.
- ✅ **Trash browser (`T`)** – a view over the whole screen listing everything in the desktop trash,
  newest first, with its name, size (bytes, or the number of entries in a folder), when it was
  deleted and the folder it came from; the columns thin out on a narrow terminal, folder first.
  `v` marks, `r` puts the marked entries back, `x` deletes them for good and `E` empties the whole
  trash, the last two behind a `y` question, and nothing ever acts on the row under the cursor
  unless it is marked (`E` is the one operation that takes everything). A restore onto a place
  that is taken, or an entry already gone, fails for that entry alone, keeps it in the trash and
  says why while the others go through. Reading and acting run on the blocking pool, the view
  showing `loading…` or `working…`. The `trash` crate gained `entries`, `restore_entries`,
  `purge_entries` and the plain-data `TrashEntry`, which looks items up again by the trash's own
  id (one listing per batch), and the view reaches the trash through a `TrashBackend` trait so its
  tests use a fake. Chosen as COA B (a popup view like the disk usage view) over browsing the trash
  as a virtual folder (A, the `Vfs` is built for real paths and every key would need a special
  case there) and shelling out to `trash-cli` (C, an external tool most people lack).
  *Preview:* on a terminal 100 columns wide or more the selected item is previewed in a pane beside
  the list, by the same pipelines as the browser's preview column and read from where the trash
  keeps the item (`TrashEntry::stored_path`, worked out from the freedesktop `info/` and `files/`
  layout), so nothing has to be restored to be looked at: text, an image, an archive's listing, a
  hex dump, or a folder's contents. `J`/`K` scroll it. The drawing the browser used for its preview
  column was moved into `draw_preview_pane` and both screens call it, so there is one set of rules
  for what a file shows as.

- ✅ **Plugin triggers (stage 1.5, first cycle)** – a `[[plugin]]` can bind several keys
  (`on_key = ["b", "B"]`, a plain string still works), declare `:` commands (`commands =
  ["blame"]`) and be fired by name with `:plugin <name> [args]`. The `key` event says which key
  fired, and a new `command` event carries the command word and its arguments, both as additive
  fields, so `API_VERSION` stays 1. A `:` line reaches a plugin only after the built-in parser has
  declined it and only when it uses no shell syntax, so a plugin adds commands but never shadows
  `cd` or `q`. `examples/plugins/dir_stats.py` is a dependency-free Python reference plugin, run
  end to end against a real `python3` by the integration tests, which closes the gap that nothing
  had exercised an external interpreter. Verified in the compiled binary through a PTY: both keys,
  `:stats`, `:plugin dir-stats touch made.txt` (the file appeared) and an unknown name's message.
- ✅ **Plugin `get_state` (stage 1.5, second cycle)** – a plugin can ask the browser where it is at
  any moment, not only when an event fires: a `get_state` request (no parameters) is answered with
  `{cwd, cursor, marks, show_hidden, sort, sort_reversed, view}`. Chosen as COA A, a shared
  snapshot: `App::poll_plugins` publishes the state on a `tokio::sync::watch` channel each tick (only
  when a plugin is running, and an unchanged state is dropped), and a plugin's reader answers from
  the latest value without touching the render thread, so a request never waits on a busy or
  modal interface. It is at most one tick (about 100 ms) behind, and the channel is the one the
  throttled `dir_changed`/`selection_changed` events will use. Before the first tick the answer
  already names the start directory. Additive, so `API_VERSION` stays 1. `examples/plugins/
  dir_stats.py` gained `:plugin dir-stats state`. Verified in the compiled binary through a PTY
  with two entries marked: the plugin reported `cursor=alpha.txt marks=2 sort=name view=list` and
  the browsed directory.
- ✅ **Plugin events (stage 1.5, third cycle)** – a plugin that lists `events = ["dir_changed",
  "selection_changed", "job_finished"]` in its `[[plugin]]` table hears about them as they happen;
  one that lists none gets nothing new. The first two carry `from` and `to` (the old and new
  directory, or cursor entry) beside the usual `cwd` and `selection` (the marks); `job_finished`
  carries the `kind` (`copy`, `move`, `delete`, `trash`, `extract`, `compress`, `command`) and the
  `outcome` (`completed`, `skipped`, `cancelled`, `failed`). Chosen as COA B: the events come from
  diffing the snapshot `get_state` already publishes each tick, and each plugin has a mailbox that
  holds at most one `dir_changed` and one `selection_changed`, folding a burst into one event from
  where it began to where it ended (and none at all if a directory burst ended where it began).
  The plugin's writer drains the mailbox, so a plugin that stops reading just stops receiving
  intermediate positions, and holding `j` cannot build a queue behind a slow one. `job_finished`
  is never folded and fires where a job truly ends: a batch paste is one job, and one paused on a
  conflict prompt has not finished. Additive, so `API_VERSION` stays 1; an unknown event name is
  reported on stderr and skipped. `examples/plugins/dir_stats.py` shows both. Tested with a
  property test over any run of changes (always one event, first `from`, last `to`), a plugin that
  never reads stdin being fed twenty thousand changes, and a real `python3` plugin. Verified in the
  compiled binary through a PTY: entering and leaving a folder showed `entered sub` and `entered
  tree`, and a marked file copied into the folder showed `copy completed`.
- ✅ **Plugin restart policy and `:plugins` (stage 1.5, fourth cycle)** – a plugin whose process has
  ended is started again the next time something is meant for it: a key, a `:` command, `:plugin
  <name>`, or, for a plugin that has only `events` and no keys or commands, an event (one with a key
  or command waits for those). Chosen as COA C, lazy respawn, over a supervisor task per plugin (B)
  and listing plus a manual restart only (A). Before this a key bound to a plugin that had exited
  was reported as handled and did nothing, silently. Exiting with status 0 counts as finishing,
  which is allowed, so a script that does its job on a key press and quits works without any
  special case; a crash does not, and a plugin that crashes three times in a row, each within a
  minute of starting, is given up on: each key press or command then says `stopped after 3 crashes
  in a row; :plugins restart NAME starts it again`. A crash after a minute of running starts the
  count again. `:plugins` lists every plugin as `running`, `finished`, `crashed (exit status: 1)`
  or `stopped after repeated crashes`, with how many times it was restarted, and `:plugins restart
  <name>` stops a live process (a restart is not reported as a crash) or clears a stopped one and
  starts a fresh process. A new process gets `init` again with the directory last shown, and has
  none of the old one's memory. Only these two exact forms are the built-in; any other `plugins
  ...` line still goes to the shell. Found by the tests: a respawned plugin's first change event
  could be written before its `init`, because the writer chose between its queue and its mailbox
  at random; the queue now always goes first. Tested with a property test over any spacing of
  crashes, and end to end with the Rust fixture (one-shot respawn with a new pid, the three-try
  limit, restart of a stopped and of a live plugin, event respawn for a listener and none for a
  plugin with a key). Verified in the compiled binary through a PTY: a one-shot plugin answered
  two key presses from two different processes, a crashing plugin was stopped after three presses
  with the message above, `:plugins` showed both, and `:plugins restart` started the crasher again.
- ✅ **Plugin `register_command` (stage 1.5, fifth cycle)** – a plugin can send `register_command
  {name}` while it runs and `:name` reaches it from then on, with no line in config;
  `unregister_command` gives a word up again. Chosen as COA A, a shared registry, over a message to
  the render thread (B) and a reply-less notification (C). New `plugins::registry` holds who owns
  each word, config words and registered ones together, behind one lock that every plugin's reader
  and the manager share, so a request is answered at once with the real reason and never waits on
  the render loop (the reasoning that chose a snapshot for `get_state`). A word is one or more
  letters, digits, `-`, `_`, `.` or `+`, so it is one a `:` line can start with and still reach a
  plugin; a built-in command word is refused (`tui` hands the list over at startup, and a test
  reads `command::parse`'s own arms so a new built-in cannot be left off it), so is a word another
  plugin owns, and asking again for one you hold is fine. Config words keep the looser rule they
  always had and are held to the same ownership, and only a word registered while running can be
  unregistered. A registered word outlives the process that registered it, so `:blame` still
  starts a plugin that has exited; `:plugins restart` forgets a plugin's registered words, and the
  fresh process registers them again at `init`. A plugin whose only commands were registered at
  run time counts as having commands, so an event does not start it again. Additive, so
  `API_VERSION` stays 1; a `description` param is ignored for now, so a plugin that sends one keeps
  working when it is used. `examples/plugins/dir_stats.py` registers `:state`. Tested against a
  plain model of the rules over any sequence of registers, unregisters and clears, and end to end
  with the fixture and a real `python3`. Verified in the compiled binary through a PTY: a plugin
  with no `commands` answered `:state`, one that asked for `cd`, `hello` and `a b` was told that
  `cd` is built in, got `hello`, and was told what a name may hold, `:hello world` reached it with
  its argument, and `:cd /tmp` was still the built-in.
- ✅ **Appearance shape and a settings-window appearance popup (COA B)** – the look now has a
  shape as well as colors: a `[shape]` table in `appearance.toml` sets the blank cells between the
  panes (`gaps_in`, 0–4) and to the screen edge (`gaps_out`, 0–8 columns and half as many rows),
  where pane titles sit (`title_align`), a shadow under menus, popups and the shell box (`shadow`)
  and dimmed parent and preview columns (`dim_inactive`). `border_type` gained `dashed`,
  `thick-dashed`, `block`, `block-inside` and `none` (blank frame symbols, so a pane keeps its cell
  and the mouse code its rows). New `theming::shape` holds a typed `Shape` (gaps are a `Gap<MAX>`
  that can only be built clamped) inside `Theme`, so every saved theme now stores its shape along
  with its colors, and `local.toml`, live preview and Reset carry it with no new state. A `[shape]`
  alone does not pin the colors, so they still follow the terminal. The gaps are applied in
  `view_mode::browser_layout`, which drawing and every mouse handler already share, so clicks land
  on the gapped rows. The appearance popup was rebuilt like a desktop settings window: five pages
  down the left (Theme, Colors, Shape, Text, Font), switched with `Tab`/`Shift+Tab`, `1`–`5` or a
  click, and the chosen page's rows on the right. The Theme page lists the built-in palettes and
  your saved themes together (a palette replaces the colors and keeps the shape); the Shape page
  steps numbers with `h`/`l` (`H`/`L` by five) and flips switches with `Enter`. Chosen as COA B
  (shape plus the window rebuild plus themes that keep their shape) over a Shape category in the
  old two-level popup (A) and named shape presets only (C). +40 KB (+0.28%) on the release binary,
  no new dependency. Verified in the compiled binary through a PTY: the five pages, the gaps (frames
  at the expected columns, two blank rows on top), a click still selecting the row it was drawn on,
  each change in `local.toml` at once, a saved theme holding a shape table, a palette keeping the
  gaps, Reset dropping them and the saved theme bringing them back, and the shadow appearing and
  going with a popup.
- ✅ **Configurable border gradient (COA B)** – the gradient that used to be one `config.toml`
  switch (accent to focused-border color, once around the focused pane, restart to change) is
  now part of the look. `gradient_from`/`gradient_to` in `[theme]` take a color or the keywords
  `accent`/`focused` (the default, so nothing changes until asked, and a palette or saved theme
  follows its own accent); `[shape] gradient` picks which frames get it (`off`, `focused`,
  `panes` for every file column, `all` adding menus and popups) and `gradient_flow` which way it
  runs (`around` as before, `mirror` around and back without a seam, `across`, `down`). The
  Colors page edits the two ends (a keyword's edit starts from the color it stands for, and its
  swatch shows that color), the Shape page cycles the other two, and a saved theme keeps all
  four. `gradient_borders = false` in `config.toml` still means `off` beneath anything the
  appearance files set, and `Config::gradient_borders` is gone in favour of
  `theme.shape.gradient`. Chosen as COA B over moving the switch alone (A) and a moving gradient
  (C, kept below as a future item). Verified in the compiled binary through a PTY by reading the
  truecolor value of each frame cell: a gradient on the focused frame and a flat parent by
  default, gradients on the side columns with `panes`, one color per side edge with `across`, a
  red left edge after setting the start to `#ff0000`, a gradient on the popup's own frame with
  `all`, flat frames with `off`, all four in `local.toml`, and the old switch still turning it off.

- ✅ **Plugin `run_action` and `list_actions` (stage 1.5, sixth cycle)** – a plugin can send
  `run_action {name}` and Minuteman does what the key bound to that action does, confirmations
  included; `list_actions` answers with every name. A name is the word that binds the action under
  `[keys]` (`paste`, `sort`, `undo`, `hidden`, ...), so a user and a plugin call an action the same
  thing. The request is one more turn of the render loop, not a second dispatcher. `theming::Action` gained `ALL`, `name`,
  `from_name` and `waits_for_key`; `plugins::actions` hands a request to the render thread as an
  `ActionRequest` and waits, and `tui::plugin_action` decides whether it may run. The browser's own
  key handling was lifted into one `Browse` step that a key press and a plugin's action both enter,
  so a plugin's `paste` is the `p` key's paste and no action is written twice. It runs only while
  the user is plainly browsing: a popup or view, a prompt, a chord, a focused shell or a running
  job refuses it and the plugin is told which. The chord starters (`bookmark_jump`, `bookmark_set`,
  `leader`) are refused and left out of `list_actions`. The reply is held until the tick has
  published the state the action left, so `get_state` straight after it sees that state. Additive,
  so `API_VERSION` stays 1. `examples/plugins/dir_stats.py` gains `:plugin dir-stats run ACTION`
  and `actions`. Tested with a check that every name binds exactly its action, properties over any
  name a plugin might send, the refusal of every action in every mode, and end to end with the
  fixture and a real `python3`.

- ✅ **Plugin system, stage 1.5 closed (seventh cycle)** – every additive hook the stage promised
  now exists (keys, `:` commands, `get_state`, events, restart, `register_command`, `run_action`),
  and the cost estimates the stage made are measured. The model stays additive: built-in features
  are native and untouched, a plugin cannot replace or remove one, and a user who wants a built-in
  gone rebinds or unbinds its key (a `disabled_actions` list remains under the action registry
  item). Measured on a Ryzen 5 5500U, release build, with new `#[ignore]`d benchmarks in
  `crates/plugins/tests/plugin_cost.rs` (three runs, the figures held within about 20%): a plugin
  is ready about 0.9 ms after `spawn` for a Rust program and about 23 ms for the Python example,
  nearly all of it the interpreter; one request and its reply cost 14 to 22 µs (`list_actions`,
  `get_state` and `run_action` alike, the last with the host answering it); and a browser with no
  plugin configured spends about 22 ns per tick on the manager, with no thread or process. So the
  estimates held: a few ms to start, well under a millisecond per call, nothing for anyone who does
  not use plugins. There is no stored baseline, so the 5% gate is a manual comparison of two runs.
- ✅ **Fuzzy finder on `f` (COA A)** – a panel over the browser that lists every entry below the
  current directory, ranked as you type, with the highlighted entry previewed beside it. It keeps a
  flat index of the tree (`browser::finder::walk`, nearest level first, through `Vfs::list_dir`
  only, on the blocking pool as `tui::finder_job`) and re-ranks it on every keystroke
  (`Ranking`, which scores each entry once and only scores what arrived since). The score is the
  `/` search's own (`Query::score`) applied to the path: a match on the entry's name earns a bonus
  of 500, so `main` puts `src/main.rs` above `domain/a/i/n.rs`, and a typed path (`src/main`)
  matches across folders. Results fill in while the walk runs, so it opens at once on a large
  tree; the walk is cancelled when the panel closes. The preview reuses the browser's pipelines
  (text, hex, archive listing, image, folder listing) through `preview_subject`, the way the trash
  browser does, and the panel is laid out by `trash_view::layout`. `Enter` closes it with the
  browser's cursor on the entry (`BrowserState::reveal`), so `l` then opens or enters it. Chosen as
  COA A (one source, the index behind a ranking type) over modes for directories, content and marks
  in one popup (B, too large for one cycle; `Ranking` and the popup are built so a second source
  can be added) and shelling out to `fd` and `fzf` (C, two external programs, nothing on a remote
  or archive mount, no theme or preview). Measured only by tests: ranking is property-tested
  (every result matches, no more than the limit, best first), the walk, the job, the popup's keys
  and the drawn panel have unit tests; it has not been timed on a large tree.
- ✅ **Fuzzy finder off the render thread** – the finder's ranking and index moved to an engine on
  the blocking pool (`tui::finder_job`), so a large tree such as `/` no longer stutters, and what it
  lists is unchanged. Measured on a Ryzen 5 5500U, release build, 200,000 synthetic paths
  (`crates/browser/tests/finder_cost.rs`, ignored, run by hand): the first letter costs about 190 ms
  of scoring in total, which used to block the interface on that keystroke and now runs in slices of
  2048 entries of about 2 ms each; the next letters narrow from the previous matches, in 64 ms for
  `ab`, 12 ms for `abc` and 3 ms for `abcd`; deleting a letter returns to its kept result at once;
  choosing the best 200 of 58,000 matches takes 1.5 ms. The render thread now only sends the query
  and takes the newest snapshot. A folder's preview is read in the background, once per selection
  and limited to 300 entries, in place of a `list_dir` on every frame. The ranking is exact:
  a property test types and deletes letters in any order, with the index growing and the work cut
  into tiny slices, and requires the same results as ranking from scratch.

- ✅ **Devices view: the storage deck (COA B)** – `V` has a fifth view, **deck**: the usual columns
  under a strip of cards, one per internal disk, USB stick or network share. A card shows the name
  (label, else mount folder, else kernel name), where it is mounted or that it is not, a gauge of
  how full it is with the free space (red from 90%), and a sparkline of the last 32 seconds of
  reads and writes with the current rate. The card holding the folder you are in has the bright
  frame; a device plugged in slides in over a quarter of a second; one that is plugged in but not
  mounted shows too, with its size. `Tab` takes the keys to the cards (`←`/`→` or `h`/`l` choose,
  `Enter` opens one, mounting it first if needed, `m` mounts or unmounts, `e` unmounts and powers
  the disk off, `Esc` gives the keys back; everything else stays the browser's, so `q` and `j`/`k`
  work); a click chooses a card, a second click opens it, the wheel moves along them. The data
  layer is a new crate, `devices`: `mountinfo` (the mounts worth showing, one per device, with
  boot partitions, snap loop devices and virtual filesystems left out), `udev` (label, filesystem
  and bus from `/run/udev/data`, which is how an unmounted stick has a name), `probe` (`Sys::scan`
  over `/proc`, `/sys` and `statvfs`, pointed at a copy of the files in tests), `activity` (rates and
  history from `/proc/diskstats`) and `udisks` (`udisksctl`, with a 25 s timeout so a password
  prompt with nobody to answer cannot hang). In `tui`, `deck` is the pure state and `deck_view` the
  drawing, with one layout function (`cards`) shared by the drawing and the mouse; the strip is cut
  from the columns by `view_mode::browser_layout`, which gained `BrowserLayout::deck`, and a window
  under 12 body rows gets no strip. The machine is looked at once a second on the blocking pool,
  only while the deck view is showing, and a scan still running is not started again, so a hung
  network mount cannot stall the interface. Found and fixed against this machine's real mounts: a
  btrfs root reports a made-up device number and showed up a second time as an unmounted device,
  and the hidden `/boot/efi` showed as unmounted; devices are now matched by the node their mount
  source links to. Chosen as COA B over a places rail (A) and a zoomable storage atlas (C); both
  remain below. Linux only. Not run by hand with a real USB stick or `udisksctl` here.

- ✅ **Cargo-aware deck** – the deck's cards answer "will it fit?" before anything is copied. What is
  carried is the yanked or cut clipboard, else the marked files (a yank spends the marks, so it is
  never both); each card then replaces its activity line with a verdict: `✓ fits · 22G left`,
  `~ tight · 8.0M left`, `✗ needs 4.0G more`, `✗ a file is over 4.0G` (a FAT32 stick cannot hold a
  file of 4 GiB or more however empty it is), `✓ same disk · instant` (a cut onto the disk the files
  already sit on takes no room), `mount to check (Enter)` or `? cannot tell`. A card that cannot take
  the load is dimmed. `p` in the deck sends the load to the chosen device's root through the ordinary
  paste (progress, cancel, the overwrite prompt), yanking the marks first if they are all there is,
  and refuses a device that cannot take it with the reason in the status line, which is the point:
  the failure is met before the copy and not halfway through it. The answer is `cargo_fit::verdict`,
  pure (sizes and a device in, a verdict out) and tested on the boundaries: the byte that tips a
  load over, the exact FAT32 limit (allowed) and one byte over (refused), a copy and a move within
  one disk, an unmounted device, an unknown size, and a load too big to measure fully. Sizing reuses
  the marks' walk (`marked_size`), now also finding the biggest single file (`measure`, with
  `total_of` unchanged for the header pill and `Tally::largest` added), and a second walk for the
  clipboard so the pill is not disturbed. A load that leaves under 16 MiB or 2% of itself is called
  tight, not fitting, because a filesystem rounds each file up to a whole block and many small
  files take more than their sizes add up to. Tried in the real app with a yanked file: the card
  showed `✓ fits · 22G left` and `p` ran the paste (which then failed, as it should, on a root
  this user cannot write to). The refusing cases (`needs … more`, a file over the FAT32 limit) are
  covered by tests only, as this machine has no small or FAT32 device to show them on.

- ✅ **Wiring diagram (COA C: wide boxes in columns, a tree when narrow)** – `V` has a sixth view,
  **wiring**, that takes the whole body and shows how the machine's storage is wired. Read from
  this machine, the NVMe drive shows as the PCIe slot, the disk (by model), its boot partition, its
  LVM physical volume, the LVM volume, the encrypted container and the btrfs filesystem on top, with
  the mount, the gauge and the fit verdict on the nodes that hold a filesystem: the stack the deck
  could only show as one "System" card. Each layer is a column of framed boxes joined by lines drawn
  from box-drawing characters (the junction at each cell comes from which of its four sides have a
  line, so a trunk meeting two children is `┤` and the corners `┌` `└` come out right); a mark
  (`●`) moves along the line into any node that is busy, at a steady pace measured from a fixed
  start; the node holding the browsed folder has the bright frame and the chosen node is filled. In
  a window too narrow for the columns the same model is drawn as an indented tree, one row a node,
  with `└─`/`├─`/`│` branches. Keys: the arrows or `h`/`j`/`k`/`l` (left to what a node is under,
  right to what is under it, up and down along the page), `Home`/`End`/`PageUp`/`PageDown`,
  `Enter` open (mounting first), `m` mount or unmount, `e` eject the whole removable disk with every
  volume on it unmounted first, `p` send the carried files with the deck's fit check; a click chooses
  a node (and, since the folder cycle, unfolds it), the wheel moves through them. The data is a new `Topology` in
  the `devices` crate: buses, disks, partitions, stacked devices (LVM, LUKS, RAID, multipath by
  device-mapper's own uuid prefix) and shares as nodes that point at each other, built from
  `/sys/class/block`, the kernel's `slaves` links, the udev database and `mountinfo` (this view
  keeps `/boot`, which the deck leaves out), with the bus read from where the disk sits under
  `/sys/devices` (`PCIe 02:00.0`, `USB 2-3`, `SATA ata1`, `MMC mmc0`). A RAID array has several
  parents, so it is drawn once under the first and the model keeps the others. `device_actions` was
  taken out of the deck so both views mount, unmount, eject and check a send the same way. Tried in
  the real app on this machine, wide and narrow. Chosen as COA C over the wide diagram alone (A,
  blank on a small terminal) and the tree alone (B, reads as a list). Not tried against a real USB
  stick, a RAID array or a SATA disk: those shapes are covered by fixtures built to match sysfs, and
  the bus rules by the path forms the kernel gives.

- ✅ **Wiring diagram: zoom and fit** – the diagram had one box size, so a filesystem with many
  unfolded folders ran past the bottom of the window. The layout now takes a scale: four levels
  (full, compact, small, and one borderless line a node), each with its own box size, gaps and
  row spacing. The default, fit, picks the largest level whose canvas fits the window's width and
  height, and falls back to the smallest if none does; `+`/`-` choose a level by hand and `0`
  returns to fit. The renderer reports the body height to the state so keys and clicks use the
  same layout as the drawing.

- ✅ **Wiring diagram: unfolding the folders (COA A)** – the diagram no longer stops at the mount
  point. A filesystem, and then any folder, unfolds into its subfolders as shorter boxes in the
  next columns to the right, joined to it by the same lines, so one picture runs from the PCIe
  slot to the folder you are in. `→`/`l` unfolds the node chosen and, once it is open, goes into its
  first folder; `←`/`h` folds it and, when it is folded, goes back to what it is under; a click
  chooses a node and unfolds or folds it; `Enter` opens a folder (or the "+N more" note's folder) in
  the browser, so a click now explores and `Enter` goes there, where a second click used to open;
  `p` on a folder sends the carried files into that folder, with the fit check of the filesystem it
  is on (closing the "sending goes to the root" gap). Opening the view unfolds every folder from the
  filesystem down to the one being browsed and chooses it, once for each folder browsed, so "you are
  here" is a path with its own bright box. Folders live apart from the hardware (`wiring_folders`):
  the hardware is rebuilt from the machine every second and the folders outlast it, and a copy of
  the hardware with the open folders added (`composed`) is what is laid out, drawn and moved
  through. A folder is read on the blocking pool when it is opened, with a "loading…" note until it
  arrives and a "cannot read: …" note if it fails; only folders are listed, hidden ones only when
  the browser shows them, and a folder with more than twelve shows the first twelve and "+N more",
  except that the one on the way to the folder being browsed is always among them. Closing a folder
  forgets what was read in it and below it, so opening it again shows the disk as it is then. The
  layout gained boxes of different heights (a folder is a frame round one line), keeps a tall box
  from crowding what follows it, and decides between columns and the tree by whether the hardware
  fits, not the folders, which scroll sideways: the view keeps the chosen box and the column of what
  is under it on screen. Tried in the real app opened in this repository's folder: it unfolded from
  `/` through `home`, `Desktop`, `Projects` (15 folders), `Programming` (46 folders), `minuteman`
  to `crates` (12 folders), `h`/`l` folded and unfolded, and `Enter` on the first folder moved the
  browser there. Chosen as COA A over a side tray of folder columns (B) and unfolding only the
  path to the current folder (C), with C's unfolding of the path as the starting state.

- ✅ **Atlas — a zoomable map of storage (`V`, COA A)** – the third idea from the *Devices view*
  note, after the deck (B) and the wiring diagram. A new last view in the `V` cycle (`view =
  "atlas"`) that takes the whole body: one block per volume, its area following capacity and its
  fill, from the bottom, following how full it is (danger colour from 90%, dim when not mounted),
  with a `◆` on the volume holding the browsed folder. `Enter` or a second click zooms into the
  chosen block; a volume then shows its biggest folders as blocks sized by what is inside them, a
  folder shows its contents, and a file's block opens its folder in the browser; `Backspace`/`Esc`
  zooms out a level, arrows (and `hjkl`) move to the nearest block, `Tab`/`Shift+Tab` walk through
  them biggest first, `o` opens the chosen block's folder in the browser, `a` switches between size
  on disk and apparent size, `r` looks again. Three new modules in `tui`: `atlas_layout` (a pure
  treemap on whole terminal cells, plus the moves between tiles), `atlas` (state, keys, clicks) and
  `atlas_view` (layout of the regions, colours, labels, drawing). The folder levels do not scan
  anything new: the atlas holds a `DiskUsageView` for the volume zoomed into, which already scans
  one folder at a time on the blocking pool, cancels when dropped, sorts biggest first and keeps a
  stack of levels, so zooming out is instant and nothing holds a whole tree. The volumes come from
  the `devices` crate, looked at every two seconds on the blocking pool only while the view shows.
  Chosen as COA A (lazy zoom on the existing scanner) over a cached whole-volume tree with a corner
  minimap (B, too big for one cycle) and a volumes-only map (C, which is level one of A). The
  layout is a balanced binary split (the entries, largest first, are cut into two groups of about
  equal weight and the map across its longer side, with a cell counted as twice as tall as wide),
  not a squarified one: cutting whole cells at integer positions is what makes "every tile is at
  least one cell, none overlap and together they cover the map" true by construction, which the
  property tests check for any sizes and any window, rather than true unless rounding disagrees. A
  group worth fewer than 6 cells of the map is folded into one `+N smaller` block that is drawn but
  never chosen, so `Tab` (which visits every block with a tile, tested) is the guarantee that
  nothing else is out of reach. Property-tested: tiles partition the map exactly, every sized
  entry is on it exactly once, a block of its own deserves its cells, every move lands on a
  different block strictly in the direction pressed, cycling reaches them all, the regions of the
  view never overlap or leave the window, a volume's free part shrinks as it fills, and drawing
  never panics for any window size or number of volumes. Verified against the real binary in a PTY
  read through `pyte`, with `HOME` pointed at a scratch folder: the map showed this machine's
  volume (`System`, btrfs, 74% of 382G) with the frame and the pin; `Enter` zoomed into `/` with
  the rows arriving while the header said `scanning`; `Enter` again went into `/var` (`lib/` 82%,
  `cache/` 17%); `Backspace` came back out to `/` and a second one to the map; `o` on `var/` moved
  the browser to `/var` and the `◆` onto `var/`; a real SGR mouse click on the chosen volume zoomed
  in. Not tried in a real terminal emulator, nor with more than one volume on the real machine (the
  tests cover several).

- ✅ **Wiring diagram fills the window (COA A: fit by budget)** – with a deep path (nine folders
  down, as in this repository) the diagram was far bigger than the window: one column for every
  level, and every unfolded sibling stacked down the page. Only the hardware was held to the
  window, so the view panned to the chosen box and the hardware scrolled off the left; and any box
  not wholly inside the window was left out while its lines were still drawn, so some things
  showed and others did not. Now, once the body's size is known (`WiringState::set_size`, called
  every turn by `main` before `poll`), `wiring_layout::fit` chooses what is drawn so that all of it
  fits both ways. It tries a cap on the subfolders each folder shows (12, 8, 5, then 3; the rest
  are the existing "+N more" note) and, for each, the four zoom levels largest first, squeezing the
  boxes' width to the columns in the window (never below 10 cells, never past one and a half times
  what the level draws them at, so a wide window is used too) and falling back to the indented tree
  if the columns cannot fit; the first combination whose whole canvas fits the width and height
  wins. Which siblings survive a cut is decided by distance: the folder on the way to the browsed
  folder and the folder chosen in the diagram are always kept (so a cursor is never trimmed from
  under you after a resize), then their nearest siblings, so a folder with 46 subfolders shows the
  twelve around the one you are in and not the first twelve. `+`/`-` still pick a level by hand
  and `0` returns to fit; both refit. If nothing fits even at the smallest cap and level (a very
  short window), the smallest boxes that fit the width are used and the view scrolls, as before.
  Two further fixes in the drawing: a box cut by the window's edge is now drawn cut (it is painted
  into an off-screen buffer and the visible part copied over) instead of being left out, and the
  tree's name column is sized to its longest row, up to three fifths of the width, where a fixed
  40 cells cut the deepest folders to `…`. Chosen as COA A over a layout that depends on the
  selection (B) and only drawing clipped boxes with the hardware pinned (C). Property-tested: for
  any window, depth, number of siblings and zoom the fitted layout holds the whole diagram inside
  the window whenever any cap and level could, and otherwise uses the fewest folders and the
  smallest boxes; boxes never overlap; every node of the trimmed topology is laid out; a squeezed
  scale always holds its columns inside the width. Verified against the real binary in a PTY read
  through `pyte`, browsing `crates/tui` of this repository: at 150×40 the whole path, hardware
  included, is on screen as the tree with every name whole; at 230×50 and 190×45 it is on screen as
  boxes joined by lines with the hardware on the left and `tui` on the right, nothing scrolled; at
  120×24 it does not fit and scrolls to the chosen folder, as it always did; arrows, `+` and `0`
  kept it fitting. Not tried through a live resize of a real terminal (the tests resize the state).

- ✅ **Operation keys work on the cursor row, and marks act where you stand** – `y`, `d`, `x` and `X`
  act on the marks when there are any and on the entry under the cursor when there are none (the
  earlier marks-only rule is gone for these four; compress and extract keep it). With marks, `y`
  and `d` copy or move them into the browsed folder at once instead of filling the clipboard for a
  `p`, so the flow is mark, go to the destination, press the key; marks already in that folder are
  skipped for a cut with "already in this folder". The batch goes through the same background paste
  as a drag and drop, so progress, cancel, the conflict prompt and undo are unchanged. The trash and delete confirmations accept `Y` as well
  as `y`. `space` is a leader with no shell pane open as well: `space x` opens the trash browser and
  `space t` the settings popup, whose branch existed but could never be reached because the pending
  leader was cleared every tick when no pane was open. The mini-shell panes are untouched. Verified
  against the real binary in a tmux session: cut two marked files into another folder with `d`, `X`
  then `Y` on the entry under the cursor, `c` clearing a mark, and `space x` opening the trash.

- ✅ **Deck view: the disk tray moves to the left, like a file manager's places panel (COA A)** –
  the strip of framed cards above the columns is now a 26-cell rail beside them: a frame titled
  "devices" holding the devices grouped under Internal, Removable and Network headings, two lines
  each (name; gauge and free space, or "not mounted" and the size, or the "fits"/"needs" verdict
  while files are carried, which dims a device that cannot take them). A `▌` at the row's edge
  marks the folder you are in, the chosen row is filled while the keys are on the rail, a busy
  disk shows a three-bar sparkline beside its name, a plugged-in device grows in a line at a time,
  and `▴`/`▾` on the frame say more is scrolled out (a group's heading stays with its first device
  when scrolled to). The rail is dropped when the body is under 70 cells wide or 6 rows high. The
  layout puts the columns in what is left, a pane gap from the rail (`BrowserLayout::split_columns`
  is the old column split, taken out so both can use it). Keys: `Tab` takes the keys to the rail and gives them back; there the up and down arrows choose,
  `Enter`, `m`, `e` and `p` as before and `Esc` leaves; `h`, `j`, `k` and `l` are the file browser's
  whether or not the rail has the keys. (First cut had `h`/`l` move between rail and files and
  `l` do nothing on files; reverted the next cycle as it broke the muscle memory of every other view.) The sparkline and the mount path are no longer on the device's row (the path is
  in the header once you are in it). Chosen as COA A over the old cards stacked in a column (B)
  and a rail that folds to icons and unfolds on focus (C). Tests: the layout (rail beside the
  columns, gaps, narrow and short windows), the rows (grouping, heights, scrolling, a click on
  each line), the keys (`h`/`l`/`Tab`, `j`/`k`, `l` swallowed, no room for the rail), and drawing
  at any size. Looked at through the test backend; not driven in the real binary this cycle.

- ✅ **File chooser for browsers** – `mman` can answer a browser's open and save dialogs through
  the XDG desktop portal. New crate `portal_backend` (binary `mman-portal`) is a backend for
  `org.freedesktop.impl.portal.FileChooser` (`OpenFile`, `SaveFile`, `SaveFiles`): per dialog it
  opens `mman --chooser-file` in a terminal window (class `mman-chooser`, first of foot, alacritty,
  kitty, wezterm, xterm, or `MINUTEMAN_PORTAL_TERMINAL[_CMD]`), waits, and replies with `file://`
  URIs; the portal's `Close` kills the window. `mman` gained `--chooser-file`, `--pick
  <file|files|directory|save>` and `--pick-name`: `Enter` on a file picks it, `Q` confirms where the
  cursor is, a save asks for the name and before replacing a file; paths on another machine or in
  an archive are refused. `shared::chooser` holds the contract (`PickKind`, and `Selection`, whose
  NUL-separated file format keeps any file name intact). Verified with property tests, an
  end-to-end test over a private D-Bus session (real daemon, stand-in `mman`), and the real `mman`
  driven through a PTY in every pick mode. Not yet tried against a real browser and the real
  `xdg-desktop-portal`; the dialog's file-type filters and extra choices are ignored.

## 🔴 High Priority: a small core and bundled plugins

The goal: the core is only the file operations (create, delete, move, copy, rename, touch, and the
trash they go to) and the terminal interface, and everything else can be replaced by a plugin.
Features move out one at a time and none of them is a rewrite; each move must leave behavior
unchanged. The features that ship by default are plugins installed with the program and enabled
on a fresh install, in the manner of the system add-ons bundled with a browser: a user plugin of
the same name, or one that claims the same action, takes their place, and the config can switch any
of them off.

**Stays in the core:** the `Vfs` trait and local disk, `file_ops`, `trash`, the directory model
(`browser`: listing, navigation, sort, cursor, marks, live refresh), the interface shell (event
loop, panes, popups, mouse, line editing, status bar, `:` command line), the mini-shell panes and
the shell popup, the keymap and config, the plugin host and action registry, minimal theme
tokens, the plain open (`$EDITOR` or `xdg-open`), and the small terminal plumbing (cd-on-exit,
OSC 52). The portal backend stays its own program on top of the core.

**Layout:** `crates/` holds the core only; `plugins/` holds one crate per bundled plugin;
`crates/plugin_host` is the host (renamed from `crates/plugins` so the two names stop colliding)
and `crates/plugin_api` holds the wire types and a client helper that the host and every plugin
share. Nothing in `crates/` may depend on anything in `plugins/`; a test over `cargo metadata`
enforces it. Installed, bundled plugins live in `<prefix>/share/minuteman/plugins/<name>/` and user
plugins in `~/.config/minuteman/plugins/<name>/`.

**Rules for every move:** `scripts/vfs-gate` keeps its allow-list entry with the file and scans
`plugins/*/src`; a build with no plugins installed still browses and runs the file operations (the
test that the core stands alone); the plugin draws nothing itself, it describes and the interface
draws.

- [x] **M1 – scaffold, no behavior change.** `crates/plugins` is now `crates/plugin_host`;
  `crates/plugin_api` holds the plain wire data (event names, `EventKind`, `JobOutcome`, `Change`,
  `Job`, `StateSnapshot`, `LogLevel`, `WireEntry`), re-exported from the host under their old paths;
  the root `plugins/` folder exists (the first plugin crate adds itself to the workspace `members`
  in M2, since Cargo refuses a `plugins/*` glob that matches nothing); the `dependency_direction`
  test fails if a core crate names a plugin, if `plugin_api` grows a dependency other than
  `shared`, or if a bundled plugin uses anything but `plugin_api` and `shared`; `scripts/vfs-gate`
  now scans `plugins/` too. The client helper and the line-reading half of `protocol.rs` (which
  needs `file_ops` and `crossterm`) stay in the host until M2 gives a plugin something to use them
  for.
- [x] **M2 – picker slot and the fuzzy finder.** A plugin can provide one of the interface's
  actions (`provides = ["find"]` on a `[[plugin]]`; the key then sends it an `action` event
  instead of acting), open a list the interface draws (`show_picker`, then `picker_items` as it
  ranks, `picker_query` and `picker_closed` coming back with a generation number so a stale answer
  is dropped) and have the tree below a folder listed for it through the host's own paths
  (`walk`, streamed as `walk_batch` events, cancellable), so the finder still works over ssh and
  inside archives. The finder is now `plugins/finder`, the program `mman-finder`: it ranks, the
  interface draws the panel and the preview and shows the chosen entry. It is found next to `mman`
  or on `PATH` and starts by itself unless a plugin of the same name, or one providing `find`, is
  configured; when it cannot be found `f` says how to get it, and when it crashes the picker closes
  and the next `f` starts it again. The fuzzy matcher (`Query`) moved to `shared` so the browser's
  search and the plugin score alike, and `plugin_api` gained the shapes and a line helper for a
  Rust plugin. Timed on 120,300 entries in a release build: the index completes in 0.66 s against
  0.68 s before, and a typed query is ranked 0.04 s after the last letter against 0.08 s.
  Follow-ups: starting a bundled plugin only on first use instead of at startup, a config switch
  to turn a bundled plugin off without writing a replacement, `preview = false` pickers have only
  been laid out and not tried by a plugin, and a chosen result always means "show it in the
  browser" (a plugin cannot yet say what choosing does).
- [x] **M3 – structured search (`L`) on the same picker slot.** `/` (the nearest-level fuzzy
  search) and the `f` finder are unchanged: `/` is a fundamental part of the core and `f` is the
  one bundled plugin. M3 adds a second bundled plugin, `plugins/search` (`mman-search`, providing
  the new `locate` action, bound to `L`), for what neither does: every entry below the folder that
  satisfies all the words typed. A word is a plain piece of the path (case ignored unless it has a
  capital), a glob (`*.rs`, `src/**.rs`), `re:` and a regular expression, `size:>1M` or
  `size:1k..2M`, `mtime:<7d` (changed less than that long ago) or `mtime:>1w`, and `type:f` or
  `type:d`. Results come nearest level first with the same preview as `f`, and a word that cannot
  be read is set aside and named in the status bar while the rest still applies. The wire grew
  two optional fields, `TreeEntry::size` and `modified`, and `WalkRequest::metadata` (off unless
  asked for, so the finder's lines are byte-identical), fed from the listing the walk already
  reads. Timed on 200,000 entries in a release build: 1 to 15 ms in all, the longest slice of
  2,048 entries under 0.2 ms. Content search (grep) was left for its own milestone, since it needs
  a way for a plugin to read file bodies through the host.
  Follow-ups: the status bar flashes `ignoring size:` while a filter is still being typed; no
  highlight of what matched; words cannot contain spaces (no quoting); `re:` is case sensitive and
  there is no smart-case switch for it; sorting is nearest level first only; the driver loop
  (`run.rs`) is a copy of the finder's and should move into `plugin_api` once a third plugin needs
  it; and a plugin cannot yet say what choosing a result does.
- [x] **M4a – the `set_view` primitive, and git status on it.** A plugin fills a named slot of the
  interface with `set_view`: a document of lines of spans, each with a tone (`normal`, `accent`,
  `danger`, `dim`) and an optional glyph by name, which the interface draws in the user's glyph set
  and palette and drops when the bar is too narrow. The plugin never sends a color or a symbol. The
  first two slots are `status_entry` (a word about the selected entry) and `status_right` (a line
  about the folder). A slot has one owner, the plugin that filled it last, and only its owner can
  empty it; a plugin that stops loses its slots at once. The host cleans every view as it reads it
  (control and direction-changing characters removed, at most one line per status slot, 64 spans,
  512 characters), since the text reaches the terminal. `tui::git_status` is now `plugins/git`
  (`mman-git`), which follows `dir_changed` and `selection_changed`, asks `get_state` once at start
  for the cursor (so `client::Incoming::Reply` now carries `result`), runs the same `git status`
  with the same safeguards (no optional locks, killed after 10 s, cancelled on leaving the
  repository, re-run 3 s after the last run ended) on a thread per run, and sends a view only when
  it differs from the last. Chosen as COA B (one view type for segments and panes) over a
  status-segment-only message (A) and moving the preview first (C). The state word on the left now
  trails the cursor by one round trip to the plugin, which is a few milliseconds; the old code asked
  git's answer in the same frame. Without `mman-git` the status bar has no git segment and
  everything else runs; `git_status = false` keeps it from starting. A bundled plugin that provides
  no action is replaced only by one of the same name. Verified against the real `mman` over a PTY
  read through `pyte`, in a scratch repository: the branch and `~1 ?1` showed, a clean file had no
  state word, a modified file said `modified`, an untracked one `untracked`; `:cd` out of the
  repository dropped both segments and `:cd` back restored them; killing `mman-git` removed its
  segment; with `git_status = false` no `mman-git` process existed; and an `mman` copied alone
  still browsed.
  Follow-ups: a status block takes one tone (the first span's), so per-span tones show only once
  a slot is a pane; an unfound `mman-git` is silent (the finder and search say how to get theirs,
  but nothing is pressed here to say it on); slots are not yet reachable by a user plugin's
  config.
- [x] **M4b – more slots, then the pane:** `header_pill` (marked size, the system HUD) and
  `entry_badge` (a mark in the list for git), then `preview_pane` (text, hex, images, thumbnails)
  and inspect, which need a scrollable multi-line view and a way to hand an image to the interface.
  - [x] `header_pill`, with the system figures (uptime, load, memory) as `plugins/hud`. The marked
    size stays in the core: the paste checks use its total to see whether files fit.
  - [x] `entry_badge`, with git's marks as its first user. A line of the slot's view names its
    entry with a new optional `key`, and the slot is bounded one badge at a time (2,000 lines, a
    two-character badge, a 255-character key) since the per-view totals would refuse any real
    folder. `mman-git` sends the marks of the folder it is in (`+` `~` `?` `!`, a folder merging
    what it holds) and empties them out of a repository; the list reserves 3 cells at the end of
    the name column once any row has a badge. Chosen as COA B (the existing `set_view`, with keyed
    lines) over a separate `set_badges` message (A) and the host asking for the visible rows (C).
    Verified against the real `mman` over a PTY read through `pyte`, in a scratch repository: the
    marks sat beside the right rows and the folder, changed with the folder, vanished with
    `mman-git` and with `git_status = false`, and an `mman` alone browsed without them.
    Follow-ups: badges show in the middle column only (not the parent or preview lists, or the
    grid, tile and filmstrip views); a staged file changed again shows the modified mark only; the
    old folder's marks can sit beside the new names for one round trip after changing folder.
  - [x] A text-only preview plugin (text, hex, archive listing), `plugins/preview`
    (`mman-preview`). The interface names the file the column is about (the new `preview` event,
    sent to the plugin that `provides = ["preview"]`, for the browser's cursor, the trash
    browser's and the finder's alike); the plugin reads it through the host (the new `read_file`
    request, over whatever serves the browsed paths) and answers with a whole finished document
    (`set_preview`): source as lines of pieces in seven token kinds, a binary's first 64 KiB, or
    an archive listing. The interface keeps the scrolling, wrapping, scrollbar, colors and
    typewriter reveal, so `J`/`K` and the wheel never wait on the plugin. Chosen as COA A (a typed
    document, scroll in the interface) over a plugin that owns the scroll and sends only the
    visible rows (B) and a slot with the built-in preview kept as a fallback (C). The document is
    a new notification rather than a `set_view` slot because it is not spans of one status line:
    its limits are a megabyte of text, 64 KiB of bytes and 5,000 entries, cleaned on arrival like
    every view. The code that read, tokenized, listed and hex-dumped moved with its tests
    (`syntect`, `zip`, `tar`, `flate2` now belong to the plugin); the file-name rules moved to
    `shared::names`; `[[preview_hook]]` tables of kind `text` run in the plugin. Timed on a 1 MiB
    source file in a release build: 3.5 s to a finished document, of which 3.4 s is `syntect`
    tokenizing (as before the move) and about 0.1 s is the megabyte reaching the interface as a
    4.7 MB line. Verified against the real `mman` over a PTY read through `pyte`: source with its
    keyword and comment colored apart, `J`/`K` four half pages down and two back, a hex dump, zip
    and tar.gz listings, a `pdf` shown as what its hook extracted, a picture still drawn by the
    core, the finder's preview of a file the cursor is not on, the last of a burst of moves being
    the file shown, a change on disk reaching the column, a killed plugin ending "loading
    preview…" and starting again for the next file, and an `mman` alone browsing with a notice
    saying how to get the plugin.
    Follow-ups: the whole document is tokenized before any of it is sent, so a file near the cap
    shows "loading preview…" for seconds (sending plain text first and the colors after would
    fix it); a document is one message, so a megabyte of text is a line of several megabytes for
    the host to parse; `read_file` lets a plugin read anything the user can, as a plugin always
    could; the trash browser's preview goes through the same code but was not tried in the PTY; a
    plugin of the user's own that provides `preview` replaces this one, and its contract is only
    what `plugin_api` says.
  - [x] The picture in the preview column, with `[[preview_hook]]` tables of kind `image`.
    `set_preview` gained an `image` document that carries no pixels: its source is either
    `original` (the previewed file is itself a picture, which the interface reads through the same
    paths as before, so a browsed ssh folder still previews) or `handoff` (the plugin ran an image
    hook, which left a PNG in the temporary folder under a name `plugin_api::handoff_path` defines,
    and the interface decodes it once and deletes it). `plugins/preview` now decides which files
    are pictures and runs the image hooks (`--image-hook <json>` beside `--text-hook`), so
    `tui::preview_hook` is gone; the interface keeps decoding, fitting to the column and drawing
    (`image_preview`, on `ratatui-image`, since it owns the terminal's graphics capabilities).
    Chosen as COA B (hand over a file) over pixels on the wire (A) and leaving the whole image
    path in the core (C). A plugin is not trusted to name a file the interface then deletes: the
    host refuses a handoff that is not directly in the temporary folder under the scratch prefix,
    the decoder checks it again and refuses a link, and a handoff for a file the cursor has left is
    deleted by whichever side drops it. Without `mman-preview` a picture shows only its name, as
    text already did. Property-tested: a handoff is accepted exactly when its name is a scratch
    name directly under the temporary folder, whatever string a plugin sends. Verified against the
    real `mman` over a PTY read through `pyte`: a PNG drew its color, a `.xyz` file turned into a
    different color by an image hook drew that one, a hook that fails drew only the name, a text
    file beside them still drew text, moving back to the first picture redrew it, no scratch file
    was left in the temporary folder, and an `mman` copied alone showed names and the notice.
    Follow-ups: a hook's picture is made again each time the file is asked for (no cache); a
    picture is still decoded by the interface, so a very large image costs the interface that
    decode on a blocking thread, as before; the filmstrip, grid and tile views keep their own
    thumbnail cache in the core (they are alternate views, M7); not tried in a real kitty.
  - [x] Inspect, as `plugins/inspect` (`mman-inspect`). The interface names the entry the panel
    opens on (the new `inspect` event, sent to the plugin that `provides = ["inspect"]`, and again
    with `null` when the panel closes); the plugin describes it with the new `stat` request, which
    the host answers from whatever serves the path (`symlink_metadata`, `read_link`, the owner and
    group ids and a `local_path` that says whether they are this machine's), starts a folder's
    count with the new `tally` request, and sends the rows as `set_inspect`, at once with
    "counting…" and again when the count lands. `tally` returns at once and ends with a
    `tally_done` event (cancelled by `tally_cancel`), like `walk`, because a count that takes
    seconds must not hold up the plugin's other requests or its own cancel. The count is the same
    `tally_dir` the marked-size pill uses (moved to `shared::tally`), so a folder's panel and the
    header's total are one number. The row wording, the owner lookup and the count labels moved
    with the plugin; `format_size`, `mode_string`, `group_thousands` and the time formatting moved
    to `shared::format` since the lists and the plugin both print them. `tui` keeps only the
    panel's state (`InspectPanel`: the entry, the last rows, a "loading…" placeholder) and its
    drawing. Chosen as COA A (a `stat` request and an `inspect` document) over a plugin that only
    formats what the core gathered (B) and a generic multi-line pane view (C). Rows are cleaned
    on arrival (control and direction characters removed, at most 32 rows, 32-character labels,
    1,024-character values). Without `mman-inspect`, "Inspect" says how to get it and opens
    nothing; an entry that cannot be described shows an `Error` row instead of a status-line
    message; a plugin that dies before answering ends the "loading…". Property-tested: nothing
    unsafe or over-long survives cleaning, and cleaning twice changes nothing. Verified against the
    real `mman` over a PTY read through `pyte`: a file's panel (type, size, permissions, owner), a
    folder's count (`3 files, 1 folders, 18B`), a link, a broken link and reopening, and an `mman`
    copied alone saying where to get the plugin and still browsing.
    Follow-ups: the status line cuts the "how to get it" message at 120 columns; a folder shows
    "counting…" with no progress until the count ends; `largest` (the biggest file) is not on the
    wire, as nothing in the panel used it; not tried in a real kitty, nor on an ssh folder or inside
    an archive (the host's `stat` is covered by tests on a memory backend). The thumbnail cache
    (`thumbnails`) goes with the alternate views in M7.
- [x] **M5 – compress and extract as a plugin (`plugins/archive`, `mman-archive`).** Scope
  changed from the original line, by decision: the trash (the browser, restore, empty and moving a
  file to it) **and undo stay in the core**, because the trash is one of the file operations the
  core is defined by and an undone copy goes to it; moving undo out would have put its "never lose
  data" rules across a process boundary. M5 is archives alone. The interface names the work (the
  `archive` event, sent to the plugin that `provides = ["archive"]`: a compress of one or more jobs,
  or an extract) and keeps the forms, the password and overwrite prompts, the progress gauge, `Esc`
  and `c` to cancel, and the trash; the plugin does it on a thread of its own on the local disk and
  answers with `archive_progress` (the entry reached and a running total, thinned to about 25 a
  second) and one `archive_done`: completed, cancelled, failed in words, `password_needed`,
  `wrong_password`, plus the indices of the jobs that wrote something. The interface sends the
  originals (or the archive) of those jobs to the trash afterward, so a failure still never costs a
  file its only copy. An extract with no password finds out from the plugin before anything is
  written, which also removes the read of the archive the interface did on its render thread. The
  code (`file_ops::archive`, with its zip-slip and zip-bomb guards, AES-256 and about 40 tests)
  moved unchanged except for its error type; `Format`, `Level`, `Conflict` and the wiped-on-drop
  `Password` moved to `plugin_api` so both sides name the same types, and `mkdir -p` moved to
  `shared::create_dir_all` since a plugin cannot use `file_ops`. Chosen as COA A (run the unchanged
  code against `LocalVfs` on local paths) over serving the nine `Vfs` calls the code makes to the
  plugin over the wire (B, which is M7's design problem done early) and keeping the code in the
  core for remote paths behind a feature (C, which duplicates it). **Accepted regression:**
  compressing or extracting where a path is on another machine or inside an archive is refused by
  name until M7 gives a plugin files through the host. Verified against the real `mman` and
  `mman-archive` over a PTY read through `pyte`, with `tar` and `7z` reading the results: a folder
  packed to a real tar.gz and unpacked over an existing folder after the overwrite question, a
  sealed zip asked for twice and opened by `7z` with the password and refused with another, the
  extract asking for its password, refusing a wrong one without writing anything and extracting
  with the right one, `Esc` stopping a 40,000-file compress with no archive or temporary file left,
  the plugin killed mid-request ending the operation with a message after a second and starting
  again for the next request, and an `mman` copied alone saying how to get the plugin.
  Follow-ups: ssh paths (M7; inside an archive it is refused by design, archives being read-only); a plugin killed mid-request cannot remove the hidden
  temporary file it was writing; the password reaches the plugin as one JSON line, and only the
  `Password` type is wiped, not the line or the parser's copies; cancel is checked per entry, so
  one very large file runs to its end first; not tried in a real kitty.
- [x] **M6 – the devices rail as a plugin (`plugins/devices`, `mman-devices`).** Scope changed from
  the original line, by decision: the rail of devices only. Places (home, the XDG folders,
  bookmarks) is left for later, and disk usage stays in the core because the atlas draws on its
  scanner; wiring and atlas keep reading the machine themselves until M7. The first check, how far
  the deck is tied to the browser keys, found it is not: `DeckState::key` is a self-contained
  state machine (the rail takes `Tab`, the arrows, `Enter`, `m`, `e`, `p` and `Esc` only while it
  has focus; `h/j/k/l` were never its). What tied it down was the data and the actions. The plugin
  now owns both: while the rail shows, the interface sends `devices` with `watch`, and the plugin
  looks at the mounts, `statvfs`, udev and `/proc/diskstats` on a thread of its own and sends the
  whole list about once a second (`set_devices`: every device with its read and write rate and a
  short history, and whether an action is under way). The interface asks it to `open`, `toggle` or
  `eject` a device by its node and it answers with a `devices_note` (a message, or a folder to go
  to). The interface keeps what is its own: the selection, the focus and keys, the slide-in of a
  device that has just appeared, the drawing, and whether carried files fit (`cargo_fit`, so `p`
  is still refused with its reason before any copy). Text from the plugin is cleaned on arrival,
  a device with no node or a node already listed is dropped (the selection follows a node), the
  list is cut to 256 and a mount path that could act on the terminal counts as no mount. To let a
  plugin use the code, the `devices` crate moved into `shared::devices` (a plugin may depend on
  `plugin_api` and `shared` only), and what to do to a device (`Plan`: open, toggle, eject, eject a
  whole disk) moved into `shared::devices::plan`, so the plugin and the wiring diagram run the same
  steps. Without the plugin the rail says "mman-devices was not found" and the status line says how
  to get it; a plugin that dies is started again, at most once in three seconds. Chosen as COA A
  (a rail fed by a plugin, disk usage left) over moving the whole storage cluster (B, which is M7's
  pane design done early) and a cargo feature (C, which makes nothing replaceable). Verified
  against the real `mman` and `mman-devices` over a PTY read through `pyte`: the rail listed this
  machine's root volume with its gauge and the `▌` mark, `e` on it was refused in words, the
  plugin killed with `SIGTERM` showed its exit and came back within the grace, and an `mman`
  copied alone showed the note and the install hint.
  Follow-ups: no mount, unmount or eject was run against a real stick or a network share (this
  machine had neither; the steps are the code that ran before); the rail's list is replaced
  whole each second and the plugin keeps running (idle) after the rail goes; wiring and atlas
  still carry their own scan until M7; the status line cuts the install hint at the terminal's
  width.
- [x] **M7a – browsing inside an archive is served by the archive plugin:** the host sends the
  plugin `vfs` requests (`metadata`, `list`, `scan`, `read_link`, `open`, `read`, `close`) for each
  path inside an archive and the plugin answers; `vfs_archive` is now only the `Vfs` that routes
  those paths (`ServeLink` turns a synchronous call into a request and waits on a channel), and
  the reading of zip, tar and tar.gz (the index, the codecs, the engine) moved into
  `mman-archive`. A file is read through a handle the plugin keeps, at most 32 at a time; a
  handle is tied to the plugin's process by an epoch, so one from a dead process can never reach a
  restarted one. Archives stay read-only. This settles the question M7 posed for archives: the
  process boundary under a synchronous call is a bounded wait (120 s, ended at once if the plugin
  dies) and a window of the file read per round trip, and it holds. Verified against the real
  `mman` and `mman-archive` over a PTY read through `pyte`: a zip opened as a folder with a
  folder and a file listed, the file's text shown in the preview column, `Q` leaving the browser
  inside `t.zip!`. Follow-ups: the missing-plugin message on `mman` copied alone was not read
  off the screen (a unit test covers its words); no kill of the plugin mid-listing was tried on
  the real binary; compress and extract inside an archive stay refused (they write).
- [ ] **M7 – decide, then do, for what is left:** the `ssh` scheme served by a plugin, and the
  alternate views (grid, tiles, filmstrip, atlas, wiring), which need `SetPane` to describe more
  than text spans. If either proves too costly, it stays in the core behind a cargo feature.
  - [ ] **M7b – `ssh` served by a plugin (`plugins/ssh`, `mman-ssh`).** Chosen over keeping
    `vfs_ssh` in the core behind a feature: Minuteman is a better Ranger, ranger has no native
    ssh browsing, and the async stack (`tokio`, `openssh`, `openssh-sftp-client`) is the largest
    dependency in the core for a minority of sessions. Staged so remote writes never regress:
    1. [x] move the address parsing (`ssh_path`) into `shared`, so the interface tells a remote
       path apart without the SSH stack; add the `vfs-ssh` word, the write requests on the wire
       and a second `ServeLink`. Behaviour unchanged.
    2. [x] `mman-ssh` (`plugins/ssh`, package `remoter`) serves reads and writes, and a host-side
       `SshVfs` (new core crate `vfs_remote`) routes `ssh://` paths to it; built and tested but
       not wired in, so `main.rs` still uses `RoutedVfs` and behavior is unchanged. The plugin is
       the `vfs_ssh` engine (session, executor, error mapping, SFTP streams) copied across, with a
       `Server` that turns each `vfs` request into a call on the session for the machine its path
       names, and keeps at most 32 files open (read or write, never both) under handles; each
       request runs on its own thread and the plugin ends once the host closes its input and the
       running ones have replied. `SshVfs` reads in doubling windows (`PluginReader`, as for
       archives) and writes in gathered 256 KiB windows (`PluginWriter`: a seek that does not
       continue the last write, `flush` and drop send what it holds); a copy between this disk and
       a machine, or two machines, is streamed through it, and a move between them is refused with
       `CrossesDevices`. `IoKind` gained `NotConnected`, `BrokenPipe`, `TimedOut` and `Unsupported`
       so a refused host key or a dropped connection keeps its meaning across the wire. The
       connection options of a plugin started by a test come from `MMAN_SSH_ACCEPT_NEW`,
       `MMAN_SSH_KNOWN_HOSTS` and `MMAN_SSH_IDENTITY`; a user sets none of them. Verified by the
       whole `Vfs` conformance suite and the cross-machine scenarios of `vfs_ssh` run through
       `SshVfs`, the real `mman-ssh` program and a real loopback `sshd` (9 scenarios, plus the
       plugin's own against the same server and a property test of writes by offset), and by
       `scripts/check`. **Measured** (256 MiB, release, loopback `sshd`, a stand-in host rather
       than the real `ServeLink`): download 20 MiB/s through the plugin against 16 MiB/s
       in-process (0.78x the time), upload 23 MiB/s against 56 MiB/s (2.4x the time). The plugin's
       engine with no wire does 69 up and 25 down, so the cost is the wire, about 7 ms per window:
       3 ms of JSON and hex, the rest pipes and a stop-and-wait round trip. Open for step 3:
       whether that upload figure is acceptable (it is a ceiling near 35 MiB/s, so it matters on a
       fast link and not on a slow one), then binary frames for `read`/`write` or sending the next
       window before the reply to the last, measured again through the real host; a half-written
       file when the plugin dies mid-copy is removed by the copy and not by the plugin.
       **Upload decided: pipelined.** `Peer::submit` returns a `Reply` to wait on later, and the
       writer keeps four 256 KiB windows in flight (`flush` and drop wait for all, and the close
       follows them). Chosen over accepting the figure and over binary frames. Measured the same
       way: upload 109 MiB/s through the plugin against 56 in-process (0.52x the time, was 2.4x),
       download unchanged; through the harness's process stand-in, so step 3 measures once more
       through the real `ServeLink`. Tested with a lagging peer (overlap bounded at five, close
       after the last write, a refused write reported by `flush`) and `ServeLink` answering two
       requests in flight to their own callers.
    3. [ ] switch `main.rs` from `RoutedVfs` to `SshVfs`, delete `vfs_ssh` and its dependencies
       from the core, add the missing-plugin message. From here ssh needs `mman-ssh` installed.
  - [ ] **M7c – the alternate views** (decided after M7b, with the cargo-feature fallback open).

---

## 🟡 Medium Priority (Important)

- **Archives, remainder** – `rar` (extract-only, proprietary), `7z`, and `xz`/`zstd`/`bzip2`
  compression are not supported, the codecs being opt-in cargo features if wanted; extraction does not restore modified times (only writing a zip records them, as UTC, since
  the standard library has no time zone database); the compress form's "Save in" folder must
  already exist and the form never overwrites (only `:compress` and `:extract` ask); the
  central-directory check refuses any zip whose last 64 KiB holds a record that claims too much,
  which a hand-made comment can trigger on an otherwise fine archive; and a per-item compress
  (one archive for each) warns about the first name already taken, not all of them.
- **Encrypted archives, follow-ups** – hiding file names needs 7z, which is not supported; an encrypted zip that uses a method
  this build cannot read is reported as damaged rather than as unsupported encryption; one password
  is checked for a whole batch (by the first encrypted entry of each archive), so an archive made
  elsewhere with a different password per entry fails on that entry during extraction; the password
  is asked for again after a wrong one but there is no limit on tries; legacy ZipCrypto archives
  can be opened (they are common) but are never written; and encrypted archives cannot be
  previewed beyond their listing.
- **Undo history, follow-ups** – undo runs on the render thread, so taking back a large
  cross-device move stalls the interface until it finishes (a background job with progress and
  cancel would fix it); there is no history popup listing the stack to pick from; a compress that
  trashed its originals, an extract, a permanent delete and an overwrite are not recorded (holding
  deleted and overwritten data in an app-owned area would make them undoable, at the cost of
  `X` no longer being immediate); a cancelled paste that had already finished its current item
  still records it, but one interrupted mid-item records nothing for that item; on platforms
  whose trash cannot be read back (macOS) a trash is not undoable; and the history is not kept
  between sessions.
- **Archive browsing, follow-ups** – an archive inside an archive is a file of the outer one and
  cannot be entered; nothing can be changed inside an archive (adding, deleting or renaming an
  entry would mean rewriting it); a file is held in memory when opened (256 MiB at most), so a large
  entry is copied out by extracting it; an encrypted zip lists but its entries cannot be opened
  from inside (extract it, which asks for the password); `rar`, `7z`, `xz`, `zstd` and `bzip2`
  are not served; indexing a large `.tar.gz` no longer freezes the interface (opening it is a
  background move) but is still read whole, so a very large one is slow to open; a zip link is shown as the file holding its text;
  the parent column does not highlight the archive's row; inside an archive the owner, link count
  and device are blank, as the format has none.
- **VFS, follow-ups** – archives are now read and written through `Vfs` and browsed as folders, but
  there is no copy between two different backends beyond the local-to-remote streaming.
  The remaining direct users of the disk are listed, each with its
  reason, in `scripts/vfs-gate`: the desktop trash, git, launching programs, the desktop's MIME
  and application files, this machine's `/proc`, the app's own config and the start directory.
  Owner names are looked up in this machine's `passwd` only for a path on its disk, so another
  backend's owners show as numbers until it can supply names.

---

## 🟢 Low Priority (Nice-to-Have)

- **Grid and details, follow-ups** – details columns cannot be resized; the grid's tile size follows the window height and is not configurable; the arrows and `h`/`l`
  mean different things in the grid than elsewhere and are fixed rather than bindable; a tile has
  no icon for a folder or a non-image beyond its text, even with the Nerd glyph set; drag and drop
  does nothing from the grid (its tiles are not rows `drop_target` knows); the grid has no
  scrollbar; and a file an `Image`-kind `[[preview_hook]]`
  covers gets a placeholder in a tile, not its thumbnail.
- **Filmstrip, follow-ups** – a thumbnail's file is decoded in full, so a folder of large photos
  costs a decode each and a decode already started for an entry that has since left the window is
  not cancelled (a scaled JPEG decode and a cancel flag would fix both); a file an `Image`-kind
  `[[preview_hook]]` covers (a PDF) has a big preview but a placeholder in the strip; right-click, drag and drop and the `Ctrl`/range clicks do
  nothing in this view (only the keyboard marks and acts); `h`/`l` still leave and enter folders,
  so there is no key for scrolling the strip sideways by a page; on a remote machine every
  thumbnail is fetched whole over the connection; and the strip's picture is drawn with the
  terminal's own graphics protocol, which was not tried in a real kitty.
- **Mouse, stage 2 follow-ups** – `Ctrl`/range clicks in the left column act as plain clicks; the
  keyboard's `v` does not set the range anchor; a range click past the edge of the view does not
  scroll it; there is no rubber-band selection by dragging; the `…` in the breadcrumb could jump
  to the nearest hidden ancestor.
- **Drag and drop, follow-ups** – no drop onto a segment of the header's path; no drag out of the
  left column or from the disk usage view; a drag cannot start on a `Ctrl`- or range-click (those
  mark), so copy is `Ctrl` at the drop rather than at the press; the label is not styled by kind
  and can cover the row being aimed at when hovering the top border; auto-scroll moves the
  selection with the view and does not restore it; dropping on the shell types paths but cannot
  choose bracketed paste; a drop of files whose names are not valid text into a shell skips them.
- **Open with, follow-ups** – a stale or missing `mimeinfo.cache` is trusted as it is (reading each
  `.desktop` file's `MimeType=` would cover an app installed since); file types come from names and
  a text-or-not check, not from content signatures (`magic`), so a mislabeled file is opened as its
  name says; the marked set opens as one file rather than passing every path to a program that
  takes several (`%F`); desktop actions (`[Desktop Action …]`) and `%i` icons are ignored;
  "Open with" cannot set the default (writing `mimeapps.list`) or remember the last choice; the
  associations are read once per session, so an app installed while it runs is not seen; `l` and
  `→` on a file now open it, which someone used to browsing with them may find eager.
- **Remote browsing, follow-ups** – opening a folder no longer blocks the interface, but a move
  already running cannot be interrupted at the connection (`Esc` abandons it and the connect runs
  to its 10 second timeout on its own thread), and the live refresh, a preview and the search walk
  still do their reading on their own; there is no password or key-passphrase prompt (keys and the agent only, run
  through `BatchMode`), no prompt to trust a new host key (run `ssh host` once), no bookmarks or
  history of machines, and no way to open a remote file with a local program except copying it
  first. A shell pane, a `:` command and the trash are refused in a remote folder. A copy passes
  through this machine even between two folders on one remote (the server's `copy-data` extension
  is not used), and the disk usage view and recursive delete cost one round trip per folder.
  SFTP v3 gives times in whole seconds and no inode, link count or allocated size, so Inspect
  leaves those out. `ssh://host/~` is not expanded. The `openssh` crate is Unix only, and this was
  not built or tried on Windows.
- **Plugin system, stage 2 — WASM plugin host (Extism), sandboxed, opt-in** – a second, sandboxed
  plugin tier alongside the stage-1 out-of-process JSON-RPC host (see Completed): a plugin
  compiled to WASM gets no ambient filesystem/network/process access by default, only what a
  versioned host API (`read_dir`, `get_selection`, `spawn_preview`, keybind registration,
  lifecycle hooks) grants it explicitly — the tier worth having once a community plugin registry
  means running code nobody local wrote. Rust, Go/TinyGo, Zig and other WASM-target languages
  first; Python/JS need a heavier bundled runtime; assembly and COBOL have no realistic path
  (stage 1 still covers any language, including those). Work: a `PluginBackend` trait splitting
  the process host from a WASM backend; host functions reusing the serde types in `protocol.rs`;
  a per-plugin permission manifest checked after canonicalizing every path (so `..` and symlink
  escapes fail); memory and time limits so a plugin cannot hang the interface; an API version
  handshake; a worked Rust example plugin; and escape-attempt, infinite-loop and
  unbounded-allocation tests. First cut read-only plus `log` and `get_selection`, write
  operations in a second. Kept behind a cargo feature and initialized only when a WASM plugin is
  configured: wasmtime would roughly double the binary (an estimated +8 to 15 MB over today's
  ~13.9 MB release build), `wasmi` about +1 to 2 MB but slower — spike both before choosing.
  Deferred until plugins have users asking for hooks and someone wants to share a plugin with
  others; building it earlier is guessing at an API that must then be kept stable.
- **Action registry, as its own cycle (optional)** – the string names and `run_action` entry
  point now exist for plugins (see the sixth stage 1.5 cycle above); what remains is a `disabled_actions` config list, useful without any plugin
  (uniform keys, menus and `:` commands, easier tests). Pure addition: the existing `match` on
  `Action` stays and is routed through it, with a test first proving every default key still
  triggers what it did. Actions that open prompts or wait for a second key (`BookmarkJump`,
  `Sort`, `Create`) need the prompt split from the operation to take arguments.
- **Optional Lua scripting tier** – lightweight `mlua`-based, in-process scripting for config/
  keybindings/simple commands, layered alongside the two process-based plugin tiers above for
  latency-sensitive hooks that don't need "any language."
- **Menu and Inspect polish** – Inspect could total the marked set and follow a symlink to its target's details. The menu could grow
  keyboard-first access (a key to open it on the selection), per-item shortcut hints, and a
  configurable item list.
- **Git status, follow-ups** – the segment refreshes every three seconds, so a `:git commit` or a
  commit made in a mini-shell shows up after up to that long; it could refresh at once when the
  live refresh sees a change or a `:` command ends. Also missing: the stash count, ignored files,
  a per-file column in the listing (the state shows for the selection only), and a marked-set
  total that refreshes when a marked file grows (it is taken when the marks change).
- **Preview polish** – the scroll keys are not in the status bar's hints; the hex view has no
  jump to an offset or search; archives with `xz`, `zstd`, `bzip2` or `7z` compression and zip64
  archives (over 65,535 entries or 4 GiB) show as bytes; a text over 1 MiB says `preview failed`
  rather than showing its start; a scrolled 1 MiB text re-wraps everything above the visible rows on
  each frame (about 43 ms at the far end), which caching wrapped lines would remove; and the
  scroll position is not remembered per file.
- **Disk usage, follow-ups** – delete or mark from inside the view (the `x` and `v` flows exist
  but the view has no way to hand a selection to them); keep the scanned tree so opening a folder is
  instant instead of a rescan; sort by name or entry count; show modified times and the owner; an
  option for the starting measure; key hints that follow rebound keys (they are fixed text, and
  `a`, `r`, `PageUp`, `PageDown`, `Home` and `End` are not configurable); refresh when the live
  refresh sees a change.
- **Search, follow-ups** – a match deeper than the nearest level that has any is never offered,
  however good (the walk answers with the first level that matches); only the best 20 are kept;
  stepping works while the prompt is open but not after `Enter` (`n` is Create, so a further key
  would be needed); the scoring weights are hand-tuned, not learned from use; a query is matched
  against the name only, not the path (`src/main` does not find `src/main.rs`); and there is no
  smart-case switch or exact-match mode.
- **Fuzzy finder, follow-ups** – the letters that matched are not highlighted in a result (the
  scorer returns a score, not positions); the index skips only hidden entries, not what a
  `.gitignore` names, so `target/` and `node_modules/` count toward the 200,000-entry limit and
  can crowd out what is wanted (they are walked last, being deep); `Enter` shows the entry and does
  not open it, and a directory result is not entered; no multi-select into the marks; no other
  sources (directories only, file contents, recent folders, marks, plugin actions); results are
  capped at 200 listed; the index is rebuilt each time the finder opens, with no cache between
  openings; no smart-case or exact-match switch; the `/` search still matches the name only.
- **Devices view, `V` (B and C delivered, see the done list; A remains)** – a view in the `V` cycle that knows about storage:
  internal disks, USB sticks and network mounts, with their capacity, state and activity, and
  mounting, unmounting and ejecting. Nothing in the codebase reads mounts or block devices yet.
  Shared data layer for all three ideas: `/proc/self/mountinfo` for what is mounted, `statvfs` (via
  `libc`) for capacity, `/sys/class/block` and the udev database under `/run/udev/data` for label,
  filesystem and the removable flag (readable without root, and it covers a device that is plugged
  in but not mounted), `/proc/diskstats` for activity, and `udisksctl` for mount, unmount and
  power-off (it needs no root; without it the view lists devices and cannot change them). Polled
  off the render thread, since `statvfs` on a hung network mount can block. Linux only to begin
  with. Three ideas were weighed, and B was chosen:
  - **A, places rail** – a left sidebar with Devices, Places (home, the XDG folders, bookmarks,
    trash) and Remote (ssh hosts), beside the columns or the grid, with one-key jump labels and
    capacity gauges drawn in cells. The familiar Dolphin/Thunar layout, easiest to learn, least new.
  - **B, storage deck (chosen)** – a strip of live drive cards across the top and the ordinary file
    view below. A card shows the name, filesystem, a capacity gauge, free space and a read/write
    sparkline from `/proc/diskstats`; the card holding the browsed folder is lit as "you are here";
    a newly plugged device slides in; the cards are the targets for mount, eject and, later,
    copy-to. Neither Ranger nor the graphical managers show which drive is busy.
  - **C, atlas (delivered, see the done list)** – a zoomable map of all storage: a block per volume sized by capacity and shaded
    by use, the current location as a pin, selecting a block zooms into its top folders and then
    the file list, and a corner minimap in the normal views shows where you are. The most original
    and the most useful for "where did my space go", but it needs a treemap layout and a
    background size scan below the device level (the disk usage scanner can feed it) with caching.
    The deck's data layer is the one it would need, so it can follow as another view.
- **Atlas, follow-ups** – the corner minimap in the other views, the cache of a whole volume's tree
  and a persistent size history are not built (zooming into a big volume rescans it each time, and
  leaving drops the scan; on this machine the root volume took minutes to finish, during which the
  first folder to finish fills the whole map); the layout is a balanced split, not a squarified
  one, so some blocks are long and thin; a `+N smaller` block cannot be chosen or opened, so the
  smallest entries are reached only through the browser or the disk usage view; a file's block
  opens its folder with the cursor at the top and does not reveal the file; an unmounted volume says
  to mount it from the deck and cannot be mounted from the atlas; there is no right-click menu,
  marking, deleting or sending from the atlas, and the wheel does nothing; labels need a block at
  least three cells wide and three tall, and a smaller one is just colour; colours are the theme's
  foreground colours used as backgrounds, with a dark gray fallback for named-colour themes where
  no darker shade exists; the volumes are scanned separately from the deck's (a second `statvfs` a
  second apart while both are alive, which they never are); no macOS data source.
- **Devices view, next ideas (planned)** – one further way for storage to react to what the user
  is doing, building on the deck and its `devices` crate:
  - *Runway* – each card shows how long until it fills, from a small history of its usage kept
    between sessions (`full in ~41 days`), and the folder that grew most this month. Needs a
    history file and a rule for how much history is enough before it will guess.
- **Wiring diagram, folders, follow-ups** – only folders are shown, never files (the browser is for
  those); twelve is the most a folder shows (a small window shows 8, 5 or 3) and the note cannot be
  unfolded in place (`Enter` opens that folder in the browser); a folder whose name is longer than the box is cut, and the boxes are squeezed to as little as 10
  cells in a crowded window, where the tree is the alternative; the fit prefers smaller boxes with
  every sibling up to the cap over larger boxes with fewer, so a deep path in a short window is drawn
  as one line a box; the open folders
  are forgotten when the view is left; what was read is not refreshed while a folder stays open,
  only when it is closed and opened again; a mount point inside a folder (a `/boot` under `/`) is
  listed as a folder and also drawn as its own filesystem; the folders are not marked or sized, so
  there is no way to see which one is large (the disk usage view is for that).
- **Wiring diagram, follow-ups** – a node with several parents shows under the first only (the
  others are not drawn as lines); the layers LVM and LUKS can be seen but not opened, unlocked or
  closed from the diagram; no SMART health, model details beyond the name, or speed of the bus;
  btrfs across several devices is shown as separate filesystems; a window too narrow for the columns
  loses the lines and the marks (the tree only prints the rates); the diagram has no zoom or pan,
  only the scroll that follows the chosen node; the marks move at a fixed pace whatever the rate, so
  they show that data is moving and not how much.
- **Devices view, follow-ups** – sending goes to the device's root, with no choice of folder (the
  folder open in the browser, or a favourite, would be the next step), and dropping files on a
  card with the mouse does not send them; network shares can be opened but not mounted or unmounted from
  the deck (`udisksctl` only handles block devices); no encrypted-container unlock (LUKS shows as
  not a filesystem and is left out); no temperature or SMART health on a card; no optical drives
  (`sr*` is skipped); the card colors follow the theme but cannot be set on their own; macOS has no
  data source; the deck exists only in its own view, not as a strip over the others; the scan
  repeats every second whether or not anything changed (udev events would make it instant and
  free); a mount or eject reports in the status line only, with no progress for a slow unmount.
- **Fuzzy finder, no limit (planned)** – lift the 16-level and 200,000-entry caps without bringing
  back the stutter. The render thread already never touches the index, so what remains is making a
  very large one cheap to hold, to build and to rank. In order: (1) walk with `Vfs::scan_dir` (one
  call per folder, each entry's own metadata, no symlink-following) in place of `list_dir`, and
  replace the depth cap, which is what stops symlink loops today, with a set of visited
  `(device, inode)` pairs, so `/proc/self/root` cannot lead back to `/`; (2) store the index as one
  shared string arena of relative paths with a small `(offset, length, flags)` record per entry,
  about 70 bytes each against about 150 now, and the path rebuilt from the root when needed; (3) keep
  a bounded best-200 heap per ranking level and only the matching positions (4 bytes), so a snapshot
  costs the same at ten million matches as at two hundred; (4) rank on several cores, each slice a
  range of the index split across `std::thread::scope` workers; (5) show progress (`ranking… 38%`)
  during a long first pass; (6) walk on several threads sharing one queue of folders, capped at half
  the cores and at 8, with ties between equal scores broken by depth then path so a run is
  deterministic. Later: keep the finished index between openings and update it with the live refresh.
  Estimated against the 200,000-entry benchmark (`crates/browser/tests/finder_cost.rs`): the first
  letter scales linearly, about 5 s for five million entries on one core and under 1 s on eight, and
  every later letter narrows from the previous matches. Open questions: whether the SFTP backend
  lets requests overlap (a parallel walk helps a remote machine only if it does), and a thread count
  that suits a spinning disk, where more threads can mean more seeking. Measure after each step.
- **Trash browser, follow-ups** – no sort order other than newest first, no search or filter, no
  preview below 100 columns or on a platform whose trash layout is not the freedesktop one, a
  picture is never scaled up in the pane (as in the browser's own column), no mouse marking (a click selects the row; `v` marks it), restore
  cannot choose another place, and the size of a folder is its entry count, not its total. The
  trash of another disk (`.Trash-<uid>`) is listed with the home trash and treated the same.
  On macOS the trash cannot be read back, so the view says so.
- **Prompt editing, follow-ups** – the password prompts (extract, compress) still only append and
  delete the last character, with no cursor; the `/`, `:` and rename prompts have no history of
  what was typed before, no `Tab` completion of a path, and no selection or paste of more than a
  typed line; paging keys are not rebindable.
- **Sorting, follow-ups** – the order lasts for the session only (like the view, it could be saved
  to `local.toml`); it cannot be set per folder; folders cannot be listed after files or mixed in;
  there is no natural ordering of numbers in names (`f2` before `f10`); and an owner or extension
  key is not offered.
- **Event-driven refresh (inotify) instead of polling** – the live refresh re-lists the browsed
  directory twice a second, which costs one `readdir` plus a `stat` per entry and can lag a
  change by about half a second. Worth replacing with `notify` only if that is noticeable in very
  large directories; it would need a watch method on `Vfs` so a remote backend can offer its own.
- **Show all of a `:` command's output** – the status line holds eight lines for five seconds.
  A scrollable output pane (or a pager) would make `:ls -l` or `:git status` usable.
- **Cancel a `:` command's whole process tree** – `Esc` kills the `sh` it started but not a
  process that shell already forked into the background.
- **Animated border gradient (COA C from the gradient cycle)** – the gradient slowly turning
  around the frame on the existing animation tick, with a speed setting. Belongs on a future
  Motion page with the other animations (focus fade, typewriter preview, splash), each with an
  on/off and a speed, like a desktop's animation settings. Costs a redraw on every tick even when
  idle, so it must be off by default and stop while the terminal is unfocused.
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
- **Still not planned: "everything is a plugin" as one rewrite** – the browser, the file
  operations, the trash and the interface shell stay in the core. The earlier objection (a plugin
  API big enough to express the whole app, serialization on the hot paths, a core plugin's failure
  becoming a core failure) is why selected features move out one at a time under "A small core and
  bundled plugins", each measured, and why the core is not one of them.

---

## 🎯 Next Actions (Immediate)

0. `cargo install --path crates/tui` (installs one executable, `mman`; make sure `~/.cargo/bin`
   is on your `PATH`) and `cargo install --path plugins/finder` (the fuzzy finder, `mman-finder`,
   which `f` now needs next to it; from a checkout `cargo build -p finder` once does it), and
   `cargo install --path plugins/git` (`mman-git`, which the status bar's git segment now needs;
   from a checkout `cargo build -p git`), and `cargo install --path plugins/preview`
   (`mman-preview`, which the preview column's text, hex dump, archive listing and pictures now
   need; from
   a checkout `cargo build -p previewer`), and `cargo install --path plugins/inspect`
   (`mman-inspect`, which the Inspect panel now needs; from a checkout `cargo build -p inspector`),
   and `cargo install --path plugins/archive` (`mman-archive`, which compress and extract now need;
   from a checkout `cargo build -p archiver`).
   Add `eval "$(mman init zsh)"` to `~/.zshrc`, reopen the terminal, run
   `mman`, navigate somewhere, and press `Q` — your shell should follow; `q` should not.
1. Nothing new to try by hand this cycle: `mman-ssh` is built but not wired in, and `mman` still
   browses `ssh://` machines in-process. Its tests are the check: `cargo test -p remoter -p
   vfs_remote` (they start a real `sshd` on a loopback port and skip themselves without one), and
   `cargo test -p vfs_remote --release --test throughput -- --ignored --nocapture` for the timing
   (upload is now about twice the in-process rate).
   Before that: `cargo run -p tui` — mark a folder with `v` and `:compress x.tar.gz` (the header
   shows the gauge; `Esc` stops it), then mark the archive and `:extract`; `:compress -p s.zip` asks
   twice for a password and `:extract` of that zip asks once and says `wrong password` for a bad
   one; with `mman-archive` missing each says where to get it, and on an `ssh://` folder each is
   refused for now. Before that: right-click a file and pick Inspect (then a folder: its row
   says "counting…" and fills in), and `Esc` closes it; with `mman-inspect` missing it says where to
   get it. Before that: `mman --chooser-file /tmp/pick --pick save --pick-name x.txt`, press `Q`, `Enter`,
   and `cat /tmp/pick` (the saved path; `--pick files` with `v` marks and `Q` picks several; `q` leaves it empty).
   To try it from a browser, see "File chooser for browsers" in the README. Before that: mark two files with `v`, go into another folder and press `d`
   (they move there; `y` copies), then press `x` or `X` with nothing marked to delete the row under
   the cursor (`y` or `Y` confirms), `c` to cancel, and `space` then `x` for the trash. Before that: keep pressing `V` until the title says `atlas` (it is the last
   view before the list): a block per volume, the one holding your folder marked `◆`; `Enter` zooms
   into it (watch the folders arrive, biggest first), arrows or `Tab` move between blocks, `Enter`
   again goes deeper, `Backspace` comes out, `o` opens a block's folder in the browser, `a` swaps
   size on disk for apparent size. Before that: press `V` in a folder of images and keep pressing it: first a
   grid of tiles (arrows, `j`/`k` to move, a click to select, a double-click to open, the wheel to
   scroll), then a flat details table (name, size, modified time, permissions), then the filmstrip,
   then back to the columns. Before that, the filmstrip on its own: the selected
   image fills the window with a strip of its neighbours underneath; `j`/`k` or a click on a
   thumbnail moves along it, the wheel over the strip moves three at a time, `V` goes back to the
   list. Before that — put the cursor on a `.zip`, `.tar` or `.tar.gz` and press `enter`
   (`l`, a double-click or right-click Open work too): it opens as a folder, header `…› a.zip!`.
   Browse it, `J`/`K` through a file's preview, `v` mark files and `y`, then `h` out and `p` into a
   real folder to copy them out; `h` from the archive's root lands on the archive's own row. Try
   `x` or `:mkdir` inside it for the read-only message. Before that — the HUD (header, size/age columns, scrollbar, powerline status bar) and an
   adaptive theme are the default (`COLORTERM=truecolor` for full color; the theme now follows
   your terminal's own background — Catppuccin Mocha or Latte — unless `[theme]` is set in
   `appearance.toml`, e.g. `name = "neon"` for the original cyberpunk look or `name = "classic"`
   for the old plain one). Press `Alt+n` to open a shell and type in it (`Tab` completes, spaces work).
   `Alt` (tap) stops typing; then `space |` splits it side by side, `space h`/`l` moves between panes,
   `space space` goes back to typing, `space x` closes a pane. `space t` flips a split, and
   `space r`/`space m` plus `hjkl` resize/move (`Esc` leaves either mode).
   Try the new `Alt` layer too: `Alt+n` splits, `Alt+hjkl` moves the box, `Alt+a`/`Alt+d`
   grow it left/up, `Alt+f`/`Alt+s` shrink it wide/tall, `Alt+zxcv` changes pane,
   `Alt+t`/`Alt+b` snap to the top/bottom, `Alt+m`/`Alt+q` close one/all, and `Alt`+drag with
   the left/right button moves/resizes. Tapping `Alt` alone switches between the shell and the
   browser. If the mouse gestures do nothing, your window manager is probably grabbing
   `Alt`+drag. If accents come out wrong inside a mini-shell, set `alt_tap = false`.
   Try the mouse too: click a row in the middle column, double-click a directory, click a row
   in the left column to go up, scroll the wheel over either. Clicking the browser while a
   mini-shell is focused gives the keyboard back to the browser. Set `browser_mouse = false` in
   `config.toml` if you would rather leave the mouse to the shell box.
   Then, from the menu cycle: right-click a file for its menu (hover "Open with", pick Inspect), right-click
   a folder and Cut/Copy elsewhere then "Paste into folder", right-click empty space for the
   directory menu, and double-click a file to open it. Put `[[open_with]]` entries in
   `config.toml` (see `config.example.toml`) to fill the "Open with" submenu.
   Press `.` to show or hide dot-files. Try `:mkdir -p a/b`, `:touch x.txt` and `:ls -l` — the
   new entries appear at once, and so does a file you create from a mini-shell or another
   terminal, with no key pressed. `:sleep 30` shows a BUSY pill and `Esc` kills it.
   Then the newest: `a` (or "Appearance…" on blank space's right-click menu) for the appearance
   popup — `j`/`k` moves, `enter` or a click acts on the row under the cursor: Theme cycles the
   base palette (neon, classic, dracula, catppuccin, nord); Accent, Focused border, Selection,
   Directory, Status bar and Danger are typed as a name or hex (`enter` to type, `enter` again to
   confirm, `Esc` to cancel, with a live preview as you type); Border style, Separator and Glyphs
   cycle with `h`/`l`/`enter`/a click; Reset to defaults clears every change back to what was in
   effect when the popup opened. `Esc`/`q` closes it. Every change here, and every one the
   settings popup makes, is saved at once to `~/.config/minuteman/local.toml` and survives a
   restart.
   Before that: `space t` with no shell pane open, for the settings popup (`j`/`k` moves,
   `h`/`l`/`enter` cycles Columns, HUD and Command bar, `Esc`/`q` closes it) — try switching to
   two-pane and hiding the HUD or the command bar (open a `:` prompt while it's hidden and it
   comes back for the prompt).
   Before that: `u` for the disk usage view (`enter` a folder, `h` back, `a` apparent size,
   `q` closes it), and the same from a folder's right-click menu.
   Before that: `J`/`K` (or the wheel over the right column) scroll the preview of a long text
   file; select a binary for its hex dump, and a `.zip`, `.tar` or `.tar.gz` for its listing.
   Before that, the git segment: open a repository (the status bar shows the branch, and `v` a
   few files for the header's total size; `git_status = false` turns the git segment off).
   Before that, four earlier ones: `/` plus the start of a name a few directories down (`Esc` returns,
   `Enter` stays); the arrow keys; `v` two files, `d`, go to another directory, `c` drops the cut
   and the marks; and `:nvim ROADMAP.md` (`:!cmd` for a program not in `interactive_commands`).
   Then: select a `.rs`, `.py`, `.toml` or other recognized source/config file — the preview now
   colors keywords, strings, comments, numbers, functions and types instead of one flat color,
   using the active theme's own `syntax_*_fg` palette; an unrecognized extension still previews,
   just uncolored.
   Then: add a `[[preview_hook]]` to `config.toml` (see `config.example.toml` for a real
   `pdftoppm`/`ffmpegthumbnailer` example) and select a file with that extension — it thumbnails
   or extracts text through your own command instead of falling back to a hex dump; a missing
   tool, a failing command or a timeout just shows the file's name, quietly.
   Then: press `Alt+n` to open a shell, then `Alt+n` again to split it — the new pane's border fades in from
   the dim frame color while the one that just lost focus fades back out, instead of either one
   snapping instantly; `Alt+z`/`x`/`c`/`v` moves focus between panes with the same fade each time.
   Then: with `boot_splash = true`, a brief splash names the app over the browser — press any key to
   skip it, or just wait under a second. The current (middle) pane's border and title now sweep
   through a gradient between two theme colors instead of one flat shade. Select a longer text
   file and watch it stream in from the top instead of appearing all at once (scroll it, or wait —
   either way it settles). Set `system_hud = true` in `config.toml` for a header segment showing
   this session's uptime, load average and memory (Linux only; shows nothing elsewhere). Any of
   the four has its own `config.toml` switch (`boot_splash`, `gradient_borders`,
   `typewriter_preview`, `system_hud`) if you'd rather it not.
   Then the newest: turn Caps Lock on — a "caps lock" pill appears in the header at once, in the
   danger color, and disappears the moment it's off again, kept in sync on every key event (even
   while a modal view or a prompt owns the keyboard) so it never lags behind what the next letter
   key will actually do.
   Then the newest: press `B` then a letter (say `a`) in any directory to bookmark it — the status
   bar confirms with `bookmark 'a' set`. Navigate elsewhere, then press `` ` `` then the same
   letter to jump straight back, or a letter nothing was ever saved under to see `no bookmark at
   'x'` instead of an error. Either chord shows `SET BOOKMARK`/`BOOKMARK` in the status bar's mode
   pill while it's waiting for that second key, with `Esc` to back out.
   Then the newest: press `a` for the appearance popup — it
   now opens on five categories instead of one long list. Enter "Theme & colors", move to any
   color and press `Tab` for the picker: click or drag in the square to set saturation and value,
   and in the strip beneath it to set the hue. "Text styles" takes a comma-separated modifier list
   per element (`bold, italic`) and previews it as you type; every commit is saved to `local.toml`.
   Then the newest: save a look with "Save theme", change some colors, then open "Saved themes" —
   `Enter` on a name brings that look back, `r` renames it and `d` (then `y`) deletes it.
   Then the newest: `Ctrl`-click rows to mark them one by one, `Alt`-click (or `Shift`-click, where
   your terminal passes it on) a row further down to mark the span from the last row you clicked,
   click a segment of the path in the header to jump there, and middle-click a folder to open it.
   `mouse_range_modifier` in `config.toml` picks `"either"`, `"shift"` or `"alt"`.
   Then the newest: press a file row and drag it onto a folder in the middle or left column, or
   onto blank space in the left column to move it up a level; hold `Ctrl` as you let go to copy;
   `Ctrl`-click a few rows first and drag one of them to carry the lot. Drag onto a mini-shell pane
   to type the quoted paths into it, and hold the pointer at the top or bottom border of the
   middle column to scroll. `Esc` cancels.
   Then the newest: put `[[open_rule]]` entries in `config.toml` (`match = ["md"]`, `command =
   "nvim"`), then press `enter` on a file — it opens with your rule, or with the program your
   desktop has for its type (a `Terminal=true` one takes the whole terminal). Right-click a file and
   hover "Open with": your rules, then every installed program for that type, then "Other…" to
   type a command.
2. `scripts/check` (format check, clippy and the whole workspace's tests — new this cycle; see
   DIARY.md) to verify everything still passes.
3. Commit this cycle (step 8 of the dev loop).
4. Next cycle: M7b step 3 (switch `main.rs` to `SshVfs`, delete `vfs_ssh`, add the missing-plugin
   message); the upload figure is settled (pipelined, 109 MiB/s). Medium Priority holds the follow-ups to remote browsing and the other features
   listed there. Folder moves no longer block, so the next biggest gaps are the other things that
   read on the render thread (undo of a large move, a large archive's index) and a prompt for a
   password or a new host key.
