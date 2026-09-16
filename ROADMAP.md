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

---

## 🔥 High Priority (Critical)

*(none — all High Priority items are done; see Medium Priority below for what's next.)*

---

## 🟡 Medium Priority (Important)

- **Inline image preview** – render images directly in the preview pane via `ratatui-image`
  (Kitty/iTerm2/Sixel graphics protocols), falling back to Unicode blocks when unsupported.
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
- **Multi-tab / multi-pane workspaces.**
- **Shell overlay as an embedded popup terminal emulator** – requested after the shell overlay
  shipped: keep minuteman visible around a smaller, still fully-interactive shell window, instead
  of today's full-screen takeover. This is a meaningfully bigger feature than the current
  overlay — it means minuteman becoming a terminal emulator itself — so it's parked here rather
  than built now. Three approaches considered, no decision made yet:
  - **COA A — Full pty + VT100-parser popup** (e.g. `portable-pty` + `vt100` crates): spawn the
    shell on a new pty sized to the popup, parse its output into a virtual screen buffer, render
    that as a bordered ratatui widget layered over the main UI, forward keystrokes to the pty
    while it's focused. Fully delivers "small window over minuteman" and keeps full
    interactivity (vim/less/ssh all still work inside it). Adds two new dependencies, a
    background I/O thread, and real edge cases (resize propagation, cursor visibility,
    child-crash cleanup, ANSI-parsing correctness). Difficulty: high.
  - **COA B — Constrained-pty takeover without rendering minuteman behind it**: spawn the shell
    on a smaller pty and let it draw directly into a sub-region of the real terminal (offsetting
    its own cursor-position escape codes), without simultaneously rendering minuteman's panes
    behind it. Still needs to intercept/offset the child's escape codes (doesn't avoid ANSI
    parsing), and the "background" isn't actually minuteman, just blank space — a weaker result
    for barely less work than COA A. Difficulty: high, weaker payoff.
  - **COA C — Cosmetic transition only, no true windowing**: keep today's full-screen
    suspend/resume shell exactly as-is, just show a bordered "entering shell…" message before
    suspending. Zero new dependencies, but doesn't deliver an actual popup — the shell still uses
    the whole terminal once it starts.
  - Recommendation when this is picked up: **COA A** — B is strictly worse for similar effort,
    and C doesn't solve the actual ask (a small window, not a cosmetic transition).
- **Multi-select (mark several entries for one bulk op)** – `BrowserState` only tracks a single
  selection today; yank/cut/paste/delete all operate on one entry. `file_ops::ConflictPolicy`
  already has `Skip` (vs. `Abort`) specifically for when a batch needs to continue past one
  conflicting item instead of stopping — that distinction is currently unobservable in the TUI
  since there's never more than one item in flight.
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

1. `cargo run -p tui -- <dir>`, and try `~/.config/minuteman/config.toml` with `[theme]` /
   `name = "dracula"` to see the alternate palette; individual color fields still override it.
2. `cargo test --workspace` (33 tests) to verify everything still passes.
3. Commit this cycle (step 8 of the dev loop).
4. Pick the next roadmap item — every High Priority item is now done, so this is the first cycle
   choosing from 🟡 Medium Priority: inline image preview, built-in trash + undo history, or VFS
   abstraction hardening are all reasonable next picks.
