# 📘 DIARY.md

## Architectural Decision Log

*This document serves as a chronological record of architectural decisions, trade-offs, and
reasoning throughout the project's lifecycle.*

---

## 📋 Decision Index

| Date       | Decision Area              | Choice                                      | Status      |
|------------|-----------------------------|----------------------------------------------|-------------|
| 2026-09-16 | Workspace Architecture      | Feature-first Cargo workspace (10 crates)    | ✅ Confirmed |
| 2026-09-16 | Config Keybinding Schema    | Action-to-keys mapping (`Vec<String>` per action) | ✅ Confirmed |
| 2026-09-16 | v0.1.0 Local Vfs            | Synchronous `std::fs`, no `tokio` yet        | ✅ Confirmed |
| 2026-09-16 | Core File Operations        | Extend `Vfs` trait with mutating methods     | ✅ Confirmed |
| 2026-09-16 | Conflict Resolution UX      | `ConflictPolicy` enum in `file_ops`, not `Vfs` | ✅ Confirmed |

---

## 📝 Decision Entries

### Workspace Architecture: Feature-First Cargo Workspace

**Date:** 2026-09-16
**Status:** Confirmed

#### Context / Background

Minuteman needed an initial crate/module layout before any feature code could be written.
`$SUITE/1architecture.md` mandates feature-first organization (top-level dirs named after
business capabilities, not technical layers) and a "delete test" — each feature folder must be
removable without breaking the rest of the app, with zero circular dependencies between
features.

#### Options Considered

**Option A: Single binary crate, modules by layer** (e.g. `ui/`, `fs/`, `config/`)
- Fastest to start, but violates the delete-test and layer-first rules directly; also forces one
  compile unit for the whole app, so iterating on the browser recompiles the (eventual) plugin
  host too.

**Option B: Single binary crate, modules by feature** (e.g. `browser/`, `file_ops/` as `mod`s)
- Satisfies feature-first within one crate, but Rust's visibility rules make cross-feature
  leakage easy to introduce accidentally (no hard compiler boundary), and it doesn't let heavy,
  optional dependencies (SSH, WASM plugin host) be excluded from a minimal build.

**Option C: Cargo workspace, one crate per feature** *(chosen)*
- Each feature is its own crate depending only on `shared`; Cargo itself enforces the "no
  feature depends on another feature" rule at compile time (a stray `browser -> file_ops`
  dependency simply won't compile without explicitly adding it to `Cargo.toml`, which is a
  visible, reviewable change). Optional heavy backends (`vfs_ssh`) can be dropped from the
  workspace member list without touching any other crate — a literal, mechanical delete test.

#### Decision & Rationale

Chose the Cargo workspace split: `shared` (Vfs trait + local impl, read-only infra),
`vfs_ssh`, `browser`, `file_ops`, `preview`, `shell_overlay`, `trash`, `plugins`, `theming`
(each an independent feature crate depending only on `shared`), and `tui` as the thin binary
that wires everything together via dependency injection in `main.rs`.

**Trade-offs accepted:**
- More `Cargo.toml` boilerplate than a single crate.
- Slightly slower clean-build time due to crate-boundary codegen, offset by much faster
  incremental rebuilds once more than one feature exists (changing `browser` doesn't recompile
  `plugins`).

#### Implementation Notes
- `plugins` is deliberately isolated from other feature crates (only depends on `shared`) so
  that internal refactors of `browser`/`file_ops` never force a plugin-facing API break — this
  directly serves the "stable, semver'd plugin API" differentiator from `ROADMAP.md`.
- `tui/src/main.rs` stays thin: arg parsing, config load, `Vfs`/`BrowserState` construction,
  terminal setup — no business logic.

---

### Config Keybinding Schema: Action-to-Keys Mapping

**Date:** 2026-09-16
**Status:** Confirmed

#### Context / Background

The user wanted config loading in place from v0.1.0 rather than hardcoded keys retrofitted
later, and asked which TOML shape to use: one key mapping to one action, or one action mapping
to a list of keys.

#### Options Considered

**Option A: key → action** (`j = "move_down"`) — simplest, but one key can only ever mean one
thing, and multi-key/leader sequences (planned for the space-leader system) don't fit the shape.

**Option B: action → [keys]** (`move_down = ["j"]`) *(chosen)* — one action can have multiple
bound keys, and the shape extends naturally to future leader sequences without a schema change.

#### Decision & Rationale

Chose B. Implemented as `RawKeyMap` (serde-deserialized, `#[serde(default)]` at both the
container and field level via a custom `Default` impl, so a partial `[keys]` table only
overrides the fields it specifies) converted into a `KeyMap` that resolves a crossterm
`KeyCode` to an `Action` via a `HashMap`. `theming` depends directly on `crossterm::event::KeyCode`
rather than inventing a separate key-representation enum — crossterm is already a hard
dependency of the application, and there's no second input backend on the roadmap that would
justify the extra abstraction layer yet.

**Trade-offs accepted:** if two keys in the config resolve to the same `KeyCode`, last-write-in
wins silently (no conflict warning) — acceptable for v0.1.0's small default keymap; worth
revisiting once user-configurable rebinding sees real use.

---

### v0.1.0 Local Vfs: Synchronous, No Async Runtime Yet

**Date:** 2026-09-16
**Status:** Confirmed

#### Context / Background

The project's core motivation is fixing Ranger's slow bulk file operations, which implies an
async runtime (`tokio`) somewhere. The question was whether to introduce it starting with v0.1.0
(directory listing) or defer it.

#### Decision & Rationale

Deferred. A single `read_dir` call for a miller-column pane is not a workload that benefits from
async, and pulling in `tokio` before there's an actual concurrent/long-running operation to
schedule would be unused complexity. `tokio` will be introduced when `file_ops` implements bulk
copy/move/delete with live progress reporting — the point where it earns its place.

---

### Core File Operations: Extend `Vfs` Trait with Mutating Methods

**Date:** 2026-09-16
**Status:** Confirmed

#### Context / Background

`file_ops` needed copy/move/delete/create/rename. The `Vfs` trait was read-only
(`list_dir`/`is_dir`), so the question was where the mutating operations should live relative to
it.

#### Options Considered

**Option A: Extend `Vfs` itself** with mutating methods (`copy_file`, `create_dir`,
`create_file`, `rename`, `remove_file`, `remove_dir_all`, `exists`), implemented for `LocalVfs`;
`file_ops` becomes thin orchestration on top *(chosen)* — one trait boundary, pulls the
already-planned "VFS abstraction hardening" roadmap item forward, keeps `file_ops`
backend-agnostic so it works over `vfs_ssh` once that lands with no changes.

**Option B: Bypass the trait**, have `file_ops` call `std::fs` directly — fastest short-term, but
directly contradicts the project's stated goal and repeats the exact per-backend duplication
problem the workspace-architecture decision already solved for reads.

**Option C: A separate `VfsOps: Vfs` trait**, keeping `Vfs` read-only-only — mirrors `Read`/
`Write`, but doubles the trait every future backend must implement for a safety benefit that
doesn't apply here (nothing constructs a `Vfs` trait object and hands it to untrusted code).

#### Decision & Rationale

Chose A. `file_ops::{copy, mv, delete, create_directory, create_new_file, rename}` all take
`&dyn Vfs` and call only trait methods — no direct `std::fs` in `file_ops` itself.

**Trade-offs accepted:**
- `copy_file` and `rename` in `LocalVfs` manually check `dst.exists()` before calling
  `std::fs::copy`/`std::fs::rename`, since both silently overwrite/replace an existing
  destination. This has a narrow TOCTOU window (another process could create `dst` between the
  check and the actual syscall); acceptable for a single-user desktop file manager, and no worse
  than Ranger/nnn's own behavior. Revisit with `renameat2(..., RENAME_NOREPLACE)` (via a new
  `libc`/`rustix` dependency) if this ever needs to be race-free.
- `mv` has no cross-filesystem fallback yet — a rename across devices just returns the
  underlying io error. Deferred to the async bulk-ops cycle, where a copy+delete fallback
  naturally needs the same progress-reporting plumbing being built for that feature anyway.
- "View" (from the roadmap's "view, copy, move, delete, create, rename" wording) was treated as
  out of scope for `file_ops` — reading file contents for display is `preview`'s job, not a
  mutating filesystem operation.

---

### Conflict Resolution UX: `ConflictPolicy` Enum in `file_ops`, Not `Vfs`

**Date:** 2026-09-16
**Status:** Confirmed

#### Context / Background

Wiring `file_ops` into the TUI needed an overwrite/skip/abort prompt for paste and rename
conflicts, but `copy`/`mv`/`rename` always errored on an existing destination (a deliberate
hardening decision from the previous cycle). See the "Core File Operations" entry above for the
three options considered (extend `file_ops` signatures vs. push overwrite into `Vfs` vs. handle
it ad hoc in the TUI).

#### Decision & Rationale

Chose to add `file_ops::ConflictPolicy` (`Abort`/`Skip`/`Overwrite`) and `file_ops::Outcome`
(`Completed`/`Skipped`), threaded through `copy`/`mv`/`rename`. `Overwrite` is implemented by
deleting the existing destination (via the existing `delete()`) and retrying the same operation,
triggered off the real `VfsError::AlreadyExists` the underlying filesystem call already reports —
no separate `exists()` pre-check, so no new TOCTOU window beyond the one already accepted for
`copy_file`/`rename` in `LocalVfs`. `Vfs` itself did not change.

**Trade-offs accepted:**
- `Skip` and `Abort` behave identically in the current TUI (both leave everything untouched and
  report a status message) because `BrowserState` only supports a single selection — there is no
  batch of remaining files for `Skip` to continue past. The distinction exists in `file_ops` for
  when multi-select/batch paste lands (see ROADMAP's low-priority "Multi-tab / multi-pane
  workspaces" and any future batch-copy work), so that feature won't need another `file_ops`
  signature change.
- The TUI's `app::App` holds clipboard + prompt state as its own module (`tui/src/app.rs`)
  rather than in `browser::BrowserState`, since `browser` is scoped to navigation only (per the
  Workspace Architecture entry above) and clipboard/prompt state is UI-interaction state, not
  navigation state. `main.rs` stays a thin `Action -> App method` dispatcher.
- Verified against the compiled binary (not just unit tests) using two scripted PTY sessions
  driving real key sequences and asserting on actual filesystem end-state, since `cargo test`
  alone can't exercise the terminal event loop, prompt-mode key capture, or the conflict prompt.

---

## 🧠 Usage Guidelines

Write a new entry here before committing to a major design choice (new dependency, new crate
boundary, UI library, etc.), or when revisiting/reversing a previous decision.
