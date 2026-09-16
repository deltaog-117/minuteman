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

---

## 🔥 High Priority (Critical)

- **Core file operations** – view, copy, move, delete, create, rename, on the local filesystem.
- **Async bulk file operations with progress** – tokio + a thread pool driving copy/move/delete
  for large batches, with a live progress UI. This is the core motivation for the whole project
  (Ranger is painfully slow here).
- **Full theme system** – extend `theming::Theme` beyond the current two-color stub (selection
  bg/fg) into a real palette (borders, headers, file-type colors) with at least one alternate
  theme to prove the system works end-to-end.

---

## 🟡 Medium Priority (Important)

- **Interactive shell overlay** – summon a real, fully interactive `$SHELL` subprocess by
  suspending the TUI (not Ranger's auto-close-after-one-command behavior); the user controls
  when it reopens/closes.
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

1. `cargo run -p tui -- <optional-start-dir>` to try v0.1.0 interactively (`j`/`k` move,
   `l`/Enter enter a directory, `h` leave, `q` quit).
2. Commit v0.1.0 (step 8 of the dev loop).
3. Pick the next roadmap item — recommended: **core file operations** in `file_ops`
   (copy/move/delete/create/rename against the `Vfs` trait), since bulk-op speed is the
   project's core motivation.
