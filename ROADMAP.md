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

---

## 🔥 High Priority (Critical)

- **Preview extras, stage 1 — scrolling, hex view, archive listing** – the text preview cannot
  scroll yet, and a binary or an archive shows only its file name. Add a scrollable preview
  (wheel over the preview column, plus keys), a hex view for binary files, and a listing for
  `zip`/`tar`/`tar.gz` archives, all in-process and testable in the `preview` crate.
- **Preview extras, stage 2 — syntax highlighting** – colour source and config files in the text
  preview with `syntect`, using the theme's palette where it can.
- **Preview extras, stage 3 — external previewers for PDF and video** – a config-driven hook
  that runs a user-chosen command (`pdftoppm`, `ffmpegthumbnailer`, ...) off the render thread
  and feeds its image or text into the existing preview pipelines, falling back quietly to the
  file name when the tool is missing or times out.
- **UI overhaul, phase C — cinematic layer** – a boot splash, animated focus transitions,
  gradient borders/titles, a pulsing selection, a typewriter reveal on the preview, and an
  optional system/git HUD. Needs an animation tick on top of the existing 100ms poll.
- **Richer status line, remainder** – the HUD now shows permissions, size, type, item count and
  position for the selection; still missing are the cumulative size of the marked entries and
  (where applicable) git status.
- **Bookmarks / marks** – jump-to-directory bookmarks (Ranger-style `` ` ``/`m` register) so
  frequently visited paths don't require re-navigating the miller columns each time.

---

## 🟡 Medium Priority (Important)

- **Open-with / file associations** – open the selected file in the program that suits it:
  `enter` on a file, and double-click now that the mouse is wired up, run an opener chosen by
  MIME type or extension from `config.toml` rules (falling back to `xdg-open`), with an
  "open with..." prompt to pick another. The terminal handover a full-screen opener needs now
  exists (`:nvim`, see above: `Handover`, `TerminalGuard::run_foreground`), so what is left is the
  rules, the prompt, and detaching a GUI opener so it outlives the browser. Until this lands, double-click only opens
  directories.
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
1. `cargo run -p tui` — the new neon theme and HUD (header, size/age columns, scrollbar, powerline
   status bar) are the default (`COLORTERM=truecolor` for full color;
   `name = "classic"` for the old look). Press `s` to open a shell and type in it (`Tab` completes, spaces work).
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
   Press `.` to show or hide dot-files. Try `:mkdir -p a/b`, `:touch x.txt` and `:ls -l` — the
   new entries appear at once, and so does a file you create from a mini-shell or another
   terminal, with no key pressed. `:sleep 30` shows a BUSY pill and `Esc` kills it.
   Then the newest four: `/` plus the start of a name a few directories down (`Esc` returns,
   `Enter` stays); the arrow keys; `v` two files, `m`, go to another directory, `c` drops the cut
   and the marks; and `:nvim ROADMAP.md` (`:!cmd` for a program not in `interactive_commands`).
2. `cargo test --workspace` (all tests) to verify everything still passes.
3. Commit this cycle (step 8 of the dev loop).
4. Next cycle: preview extras, stage 1 (a scrollable preview, hex view and archive listing).
   Richer status line and bookmarks/marks (directory bookmarks — distinct from the file marks
   added earlier) remain the other 🔥 candidates.
