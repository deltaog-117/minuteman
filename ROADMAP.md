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

---

## 🔥 High Priority (Critical)

- **Preview extras, stage 2 — syntax highlighting** – colour source and config files in the text
  preview with `syntect`, using the theme's palette where it can.
- **Preview extras, stage 3 — external previewers for PDF and video** – a config-driven hook
  that runs a user-chosen command (`pdftoppm`, `ffmpegthumbnailer`, ...) off the render thread
  and feeds its image or text into the existing preview pipelines, falling back quietly to the
  file name when the tool is missing or times out.
- **UI overhaul, phase C — cinematic layer** – a boot splash, animated focus transitions,
  gradient borders/titles, a pulsing selection, a typewriter reveal on the preview, and an
  optional system/git HUD. Needs an animation tick on top of the existing 100ms poll.
- **Bookmarks / marks** – jump-to-directory bookmarks (Ranger-style `` ` ``/`m` register) so
  frequently visited paths don't require re-navigating the miller columns each time.

---

## 🟡 Medium Priority (Important)

- **Open-with, remainder: associations and discovery** – Open and Open with exist (the menu,
  double-click, `[[open_with]]`); what is left is choosing the program by MIME type or extension
  from `config.toml` rules, listing the programs actually installed by reading XDG `.desktop`
  files and `mimeinfo.cache` (instead of a hand-written list), `enter` on a file opening it, and
  an "Other..." entry that prompts for a command.
- **Mouse, stage 2 — multi-select and breadcrumb clicks** – `Ctrl`-click toggles a mark and
  `Shift`-click marks a range, the way a desktop file manager selects; a click on a segment of
  the header's path jumps to that directory; a middle-click opens the entry (a folder in place,
  a file with its default program). Needs modifier bits on the click path and a path-segment
  hit-test in `browser_mouse`.
- **Mouse, stage 3 — drag and drop** – drag a row (or the marked set) onto a folder to move it,
  with `Ctrl` held to copy, using the existing `file_ops` paste and conflict flow. Needs drag
  events routed to the browser, a highlighted drop target, and a rule for dropping on the parent
  column, blank space and the shell box. The scrollable preview (wheel over the preview column)
  is under *Preview extras, stage 1* above.
- **Undo history for recent file operations** – a stack of recent copy/move/delete/trash/rename/
  create operations that can be stepped back through. Delete-to-trash itself shipped this cycle
  (see Completed); this is the remaining half of the old "trash + undo history" item.
- **Full appearance editor: every field, with categories (COA B from the appearance-popup
  cycle)** – the appearance popup (see Completed) covers a Theme pick plus six highlight colors,
  border style, separator and glyphs; the rest of `Theme`'s ~19 fields, every `[style]` element's
  bold/italic/… flags, and the font table are still config-file-only (though now that saves are
  wired up to `local.toml`, whatever this editor eventually adds gets persistence for free). The
  fuller design considered at the time: a category submenu (Theme colors / Border & separator /
  Glyphs / Styles / Font) reusing `ContextMenu`'s hover/click/submenu machinery, with the same
  free-text entry the popup already has for a leaf field. Deferred rather than built in the same
  cycle because free-text editing of ~40 fields behind a two-level submenu is a project of its
  own, not a bounded addition to one already-large feature.
- **Saved custom themes: reselect, rename, delete** – the color picker and "Save theme" row (see
  Completed) can create and update named themes in `local.toml`, but the Theme row's cycle still
  only knows the five fixed built-in palettes — there's no way yet to pick a previously saved
  custom theme back up, rename one, or delete one, from inside the popup. Needs the Theme row's
  `RowKind::Cycle(&'static [&'static str])` to grow into (or sit alongside) something that can
  cycle a dynamic, session-loaded list of names, which is more than the one-line addition it
  sounds like.
- **VFS abstraction hardening** – a `Filesystem`/`Vfs` trait consumed uniformly by browser,
  file_ops, preview, and trash, so backends can be swapped without touching feature code.

---

## 🟢 Low Priority (Nice-to-Have)

- **Native remote filesystem browsing (SSH/SFTP)** – browse and operate on `ssh://`/`sftp://`
  paths directly through the VFS abstraction, no FUSE mount required.
- **Plugin system, stage 2 — WASM plugin host (Extism), sandboxed** – a second, sandboxed plugin
  tier alongside the stage-1 out-of-process JSON-RPC host (see Completed): a plugin compiled to
  WASM gets no ambient filesystem/network/process access by default, only what a versioned host
  API (`read_dir`, `get_selection`, `spawn_preview`, keybind registration, lifecycle hooks) grants
  it explicitly — the tier worth having once a community plugin registry means running code
  nobody local wrote. Rust, Go/TinyGo and other WASM-target languages first; Python/JS need a
  heavier bundled runtime.
- **Plugin system, follow-ups** – a plugin binding more than one key, or a `:plugin <name>`
  manual-trigger command; a crash-restart/supervisor policy (stage 1's plugins get one log line on
  an abnormal exit and are not restarted); a real, non-Rust reference plugin (Python or shell)
  checked into the repo as a worked example, since stage 1's own tests intentionally hand-write
  the wire format rather than reuse the crate's types, but nothing yet exercises an actual
  external interpreter end to end.
- **Optional Lua scripting tier** – lightweight `mlua`-based, in-process scripting for config/
  keybindings/simple commands, layered alongside the two process-based plugin tiers above for
  latency-sensitive hooks that don't need "any language."
- **Menu and Inspect polish** – Inspect times are UTC because the standard library has no time
  zone database; a `chrono`/`jiff` dependency or `TZ` handling would show local time. Inspect
  could also total the marked set, follow a symlink to its target's details, and work on `ssh://`
  paths once a backend exists (owner and on-disk size need a `Vfs` method). The menu could grow
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
- **Disk usage, follow-ups** – delete or mark from inside the view (the `d` and `v` flows exist
  but the view has no way to hand a selection to them); keep the scanned tree so opening a folder is
  instant instead of a rescan; sort by name or entry count; show modified times and the owner; an
  option for the starting measure; key hints that follow rebound keys (they are fixed text, and
  `a`, `r`, `PageUp`, `PageDown`, `Home` and `End` are not configurable); refresh when the live
  refresh sees a change; and a `Vfs` method that reports allocated size, without which the view
  cannot work on `ssh://` paths once a backend exists (it reads `std::fs` metadata directly).
- **Fuzzy ranking for `/` search** – search now finds the nearest substring match anywhere below
  the directory. Still missing: subsequence matching (`aernd` finding `aerend`), ranking by match
  quality rather than depth alone, and stepping through further matches (`n`/`N`, or the arrows
  while the prompt is open).
- **Arrow keys, `PageUp`/`PageDown`, `Home`/`End` in prompts and lists** – arrows browse now, but
  the text prompts have no cursor movement, and paging and jumping to the first or last entry
  have no keys.
- **Event-driven refresh (inotify) instead of polling** – the live refresh re-lists the browsed
  directory twice a second, which costs one `readdir` plus a `stat` per entry and can lag a
  change by about half a second. Worth replacing with `notify` only if that is noticeable in very
  large directories; it would need a watch method on `Vfs` so a remote backend can offer its own.
- **Show all of a `:` command's output** – the status line holds eight lines for five seconds.
  A scrollable output pane (or a pager) would make `:ls -l` or `:git status` usable.
- **Cancel a `:` command's whole process tree** – `Esc` kills the `sh` it started but not a
  process that shell already forked into the background.
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

0. `cargo install --path crates/tui` (installs one executable, `mman`; make sure `~/.cargo/bin`
   is on your `PATH`). Add `eval "$(mman init zsh)"` to `~/.zshrc`, reopen the terminal, run
   `mman`, navigate somewhere, and press `Q` — your shell should follow; `q` should not.
1. `cargo run -p tui` — the HUD (header, size/age columns, scrollbar, powerline status bar) and an
   adaptive theme are the default (`COLORTERM=truecolor` for full color; the theme now follows
   your terminal's own background — Catppuccin Mocha or Latte — unless `[theme]` is set in
   `appearance.toml`, e.g. `name = "neon"` for the original cyberpunk look or `name = "classic"`
   for the old plain one). Press `s` to open a shell and type in it (`Tab` completes, spaces work).
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
   `Enter` stays); the arrow keys; `v` two files, `m`, go to another directory, `c` drops the cut
   and the marks; and `:nvim ROADMAP.md` (`:!cmd` for a program not in `interactive_commands`).
2. `scripts/check` (format check, clippy and the whole workspace's tests — new this cycle; see
   DIARY.md) to verify everything still passes.
3. Commit this cycle (step 8 of the dev loop).
4. Next cycle: preview extras, stage 2 (syntax highlighting), the mouse's stage 2 (multi-select
   and breadcrumb clicks), built-in trash and undo, or persisting the settings popup's changes to
   disk (see Medium Priority).
   Bookmarks (directory bookmarks — distinct from the file marks added earlier) and the preview
   extras' later stages remain the other 🔥 candidates.
