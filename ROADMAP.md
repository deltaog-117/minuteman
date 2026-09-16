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

---

## 🔥 High Priority (Critical)

- **Full theme system** – extend `theming::Theme` beyond the current two-color stub (selection
  bg/fg) into a real palette (borders, headers, file-type colors) with at least one alternate
  theme to prove the system works end-to-end.

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

1. `cargo run -p tui -- <dir>` and press `s` to drop into a shell in the browsed directory;
   `exit` to return — the TUI should resume cleanly and stay fully interactive.
2. `cargo test --workspace` (29 tests) to verify everything still passes.
3. Commit this cycle (step 8 of the dev loop).
4. Pick the next roadmap item — recommended: **full theme system**, the only remaining High
   Priority item; it's self-contained and doesn't block on anything else in flight.
