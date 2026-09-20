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
| 2026-09-16 | Async Bulk Ops Concurrency  | `spawn_blocking` + poll loop, not a full async rewrite | ✅ Confirmed |
| 2026-09-16 | Shell Overlay Mechanism     | Direct stdio inheritance, no pty multiplexing | ✅ Confirmed |
| 2026-09-16 | Post-Shell Redraw           | `Terminal::resize`, not `Terminal::clear`    | ✅ Confirmed |
| 2026-09-16 | Theme Selection Model       | Named base palette + per-field override layering | ✅ Confirmed |
| 2026-09-16 | Image Preview Concurrency   | `ThreadProtocol` + `spawn_blocking`, not the naive `StatefulProtocol` | ✅ Confirmed |
| 2026-09-17 | Command/Search Bar Mechanism | Extend the existing `Prompt` enum, not a new `Mode` state machine | ✅ Confirmed |
| 2026-09-17 | Text Preview Concurrency    | `spawn_blocking` read, mirroring `ImagePreview`'s decode pipeline | ✅ Confirmed |
| 2026-09-17 | Popup Shell Mechanism       | Full pty (`portable-pty`) + `vt100` parser rendered as a ratatui widget (COA A) | ✅ Confirmed |
| 2026-09-17 | Popup Shell `Esc`-to-Close  | Intercept `Esc` unconditionally rather than forward it, trading away in-popup `vim` `Esc` usage | ✅ Confirmed |
| 2026-09-18 | Marks / Multi-Select Scope  | Wire marks into `Delete` only this cycle, not `Yank`/`Cut`/`Paste` | ✅ Confirmed |
| 2026-09-18 | Leader Key Design           | Inert placeholder `Action`, non-modal — never captures or blocks other keys | ✅ Confirmed |
| 2026-09-18 | Movable/Detachable Popup Shell | Two literal keys (focus-toggle + move-mode), state local to `main.rs::run` (COA A) | ✅ Confirmed |
| 2026-09-18 | Multi-Shell Layout Model | Tmux-style split-pane tree, not multiple floating popups or tabs (COA B) | ✅ Confirmed |
| 2026-09-18 | Pane Mouse Interaction | Divider-drag-to-resize + click-to-focus; no drag-to-reposition or reorder (COA A) | ✅ Confirmed |
| 2026-09-18 | Shell Pane Container Sizing | Centered 80%/70% box, not the full browser area | ✅ Confirmed |
| 2026-09-18 | Shell Box Drag Mechanism | Drag the box's title bar by mouse, offset re-added to `shell_area` (COA A) | ✅ Confirmed |
| 2026-09-18 | Batch Yank/Cut/Paste Continuation | `Clipboard` holds `Vec<PathBuf>`, `poll_bulk` re-spawns the next item itself (COA A) | ✅ Confirmed |
| 2026-09-20 | Alt Layer for Mini-Shell Box | Held `Alt` drives the existing single box; independent floating windows rejected (COA A) | ✅ Confirmed |

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

### Async Bulk Ops Concurrency: `spawn_blocking` + Poll Loop, Not a Full Async Rewrite

**Date:** 2026-09-16
**Status:** Confirmed

#### Context / Background

The project's core motivation — fixing Ranger's slow bulk file operations — meant the whole TUI
needed to stop being purely synchronous. Everything up to this point (`Vfs`, `file_ops`, the
`browser`/`tui` event loop) was blocking `std::fs` on the main thread. Three approaches were
considered (see the "Which concurrency architecture" question resolved with the user this
cycle):

**Option A: `tokio::task::spawn_blocking` + a non-blocking poll loop** *(chosen)* — one
`tokio::runtime::Runtime` constructed in `main`, its `Handle` stored on `tui::app::App`. A bulk
op spawns the existing synchronous `file_ops` walk (now progress-callback-aware) onto tokio's
blocking thread pool; progress flows back over `tokio::sync::mpsc`. `run()`'s loop switched from
a blocking `crossterm::event::read()` to `event::poll(Duration::from_millis(100))`, so it drains
the channel and redraws even with no keypress — this is ratatui's own documented pattern for
background tasks.

**Option B: a fully async event loop** (`#[tokio::main]`, `crossterm::event::EventStream` +
`tokio::select!` for everything) — the "textbook" async TUI architecture, but a full rewrite of
the already-working, already-tested event loop for a payoff (easier future async I/O for
`vfs_ssh`) nothing needs yet.

**Option C: raw `std::thread::spawn` + `std::sync::mpsc`, no tokio at all** — simplest, zero new
dependencies, and sufficient for "one background thread + a channel" since there's no actual
async I/O multiplexing happening (`std::fs` is blocking either way). Rejected because it
contradicts `ROADMAP.md`'s explicit "tokio + a thread pool" wording (written into the plan
before this cycle) and defers the tokio adoption `vfs_ssh`'s real async network I/O will need
eventually — introducing it twice instead of once.

#### Decision & Rationale

Chose A. `spawn_blocking` is the textbook-correct tool for wrapping blocking `std::fs` work
inside a tokio context, and it keeps the blast radius contained: `Vfs`, `browser`, and every
single-item synchronous path (rename, create) are untouched. `file_ops` grew additively
(`copy_with_progress`/`mv_with_progress`; the old `copy`/`mv` became thin wrappers calling them
with a no-op callback), so none of last cycle's 13 tests needed to change.

**Design choices made along the way:**
- **Progress granularity is per-file, not per-byte.** A callback (`&mut dyn FnMut(&Path) ->
  ControlFlow<()>`) fires once per file/directory finished. This needs no change to
  `Vfs::copy_file` (still one atomic `std::fs::copy` call) and avoids a separate counting pass
  over the tree before copying (which would double the directory-listing I/O). A large *single*
  file shows no incremental movement until it's done — logged in `ROADMAP.md`'s low-priority
  list as a follow-up requiring a streaming `Vfs::copy_file`.
- **Cancellation is the progress callback's return value**, not a separate parameter — the
  callback returns `ControlFlow::Break(())` when it observes a shared `AtomicBool` flip (set by
  `Esc`), so one closure carries both "report progress" and "should I stop" without a second
  parameter threaded through every layer.
- **Delete has no per-file progress or cancel.** `Vfs::remove_dir_all` is one opaque
  `std::fs::remove_dir_all` call with no hook to report through; rewriting it into a manual
  recursive walk (like `copy_dir`) just for progress cosmetics wasn't worth it, since deletion
  (unlinking) is normally far cheaper than copying (writing data) — the actual bottleneck
  `ROADMAP.md` names. Delete still runs on `spawn_blocking` so it doesn't freeze the UI, just
  shows an indeterminate "deleting…" status.
- **The cross-filesystem move fallback** (copy + delete when `vfs.rename` fails with
  `ErrorKind::CrossesDevices`, stabilized in Rust 1.83) was implemented this cycle as promised in
  the "Core File Operations" entry above, reusing the same `copy_with_progress` plumbing.
- **While a bulk op is running, all other actions are blocked except `Esc`** (checked before the
  keymap is even resolved) — including quit. Letting the process exit mid-write could leave a
  truncated file at the destination; the user must let the operation finish or cancel it first.
- **Rename stays fully synchronous**, calling the plain `mv` (not `mv_with_progress`) — it always
  targets the same parent directory (same filesystem, same mount), so it's an O(1) metadata
  operation with no cross-device case and nothing worth reporting progress on.
- Verified against the compiled binary (not just `cargo test`) with three scripted PTY sessions
  against a 4000-file directory: cancelling a copy after ~1 file (proving the main loop kept
  processing input while the background thread was writing), running the same copy to
  completion, and deleting the 4000-file tree via the background path.

---

### Shell Overlay Mechanism: Direct Stdio Inheritance, No Pty Multiplexing

**Date:** 2026-09-16
**Status:** Confirmed

#### Context / Background

The roadmap called for a "real, fully interactive `$SHELL` subprocess" via suspending the TUI —
Ranger's own shell command auto-closes after one command, which this project explicitly rejects.
The only real question was mechanism: does the child shell get a brand-new pseudo-terminal that
minuteman manages (a `portable-pty`-style approach), or does it just inherit the process's actual
stdio?

#### Decision & Rationale

Chose direct inheritance: `std::process::Command::new(shell).current_dir(cwd).status()` with
default (inherited) stdin/stdout/stderr, after the caller (`tui`) disables raw mode and leaves
the alternate screen. This works because minuteman is *already* running inside a real terminal —
raw mode is just a mode flag on that same terminal, not a separate pty layer minuteman owns.
Once raw mode is off, handing the exact same file descriptors straight to the child shell makes
it behave exactly like a normal shell session, because as far as the real terminal emulator is
concerned, it is one. A pty-multiplexing approach would only earn its keep if minuteman needed to
capture the shell's output or run something alongside it — neither applies to "get out of the
way until the user types `exit`."

**Implementation:** new `shell_overlay` crate, `pub fn spawn_shell(cwd: &Path) ->
io::Result<ExitStatus>` — reads `$SHELL` (falling back to `/bin/sh`), sets `cwd`, blocks until
exit. It does not touch raw mode/the alternate screen itself; `tui` owns that via
`TerminalGuard::suspend`/`resume`, keeping the crate boundary the same shape as `file_ops`
(does the work) vs. `tui` (owns the terminal).

**Trade-offs accepted:** the child shell has no sandboxing or capability restriction — it runs
with minuteman's own privileges, which is correct for the actual use case (a personal shell
escape, exactly like every other terminal app's `:sh`/`!`) but would need reconsidering before
any future plugin could trigger this path un-prompted.

---

### Post-Shell Redraw: `Terminal::resize`, Not `Terminal::clear`

**Date:** 2026-09-16
**Status:** Confirmed

#### Context / Background

After resuming from the shell overlay, the screen has arbitrary leftover content from whatever
the shell printed, and ratatui's internal diff buffer doesn't know that — the obvious fix is
forcing a full redraw. The first implementation called `Terminal::clear()`, which crashed the
whole app on return from the shell during PTY-based verification testing, with `crossterm`
error: "the cursor position could not be read within a normal duration."

#### Root Cause

`Terminal::clear()` (ratatui-core `terminal/buffers.rs`) calls `self.backend.get_cursor_position()`
*before* clearing, purely to restore the cursor afterward — and `get_cursor_position()`
(`ratatui-crossterm`) shells out to `crossterm::cursor::position()`, which writes a `ESC[6n`
device-status-report escape sequence and blocks reading stdin for the terminal emulator's
response. A real terminal emulator answers this instantly; the PTY-based test harness used to
verify this feature doesn't emulate that protocol, so the read timed out and `clear()` returned
`Err`, which was propagated with `?` — crashing the app immediately after the user's shell
session, the worst possible moment for a crash. This is not purely a test-harness artifact: any
real terminal/multiplexer slow to answer the DSR query (a laggy SSH session, an unusual
emulator) would hit the same failure in production.

#### Decision & Rationale

Switched to `terminal.resize(terminal.size()?.into())` — same current size, so nothing visually
moves, but `Terminal::resize` calls `clear_viewport()` internally (full `ClearType::All` +
back-buffer reset) *without* ever querying cursor position, since that's only needed by `clear()`
to restore the cursor afterward, something not worth doing right before a full redraw anyway.
`terminal.size()` itself just reads the terminal's dimensions via a `TIOCGWINSZ`-style ioctl,
which doesn't require the emulator to answer an escape code and so can't hang the same way.

**Trade-offs accepted:** `resize` is a slightly less obviously-named tool for "force a redraw"
than `clear` — worth a comment at the call site (present in `main.rs`) so a future reader doesn't
"simplify" it back to `clear()` and reintroduce the hang.

---

### Theme Selection Model: Named Base Palette + Per-Field Override Layering

**Date:** 2026-09-16
**Status:** Confirmed

#### Context / Background

`Theme` was a 2-field stub (`selection_bg`/`selection_fg`) with per-field config fallback via
serde's container-level `#[serde(default)]`, the same shape as `RawKeyMap`. Growing it into a
real palette (borders, headers, file-type colors) raised a question `RawKeyMap` never had to
answer: the roadmap explicitly wanted "at least one alternate theme to prove the system works
end-to-end," which a flat set of individually-overridable color fields doesn't really deliver —
proving a full *palette* swap needs more than a README snippet showing seven colors to paste.

#### Options Considered

**Option A: keep the flat per-field-only model**, just with more fields, and document a second
palette as an example TOML block in the README for users to copy. Simplest, but "prove the
system works end-to-end" would rest on documentation, not code — nothing in the binary actually
demonstrates a full theme swap.

**Option B: a named-theme registry with per-field override layering** *(chosen)* — `[theme]
name = "dracula"` selects a built-in base palette; any individually specified color field still
overrides that palette's value for just that field. One config line proves the whole system
swaps correctly, while power users keep the existing fine-grained override behavior on top.

**Option C: theme files** (e.g. a `themes/dracula.toml` users drop into the config directory,
loaded by filename) — matches how some other terminal tools do it, but is meaningfully more
implementation for a project with exactly two built-in palettes today; nothing yet needs
user-authored theme *files* as opposed to a user-authored theme *name* plus overrides.

#### Decision & Rationale

Chose B. Implemented as `RawTheme` (all fields `Option<String>`, including `name`) converting to
`Theme` (all fields always populated) via `Theme::named(name)` as the base, then
`raw.field.unwrap_or(base.field)` per field — the same two-step "resolve a base, then layer
explicit overrides on top" shape used for `ConflictPolicy` resolution in `file_ops`, just applied
to config instead of a filesystem conflict. `Theme::named` falls back to `"default"` for any
unrecognized name, matching the project's standing rule that a config typo must never block
startup (same reasoning as `parse_key`'s silent-skip and `Config::load`'s parse-failure
fallback).

**Trade-offs accepted:** only two built-in palettes exist today (`"default"`, `"dracula"`); a
third would just be another `match` arm in `Theme::named`, so this scales fine short-term but
would want Option C's file-based approach if the built-in set grew large enough to be unwieldy
in a single `match`.

**Verification note:** rather than trusting that TOML parsing + struct fields "should" produce
different rendered colors, the two palettes were checked against the actual compiled binary —
decoding the raw ANSI escape codes from a scripted PTY session confirmed the exact color-index
change expected for every themed element (border, title/directory color, selection highlight,
status bar) between the default and dracula runs.

---

### Image Preview Concurrency: `ThreadProtocol` + `spawn_blocking`, Not Naive `StatefulProtocol`

**Date:** 2026-09-16
**Status:** Confirmed

#### Context / Background

`ratatui-image`'s adaptive `StatefulImage` widget resizes and encodes the image for the
terminal's graphics protocol *inside* its `render()` call. Its own docs are explicit: **"Do not
use it without `ThreadProtocol` in a reactive UI — rendering the widget will block the UI
thread."** This is the same class of problem the project spent all of the async-bulk-ops cycle
eliminating for file operations, so it needed the same scrutiny before writing any code (see the
"Which concurrency architecture" question resolved with the user this cycle for the 3 options
considered: naive blocking `StatefulProtocol`, `ThreadProtocol` + a background worker, or forcing
halfblocks-only to sidestep the cost).

#### Decision & Rationale

Chose `ThreadProtocol` + `tokio::runtime::Handle::spawn_blocking`, reusing the exact pattern
already built and tested for bulk file operations (`tui::app::App`'s `BulkOp`): a channel carries
`ResizeRequest`s out to the blocking pool and `ResizeResponse`s back, drained once per render
tick in `ImagePreview::update`. The *decode* step (`preview::load_image`, a separate blocking
cost `ratatui-image` doesn't manage at all) got the same treatment via a second, smaller channel.
`ThreadProtocol::replace_protocol` (not constructing a fresh `ThreadProtocol` per image) is
important here: it correctly invalidates any in-flight resize request for a since-abandoned
selection via its internal `id` counter, so a stale response can never clobber a newer image if
the user navigates quickly.

**Verification finding (not a code change, but worth recording):** the first attempt at PTY
verification for this feature found navigation keystrokes vanishing intermittently right after
startup. Root cause: `Picker::from_query_stdio()` probes the terminal for Kitty/Sixel/font-size
support by writing several escape-code queries (including a Device Status Report, `\x1b[5n`,
chosen by the crate's own author specifically because — per its source comment — "[DSR is]
implemented by all terminals, ensure there is some response and we don't hang reading forever").
The *main* thread that calls `from_query_stdio()` handles a non-response correctly (falls back to
`fallback_picker` after a 2s timeout, matching this project's established "must never block
startup" rule). The problem is the *background thread* it spawns to do the actual blocking stdin
read: nothing tells it to stop when the main thread times out — a "fire and forget" by the
crate's own design — so it keeps reading (and silently discarding) all subsequent stdin bytes
forever if the terminal never answers even the DSR probe. A synthetic dumb pty (no terminal
emulation at all, used for this project's PTY-based verification) never answers, so it reliably
hit this; a real terminal emulator answers DSR in microseconds, since virtually every terminal
emulator has implemented it since the VT100 era, so this is not a practical concern for actual
users. Documented here rather than worked around, since there is no way to cancel that thread
through the crate's public API, and the realistic risk to real users is effectively nil. The PTY
test harness itself was fixed to emulate a DSR reply, which is what let verification proceed and
confirm the actual feature (not just the harness limitation).

---

### Command/Search Bar Mechanism: Extend the Existing `Prompt` Enum, Not a New `Mode` State Machine

**Date:** 2026-09-17
**Status:** Confirmed

#### Context / Background

The TUI needed a Ranger/lf-style `:`-command bar and `/`-incremental-search, the first two of
three TUI-polish items (alongside bookmarks/marks) identified as high priority. Three options
were considered: (A) add `SearchInput`/`CommandInput` variants to `tui::app::Prompt`, reusing the
`handle_prompt_key` dispatch already built for rename/create/delete-confirm/conflict; (B) a new
`Mode` state machine with a small command-registry trait, so future commands register instead of
piling into one `match`; (C) pull in an external line-editor crate (e.g. `tui-input`) for real
cursor movement instead of the existing char-append/backspace-only buffer editing.

#### Decision & Rationale

Chose **A**. It reuses the exact machinery already rendering/editing the other four prompt kinds
(`Prompt::display()` feeds the same status-bar line, so no new rendering code was needed at all),
and it doesn't foreclose B or C later — a command registry or a real line-editor are both things
to layer on once there's enough command variety or editing complexity to justify them, not
something to decide up front for an initial `:q`/`:cd` command set. `handle_prompt_key` changed
its return type from `Result<()>` to `Result<ControlFlow<()>>` so a `:q`/`:quit` command can
signal the app to exit — the only new plumbing this cycle needed, everywhere else unchanged.
`BrowserState` gained three small, independently-testable methods to back this:
`select_index` (clamped jump, used by both `/`'s live match-jump and `Esc`'s restore-to-origin),
`find_match` (case-insensitive substring search, always from the top so backspacing a query
re-finds the same match deterministically), and `goto` (arbitrary-directory jump backing `:cd`,
resolving a relative path against the current directory).

**Verification finding (harness bugs, not app bugs):** the first PTY verification attempts for
this feature appeared to hang or find nothing, and both causes were in the test harness, not the
app. First, `pty.openpty()` leaves the pty at its default 0x0 window size unless
`ioctl(TIOCSWINSZ)` is called explicitly; ratatui then lays out every pane as a zero-area rect and
draws only per-tick escape-code boilerplate (cursor-hide, default-color reset) forever, which
looks exactly like a hang from the outside. Second, once the window size was fixed, a raw
keystroke-by-keystroke capture never contained the typed text as one contiguous string — ratatui
only rewrites *changed* cells between frames, so e.g. typing `/bravo` one character at a time
mostly emits single-cell diffs, not a re-write of the whole line. The fix: force a genuinely full,
non-diffed redraw before capturing — changing the pty's reported size and sending `SIGWINCH`
triggers exactly the same `Terminal::resize`-driven full-buffer redraw this codebase already
relies on after the shell overlay closes (see the Post-Shell Redraw entry above) — then capture
that one frame instead of the raw incremental stream.

---

### Text Preview Concurrency: `spawn_blocking` Read, Mirroring `ImagePreview`'s Decode Pipeline

**Date:** 2026-09-17
**Status:** Confirmed

#### Context / Background

The `preview` crate's own module doc has called out "text + image preview" as its scope since
v0.1.0, but only the image half existed. Three options were considered for the read itself: (A) a
plain synchronous `std::fs::read` call inline in `draw`, since reading a small text file is fast;
(B) the same `tokio::runtime::Handle::spawn_blocking` + channel pipeline `ImagePreview` already
uses for decoding, adapted for a plain read instead of a decode; (C) memory-map the file instead
of reading it fully, to avoid copying large files into a `String`.

#### Decision & Rationale

Chose **B**. This project's established rule (first applied to bulk file ops, then to image
decoding) is that no I/O capable of stalling — however rarely — belongs on the render thread, and
a text file is not exempt just because it's *usually* small: a slow or network-backed filesystem
can make even a small read block for an unbounded time, and the failure mode (a frozen TUI) is
identical regardless of why the read is slow. Reusing the exact channel-based
target-changed/spawn/drain shape `ImagePreview::update` already established (new `TextPreview` in
`crates/tui/src/text_preview.rs`) means the two preview pipelines read the same to a future
maintainer, at the cost of one more `PreviewStatus` enum and channel pair. Rejected C: memory
mapping serves a materially larger workload than a preview pane will ever show, and adds an
`unsafe`-adjacent dependency for no benefit the `MAX_TEXT_PREVIEW_BYTES` size cap doesn't already
solve more simply.

`preview::is_text`/`load_text` mirror `is_image`/`load_image`'s exact shape: extension-based
eligibility (plus a small filename whitelist for extensionless files like `Makefile`/
`.gitignore`), and a `None` return — never an error — on any read/decode failure, so the caller's
existing "preview failed" fallback needed no new branch. `load_text` treats a null byte anywhere
in the file, invalid UTF-8, or a size over 1 MiB as failure, so a binary file mislabeled with a
text-like extension degrades exactly like a corrupt image does today, rather than dumping garbage
or stalling on a huge log file. `ImagePreview` and `TextPreview` are now constructed and updated
together through a new `Previews` struct in `tui::main` — a direct, in-scope fix for the
`clippy::too_many_arguments` warning that adding a second preview pipeline's parameters to `run`/
`draw` would otherwise have triggered (8 arguments, one over the default limit), not an unrelated
refactor.

**Verification finding (harness bug, not an app bug):** the first PTY verification attempts for
this feature saw every keystroke sent after startup vanish with zero effect — no selection
movement, no screen change at all. Root cause, once isolated: `ratatui-image`'s
`Picker::from_query_stdio()` (already in use for image preview, and already the subject of a
`Verification finding` in the Image Preview Concurrency entry above) spawns a background thread to
read its terminal-capability query's response and never stops that thread if nothing answers —
this project's synthetic PTY test harness deliberately never answers unless told to. That
lingering thread keeps consuming stdin bytes forever, racing the app's own event-loop thread for
the same fd, so it silently stole every subsequent keystroke the test harness sent. The fix
(harness-only, documented rather than worked around in application code, per the same reasoning as
the earlier entry): have the harness answer the Device Status Report query (`\x1b[5n` → `\x1b[0n`)
itself right after startup, which lets that background thread finish and stop competing for
stdin. A second, unrelated harness gap surfaced alongside it: raw-diffed escape-code output can
split a single line of rendered text across multiple writes with cursor-repositioning escapes in
between (ratatui skips re-writing a cell whose content is unchanged from the previous frame, even
mid-word), so a naive substring search over the raw byte stream can miss text that is genuinely on
screen. Switching the harness to feed the raw stream through a `pyte` terminal emulator and
searching its reconstructed screen buffer instead fixed this reliably.

---

### Popup Shell Mechanism: Full Pty (`portable-pty`) + `vt100` Parser Rendered as a Ratatui Widget

**Date:** 2026-09-17
**Status:** Confirmed

#### Context / Background

The previous cycle's roadmap entry for "shell overlay as an embedded popup terminal emulator"
parked three options without a decision: (A) a full pty + `vt100`-parser popup, rendering the
child's screen buffer as a bordered ratatui widget layered over the main UI; (B) a constrained-pty
takeover that draws directly into a sub-region of the real terminal without rendering minuteman
behind it; (C) a purely cosmetic transition, keeping today's full-screen suspend/resume shell and
just showing a message before suspending. The roadmap entry's own recommendation was A — B needs
the same ANSI-escape-code interception work as A for a strictly weaker result (blank space instead
of minuteman behind the popup), and C doesn't deliver a small window at all, just a nicer
transition into the existing full-screen behavior.

#### Decision & Rationale

Chose **A**, confirming the parked recommendation. `shell_overlay` gained a new `PopupShell` type:
`PopupShell::spawn` opens a `portable-pty` pty sized to the popup and spawns `$SHELL` on it (same
`$SHELL`-or-`/bin/sh` fallback as the existing `spawn_shell`), then hands the master's reader to a
background thread that feeds every byte into a `vt100::Parser` behind an `Arc<Mutex<_>>` — the
same "never block the render loop" rule this project has applied to every I/O source since the
async-bulk-ops cycle, just applied to pty reads instead of file reads. `write_input`, `resize`,
`with_screen`, and `try_wait` round out the API; `try_wait`'s return type (`ExitOutcome`) is a
small local struct rather than `portable_pty::ExitStatus` directly, so the pty backend stays fully
contained to `shell_overlay` — nothing above it needs to depend on `portable-pty` to read an exit
code. `vt100::Screen`/`Cell`/`Color` are unavoidably part of `PopupShell`'s public surface (`tui`
needs real access to cell styling to render it), so `vt100` — but not `portable-pty` — is also a
direct dependency of `tui`; that's the deliberate line between "the pty mechanism" (contained) and
"the screen-buffer data model" (necessarily shared).

The crossterm/ratatui half lives in a new `tui::popup_shell` module: `popup_area` centers a
popup at 80%×70% of the frame (clamped so it never exceeds a tiny terminal), `pty_size` derives
the pty's rows/cols from the popup's inner area (minus the one-cell border), `encode_key`
translates a `KeyEvent` into the raw bytes a real terminal would send (arrows, function keys,
`Tab`/`BackTab`/`Home`/`End`/paging, `Ctrl`+letter → its control code, `Alt` → a leading `ESC`),
and `render` walks the `vt100::Screen` cell-by-cell into styled `Span`s (color, bold, italic,
underline, inverse) and positions the real cursor over the child's cursor cell when it isn't
hidden. `main.rs`'s event loop forwards every key event straight to the popup (bypassing the
normal `Action` dispatch entirely) while one is open, handles `Event::Resize` by resizing the pty
to match, and polls `try_wait` non-blockingly once per tick — closing the popup and setting a
"shell exited"/"shell exited: code N" status the moment the child exits, with no dedicated
close-hotkey needed. The old `spawn_shell` (full-screen, direct-stdio-inheritance) function and its
test are left exactly as they were in `shell_overlay` — just no longer wired to the `s` keybind —
since deleting tested, working code the roadmap never asked to remove would be scope creep beyond
what "add the popup" requires; `TerminalGuard::suspend`/`resume` in `main.rs`, which existed only
to support that old call site, were removed as now-genuinely-dead code.

**Verification:** confirmed against the real compiled binary via a scripted PTY session,
reconstructed through `pyte` and with the harness answering the startup Device Status Report query
itself — this project's own established fix (see the Image Preview Concurrency and Text Preview
Concurrency entries above) for `ratatui-image`'s terminal-probe background thread otherwise
swallowing every keystroke sent after startup on a synthetic, non-responding pty. Pressing `s`
showed a genuinely bordered "shell" popup with the rest of the browser's three panes still visible
around it — not a full-screen takeover. A real `echo` command typed inside the popup produced its
actual output inside the popup's own screen buffer. `Ctrl-C` sent while a foregrounded `sleep 20`
was running interrupted it almost immediately (the shell's prompt returned well before 20 seconds
had passed), confirming the control-code encoding path works and not just plain characters.
Resizing the pty mid-session (`ioctl(TIOCSWINSZ)` + `SIGWINCH`, the same combination the
Command/Search Bar entry above found necessary to force a real redraw) reflowed both the popup and
the panes behind it, and the shell inside kept accepting commands afterward. Typing `exit` closed
the popup and restored the exact underlying UI with a "shell exited" status message, and `q`
quit the app cleanly (exit code 0) afterward.

---

### Popup Shell `Esc`-to-Close: Intercept Unconditionally, Trading Away In-Popup `vim` `Esc` Usage

**Date:** 2026-09-17
**Status:** Confirmed

#### Context / Background

Requested immediately after the popup shell shipped: give `Esc` a way to close the popup, rather
than requiring `exit` (or waiting for the child to exit on its own) every time. The popup shell's
own design goal, recorded in the entry above, was full interactivity — "vim/less/ssh all still
work inside it" — and `vim` treats `Esc` as meaningful input (leaving insert mode). Any
unconditional interception of `Esc` at the popup layer necessarily conflicts with that: there's no
way for `vim` running inside the popup to ever see an `Esc` keystroke once the popup itself claims
it first.

#### Decision & Rationale

Chose the simple, unconditional version anyway, as explicitly requested, rather than a
foreground-process-aware heuristic (e.g. checking the pty's foreground process group via
`tcgetpgrp` and only closing when the shell itself — not a subprogram like `vim` — is in the
foreground). That heuristic would preserve `vim`'s `Esc` while still giving a quick-close
shortcut at the shell prompt, but adds real complexity (a new syscall dependency, more edge cases
around job control accuracy) for a distinction that wasn't asked for. `exit` still works
regardless, so nothing is lost for the `vim` case beyond `Esc` specifically no longer reaching
it — a real, deliberate narrowing of the original "full interactivity" claim, recorded here so a
future reader doesn't mistake it for an oversight.

Implementation: `main.rs`'s key-handling branch for an open popup checks `key.code == KeyCode::Esc`
before calling `popup_shell::encode_key` at all, so `Esc` never reaches the pty. New
`PopupShell::close` calls the child's `kill()` then `wait()` (not just `kill()`) so the process is
reaped immediately rather than left as a zombie until the next `try_wait` poll or the whole app
exiting.

**Verification:** confirmed against the real compiled binary via the same `pyte`-reconstructed PTY
harness used for the popup shell's own verification above: `Esc` pressed while a real shell prompt
was active inside the popup closed it immediately, restored the underlying UI exactly as it was
before opening the popup, and set the status line to "shell closed".

---

### Marks / Multi-Select Scope: Wire Into `Delete` Only This Cycle

**Date:** 2026-09-18
**Status:** Confirmed

#### Context / Background

Requested: a Ranger-style `v` to toggle a mark on the current entry, "for the action later (e.g.:
deletion)." `ROADMAP.md` had already flagged full multi-select (marking several entries for one
bulk op across `Yank`/`Cut`/`Paste`/`Delete`) as separate, unscheduled Low Priority scope, since
`BrowserState` only ever tracked a single `selected: usize` and `Clipboard` only ever held one
`PathBuf`.

#### Options Considered

**Option A: Mark set, visual indicator only** — `v` toggles a `HashSet<PathBuf>` on
`BrowserState`; nothing consumes it yet.
- Smallest possible diff, but doesn't satisfy the explicit "for the action later (e.g.: deletion)"
  ask — marking would be visible but functionally inert.

**Option B: Mark set wired into every bulk action** (`Yank`/`Cut`/`Paste`/`Delete`) *(rejected)*
- Matches Ranger most closely, but `Clipboard` (and the whole `spawn_paste` pipeline: conflict
  resolution, progress reporting) is built around exactly one path. Extending it to N paths in
  the same cycle that introduces marking at all is a large, higher-risk diff for scope nobody
  asked for yet.

**Option C: Mark set wired into `Delete` only** *(chosen)*
- Matches the literal, concrete example given. `spawn_delete` already ran one `file_ops::delete`
  call per invocation with no conflict-resolution branch to generalize (delete has no
  `AlreadyExists` case), so extending it from one target to `Vec<PathBuf>` — looping the same
  call, sending one `Progress` message per completed target, stopping at the first failure — was
  a contained, mechanical change. `Yank`/`Cut`/`Paste` are left exactly as they were, still
  operating on the single cursor selection.

#### Decision & Rationale

Chose C. `begin_delete` now prefers `browser.marked_paths()` when non-empty, falling back to the
single cursor entry otherwise — Ranger's own "act on marks if any, else the current file"
convention. `Prompt::ConfirmDelete` gained a `targets: Vec<PathBuf>` (from a single `target:
PathBuf`) and its confirmation text pluralizes ("delete 3 marked items permanently?" vs. the
original single-file wording). A successful or partially-failed bulk delete now also calls the
new `BrowserState::prune_marks(vfs)` (drops any mark whose path no longer exists), so a mark
pointing at an already-deleted file can't linger and silently get swept into some future bulk
action. Marks persist across `enter`/`leave` navigation rather than being scoped to one directory
listing, matching Ranger's session-wide marking rather than clearing them on every `refresh`.
Extending `Yank`/`Cut`/`Paste` to the marked set is recorded here as the natural next step, not
implemented — it needs `Clipboard` to hold multiple paths and `spawn_paste`'s conflict handling to
iterate, which is a bigger, separate design decision.

**Verification:** confirmed against the real compiled binary via a scripted PTY session
reconstructed through `pyte`, answering the startup Device Status Report query as this project's
own established fix requires (see the Image/Text Preview Concurrency and Popup Shell entries
above). Pressing `v` on two different files showed each one prefixed with `* ` in the current
pane; `d` with two marks active prompted "delete 2 marked items permanently?" (not the single-file
wording); `y` deleted exactly those two files off disk while a third, unmarked file and a
directory in the same listing were left untouched, and the status line read "delete complete".

---

### Leader Key Design: Inert, Non-Modal Placeholder

**Date:** 2026-09-18
**Status:** Confirmed

#### Context / Background

Requested: bind `space` as a leader key "for features i may add later," with an explicit
constraint that every other key must keep working exactly as before — no modal capture.

#### Decision & Rationale

Added `Action::Leader` (default `space`) resolved through the exact same `KeyMap::resolve`/
`Action` dispatch every other key already goes through, with a literal no-op arm in `main.rs`'s
match. No new state (no "awaiting chord" flag, no timeout, no capturing sub-loop) was introduced,
since there is nothing yet for a chord to invoke — a stateful chord-capture mechanism would be
speculative infrastructure for bindings that don't exist. Because dispatch stays a flat,
one-keystroke-in-one-action-out match (unchanged from how `Select`, `Delete`, etc. already work),
`space` pressed on its own truly does nothing and cannot intercept, delay, or swallow any
subsequent keystroke — satisfying "the other keys to work without it" by construction rather than
by special-casing.

**Verification:** confirmed against the real compiled binary via the same PTY session as the
marks entry above: pressing `space`, then continuing to navigate and mark files and run a
multi-target delete, produced identical behavior to a run with the `space` keypress omitted.

---

### Movable/Detachable Popup Shell: Two Literal Keys, State Local to `main.rs::run`

**Date:** 2026-09-18
**Status:** Confirmed

#### Context / Background

Requested: make the popup shell (see the Popup Shell Mechanism entry above) movable on screen,
and let the browser stay usable while the popup is open, through a specific key — i.e. the shell
should be detachable from keyboard focus instead of monopolizing every keystroke until it closes.
Until this cycle, `main.rs`'s event loop forwarded every key except `Esc` straight into the
popup's pty whenever it was open, so the browser was entirely unreachable while a shell window
was up.

#### Options Considered

**Option A: Two separate literal keys — a focus-toggle plus a dedicated move-mode** *(chosen)*
- A new key toggles whether keystrokes go to the shell or the browser; a second key enters a
  transient move-mode where `hjkl`/arrows nudge the popup's offset until `Enter`/`Esc` confirms.
- Keeps `shell_overlay` exactly as UI-agnostic as it already was (position lives only in the
  `tui` crate), and needs no new cross-cutting mechanism — each mode is the same shape as the
  existing prompt/command-bar states.

**Option B: One focus-toggle key plus always-live `Alt+hjkl` for moving** *(rejected)*
- Fewer explicit modes, but `Alt+letter` would have to be intercepted before pty forwarding,
  permanently shadowing that combo from ever reaching the shell — a real (if narrow) regression
  for shell programs/readline that use `Alt+letter` for word navigation.

**Option C: Build on the already-shipped, currently-inert `Leader` key for chorded moves**
*(rejected)*
- Would give `Leader` (`space`) its first real use via a `space`+`hjkl` chord instead of a third
  literal keybinding, but requires building generic chord-sequencing machinery that doesn't exist
  anywhere in the codebase yet — more new surface than this feature strictly needs.

#### Decision & Rationale

Chose A. New `Action::ShellFocus` (default `tab`) and `Action::ShellMove` (default `g`), resolved
through the same `KeyMap::resolve` every other key already goes through. Both are meaningless
without an open popup and are deliberate no-ops in the main dispatch match in that case — the
same "inert when not applicable" precedent the `Leader` key set. All of the actual state
(`popup_focused: bool`, `popup_offset: (i32, i32)`, `moving_popup: bool`) lives as local variables
in `main.rs::run`, right next to the existing `popup: Option<PopupShell>` — not on `App` or in
`shell_overlay` — since the popup's on-screen position is purely a `tui`-crate/UI concern that
`shell_overlay` has no reason to know about. `popup_shell::popup_area` gained an `(i32, i32)`
offset parameter, clamping the final rect so the popup can never be nudged fully or partially
off-screen (nudging further in a direction that's already at the clamp boundary is simply a
no-op, rather than needing separate bounds-checking on the accumulator itself). Re-spawning a
shell (`s`) always resets `popup_offset` to `(0, 0)` and `popup_focused` to `true`, so every new
popup starts centered and focused exactly like before this cycle; the `Action::Shell` match arm
also gained an explicit `popup.is_none()` guard even though that branch is already unreachable
while a popup is open (handled earlier in the same `if let Some(active) = popup.as_mut()` block)
— guarding it directly rather than relying on that invariant, since spawning a second shell here
would have silently dropped the running one without closing it.

**Verification:** confirmed against the real compiled binary via a scripted PTY session
reconstructed through `pyte`, using this project's own established fix (answering the startup
Device Status Report/Device Attributes queries so `ratatui-image`'s probe thread never blocks and
swallows later keystrokes — see the Image Preview Concurrency entry above). This cycle's harness
also needed one further fix not previously written up: `pyte` has no APC (Application Program
Command) handler, so `ratatui-image`'s Kitty-graphics capability query (`ESC _ ... ESC \`, sent
once at startup) corrupted `pyte`'s parser state for the rest of the session unless stripped from
the raw byte stream before feeding it in — and every read had to be fed into the *same* persistent
`pyte` screen rather than only the bytes captured by the "final" read, since ratatui's own
cell-diffing means a `SIGWINCH`-forced redraw can emit little or nothing new once the screen
already reflects the current state (an early version of the harness discarded the real first
frame this way and then saw nothing on every subsequent capture). With both fixed: pressing `s`
opened the bordered popup and typing `echo hello_from_popup` produced that real output inside it;
`tab` unfocused the shell (status line read "browsing — tab to refocus the shell") and the
browser's file list stayed visible and responsive to `j` while the popup itself stayed open and
rendered, with no stray `j` reaching the shell's own output; `g` entered move mode ("moving
shell…" status), `llllllll` plus `Enter` moved the popup measurably to the right on screen and set
a "shell moved" status; `tab` refocused it ("shell focused"), and `Esc` closed it ("shell closed"),
with the popup's border genuinely gone from every row above the status line afterward.

---

### Multi-Shell Layout Model + Pane Mouse Interaction: Tmux-Style Split Tree, Divider-Resize, Click-Focus

**Date:** 2026-09-18
**Status:** Confirmed

#### Context / Background

Requested: a tmux-like feature for opening multiple mini-shells at once, plus moving them around
with the mouse. Until this cycle, `main.rs` held exactly one `Option<PopupShell>`, positioned by
a free `(i32, i32)` offset from center and moved only by the keyboard move-mode from the previous
cycle (see the Movable/Detachable Popup Shell entry above) — there was no way to have more than
one shell open, and no mouse handling anywhere (`EnableMouseCapture` was never on).

#### Options Considered — Multi-Shell Layout

**Option A: A `Vec` of independent floating popups, each still using the existing offset model**
- Smallest diff — reuses the existing popup rendering/move-mode almost unchanged, just per-window
  instead of global.
- Rejected: no auto-layout means overlapping windows bury each other, and floating positioning
  doesn't compose with the mouse feature actually wanted (dragging a *tiled* pane means resizing a
  neighbor, not repositioning a free-floating rect).

**Option B: A binary split-pane tree, tmux-style** *(chosen)*
- Each `Split` node carries a direction and a ratio, recursively partitioning its parent's rect
  between two children, so panes fill the frame and never overlap.
- No manual positioning is needed at all — panes resize via their split ratio instead of moving.

**Option C: Tabs — one visible pane at a time, others kept running in the background**
- Rejected: loses the "see several shells side by side at once" tmux feel the feature exists to
  deliver, and a mouse can't meaningfully reposition something you can't see.

#### Options Considered — Mouse Interaction

Once B was chosen, the original three mouse COAs (all assumed a free-floating popup with an
independent `(x, y)` — drag-by-titlebar, drag-while-unfocused, click-to-focus-only) stopped
applying: a tiled pane's rect is *derived* from the tree, not an independent position, so
"moving" it necessarily means resizing a sibling, not repositioning a window.

**Option A: Drag the shared divider between two sibling panes to resize; click a pane to focus**
*(chosen)*
- The literal, correct interpretation of "move" once panes are tiled — exactly how tmux/i3/most
  tiling window managers handle the mouse. No z-order to reason about since panes never overlap.

**Option B: Drag-to-reorder — grabbing a pane and dropping it onto another swaps their tree
positions**
- Rejected for this cycle: real tree-surgery complexity for an interaction that's uncommon even
  in mature tiling multiplexers. Left for a future cycle if it's actually missed.

#### Decision & Rationale

Chose split-pane-tree (B) + divider-resize/click-focus (A), explicitly deferring reordering.
`crates/tui/src/shell_layout.rs` is new: a private `Tree<T>`/`Node<T>` (`Leaf(T)` or
`Split{direction, ratio, first, second}`) generic over the leaf payload, plus free functions
(`split_rect`, `Divider`, `close_id`, `split_id`, `focus_at`, `leaf_ids`) that never touch `T`'s
contents — only its identity (`id: usize`) and the tree's shape. This genericity isn't
speculative future-proofing; it's what makes the tree-surgery logic (which is the actual risky,
novel part of this feature) unit-testable with plain `String`/`u32` payloads instead of needing to
spawn a real `$SHELL` process per test case, matching this project's existing preference for pure
unit tests wherever the logic allows it. `close_id`/`split_id` are ownership-based (`fn(tree:
Tree<T>, ...) -> Result<Tree<T>>`-shaped), which surfaced a real bug during implementation: the
first draft of `split_id` moved `new_payload` into the recursive call on `first` unconditionally,
so if the target leaf was actually in `second`, the newly-spawned shell was already gone by the
time `second`'s recursion needed it. Fixed by having the `NotFound` variant hand the still-unused
payload back to the caller so it can be retried against the sibling — caught by a dedicated
regression test (`split_id_finds_the_target_inside_the_second_child`) before it ever reached the
real binary. `ShellPanes` (public) specializes the tree to `PopupShell` and owns everything that
actually touches a pty: `open`/`split`/`close` spawn/kill real shells and resize every remaining
leaf's pty to its new rect immediately (before the next render), `poll_exits` reaps a shell that
exited on its own (e.g. typing `exit`), and `render` highlights the focused pane's border with the
theme's `selection_bg` so it's visible at a glance which one keystrokes route to. Panes render
docked into the frame's existing `rows[0]` (the same area the three browser columns use, computed
via a shared `shell_area` helper), not the old floating popup's full-frame-relative rect — this
keeps the bottom status bar always visible instead of risking a tiled pane growing over it.
`Action::ShellMove` is removed entirely (there's nothing left to "move" — a pane's rect is a pure
function of the tree); `theming::Action` gained `ShellSplitHorizontal`/`ShellSplitVertical`
(defaults `%`/`"`, tmux's own bindings) and `ShellPaneNext` (default `o`) for keyboard-only pane
cycling, kept alongside click-to-focus rather than replacing it, since not every session has a
usable mouse.

**Verification:** confirmed against the real compiled binary via a scripted PTY session
reconstructed through `pyte`, using this project's established fixes (answering the startup
Device Status Report probe, stripping the Kitty-graphics APC query before feeding `pyte` — see
the Image Preview Concurrency and Movable/Detachable Popup Shell entries above). `s` opened one
bordered pane; `%` split it into two side-by-side panes sharing the same top-border row, with the
new (right) one focused — confirmed by typing distinct marker commands into each and finding each
marker's output landed only on its own side, never leaking across. `o` (keyboard pane-cycling)
moved focus to the left pane, confirmed the same way; a real SGR mouse click back on the right
pane refocused it, confirmed by a third marker landing there. A real SGR mouse-down on the exact
column where the two panes' borders meet, followed by a drag and mouse-up, moved that column
measurably to the right — confirming `Divider::hit`/`ratio_at`/`ShellPanes::set_ratio` all wire up
correctly through actual mouse escape sequences, not just their unit tests. `Esc` closed the
focused pane and the remaining one was resized to fill the whole freed width (not left stopping at
the old divider); a second `Esc` closed the last pane entirely, and the browser stayed responsive
to `j`/`q` afterward.

---

### Shell Pane Container Sizing: Centered 80%/70% Box, Not the Full Browser Area

**Date:** 2026-09-18
**Status:** Confirmed

#### Context / Background

Right after shipping the split-pane tree (see the entries above), tried it against the real
binary and found `shell_area` sized panes to the *entire* browser region (the full frame minus
the status bar) — so a single shell pane filled the whole screen, and the browser columns
disappeared behind it entirely. Feedback: it should behave like a mini floating window again (as
the original single popup did), not take over the full screen — panes should still split/tile
normally, just within a smaller contained area.

#### Decision & Rationale

`shell_area` now computes its old full-browser-region rect first, then centers an 80%-width/
70%-height box inside it (the exact sizing the original single-popup `popup_area` used, clamped
the same way — `.max(...).min(...)` so a tiny terminal still gets a sane, non-overflowing box).
No offset/move-mode was reintroduced — that mechanism was deliberately dropped when the tree
replaced the floating popup (see the Multi-Shell Layout Model entry) and nothing here changes
that; the box's position is fixed and centered, only its *contents* (the split tree) are dynamic.
`draw` was also changed to call `shell_area` directly for rendering instead of separately
inlining the same layout math it already had for the browser columns — the previous version
worked by coincidence (both computations happened to agree) but had no structural guarantee they
always would as the code evolved further; calling the one function from both places removes that
risk entirely.

**Verification:** re-ran the same scripted PTY session used for the split-pane feature itself
(same divider-drag/focus-cycling/close assertions, all still passing) and confirmed the shell
box's top-left corner is now several rows/columns in from `(0, 0)` — the browser is visible around
it — rather than starting flush with the frame's corner.

---

### Shell Box Drag Mechanism: Drag the Title Bar by Mouse, Offset Re-Added to `shell_area`

**Date:** 2026-09-18
**Status:** Confirmed

#### Context / Background

Requested right after the box was confined to a mini floating area: make that box itself movable,
"like a window-tab" — i.e. grab it and drag it around the screen, the way a floating window's
title bar works. This is a different thing from the pane-mouse-interaction decision earlier this
cycle, which explicitly ruled out dragging *individual panes* (a pane's rect is derived from the
split tree, not an independent position) — the box is the tree's container, not a member of it, so
repositioning the whole box doesn't reintroduce that rejected idea.

#### Options Considered

**Option A: Drag the box's top border with the mouse** *(chosen)*
- Grabbing the row where the "shell" title already renders and dragging moves the whole box.
  Reuses the exact mouse-drag plumbing (`Down`/`Drag`/`Up` on `MouseButton::Left`, an accumulating
  `(i32, i32)` state variable) that already shipped for divider-dragging one entry ago, and is the
  literal match for "like a window-tab" — that's how a real window manager's floating windows move.
- Only works with a mouse; a session with no mouse-reporting terminal has no path to move the box
  this cycle.

**Option B: Keyboard move-mode, resurrecting the pre-tiling `g` + `hjkl` design**
- Would work with no mouse and matches the rest of the app's keyboard-first bindings, but a nudge-
  by-key modal doesn't really read as "dragging a tab" — it's the mechanism the *pane*-mouse
  decision already superseded once tiling landed, now being brought back only for the outer box.

**Option C: Both A and B on one shared offset**
- Most complete "real floating window" feel, but doubles the surface (two input paths, more tests)
  for a single cycle. This project's own history ships one input modality per feature and adds the
  other later if it's actually missed — keyboard splits/focus shipped a full cycle before mouse
  divider-drag did.

#### Decision & Rationale

Chose **A**, keeping this cycle surgical the same way divider-drag and split-bindings were kept
separate features rather than one combined "shell mouse+keyboard" cycle. `shell_area` gained an
`offset: (i32, i32)` parameter — centered position plus offset, then clamped to the browser area
(`min_x`/`max_x`/`min_y`/`max_y` derived from the same box-size math, `.max(min)` guarding against
a degenerate case where the box is as large as the browser area itself) — so the box can never be
dragged fully or partially off screen. `main.rs::run` gained `shell_offset` (the accumulated
offset) and `dragging_shell` (the last mouse position seen mid-drag, `None` when no drag is in
progress) alongside the existing `dragging_divider`. A mouse-down is checked against the divider
hit-test first (unchanged priority from the previous cycle), then against the box's exact top row
— `mouse.row == area.y` — before falling through to pane click-to-focus, so a divider that happens
to touch the top edge still wins the ambiguity the same way it always has. `shell_offset` resets to
`(0, 0)` on every fresh `s` spawn, matching the pre-tiling popup's own "starts centered" behavior.
`draw`'s growing argument list (already at the clippy-flagged edge) got a new `ShellView<'a>`
struct (`panes: &'a ShellPanes`, `offset: (i32, i32)`) bundling the pane tree with its offset,
mirroring how `Previews` already bundles the image/text preview pipelines — `draw` takes one
`Option<ShellView<'_>>` instead of two separate parameters that always travel together.

**Verification:** confirmed against the real compiled binary via a scripted PTY session using
this project's established fixes (answering the DSR probe, stripping the Kitty-graphics APC query
before feeding `pyte` — see the Image Preview Concurrency and Multi-Shell Layout Model entries)
plus real SGR mouse escape sequences for the drag itself, not just the new `shell_area` unit
tests: pressing `s` opened the box centered exactly where the unit test predicts; a mouse-down on
its title row followed by a drag and mouse-up moved the box by precisely the dragged delta (row
+4, column +10, matching the mouse movement exactly); a second run dragging far past the frame's
edge left the box clamped flush against it instead of vanishing or panicking; the browser's title
(showing the real cwd) stayed visible and correct throughout, and `Esc` then `q` still closed the
shell and quit the app cleanly afterward.

---

### Batch Yank/Cut/Paste Continuation: `Clipboard` Holds `Vec<PathBuf>`, `poll_bulk` Re-Spawns the Next Item Itself

**Date:** 2026-09-18
**Status:** Confirmed

#### Context / Background

`v` marks already drove `Delete` (a prior cycle), which snapshots `browser.marked_paths()` into a
`Vec<PathBuf>` at confirm-time and loops `file_ops::delete` over it inside one `spawn_blocking`
call — straightforward, since delete has no conflict/retry path to interrupt the loop. `Yank`/
`Cut`/`Paste` were the one piece of that prior cycle's scope explicitly deferred (see the Marks /
Multi-Select Scope entry): `Clipboard` only ever held a single `PathBuf`, so pasting a multi-marked
batch still only ever moved/copied the single entry under the cursor. Unlike delete, paste's
`file_ops::copy_with_progress`/`mv_with_progress` can hit `VfsError::AlreadyExists` mid-item and
need the user to pick overwrite/skip/abort before continuing — a naive single `spawn_blocking` loop
like delete's has no way to surface that prompt and then resume where it left off.

#### Options Considered

**Option A: Batch `Clipboard`, chained via the existing bulk-completion path** *(chosen)*
- `Clipboard.path: PathBuf` becomes `Clipboard.paths: Vec<PathBuf>`; `yank`/`cut` snapshot marks-
  if-any-else-cursor, the same convention `begin_delete` already established (now shared via a new
  `App::marked_or_selected` helper). One item is spawned at a time via `spawn_paste_item(clip,
  dst_dir, index, policy)`; `poll_bulk`'s completion handling (`Outcome::Completed` /
  `Outcome::Skipped`) re-spawns the next index itself before returning, instead of ending the
  operation, so an N-item batch stays one continuous "busy" state from the UI's perspective. A
  conflict still surfaces the existing single-item `Prompt::Conflict` and pauses the whole batch;
  `resolve_conflict` resumes at the same index (retry, on overwrite) or lets the next `poll_bulk`
  tick advance past it (on skip).
- Reuses `Prompt::Conflict`/`ConflictPolicy`/the per-item `spawn_blocking` shape almost unchanged;
  conflict UX for a batch is pixel-identical to today's single-item flow — nothing new to learn.
- Progress is per-item, not one continuous byte/file stream; an accurate `[i/N]` batch-position hint
  needed a small addition to `BulkKind::Paste` (`index`) rather than falling out for free.

**Option B: One background task drives the whole batch, conflict policy decided up front**
- A new `file_ops::paste_batch_with_progress` loops every `(src, dst)` pair inside a single
  `spawn_blocking`, mirroring `spawn_delete`'s existing internal loop. Since nothing can pop out to
  ask mid-flight, the overwrite/skip/abort choice would have to be made once before the batch starts
  instead of per conflicting file.
- Trivial accurate progress counter and the fewest background spawns, but a real UX regression:
  losing the ability to make a different call on each conflicting file within one batch — and it
  needs a new `file_ops` API plus a new "decide up front" prompt type that doesn't exist anywhere
  else in the app.

**Option C: UI-only queue, zero `file_ops`/`Clipboard` signature changes**
- Keep `Clipboard.path` singular; `App` reads live marks into a queue at `begin_paste` time (not at
  yank/cut), popping and calling the unmodified single-item `spawn_paste` as each finishes.
- Smallest textual diff, but semantically wrong: yank/cut wouldn't actually capture the batch when
  pressed — marks re-read at paste time mean marks changed in between silently change what gets
  pasted, breaking the snapshot-at-mark-time convention `begin_delete` (and now yank/cut) already
  rely on.

#### Decision & Rationale

Chose **A** — it was the direction this project's own prior-cycle roadmap note already pointed at
("Extending `Clipboard` to multiple paths and looping `spawn_paste` per marked entry is the
remaining piece"), it's the only option that doesn't change conflict-resolution UX for existing
single-item pastes, and it extends the exact snapshot-marks-into-a-`Vec<PathBuf>` convention
`begin_delete` already set rather than inventing a parallel one. The real cost — `poll_bulk` gaining
a "continue the batch" branch instead of staying purely a drain-and-finish function — was judged
worth it against Option B's outright UX regression and Option C's stale-snapshot correctness gap.
This also closes the exact gap called out when marks first landed: `file_ops::ConflictPolicy::Skip`
existed specifically "for when a batch needs to continue past one conflicting item instead of
stopping," but was unobservable in the TUI since there was never more than one item in flight —
it's now the thing that makes Scenario B below actually exercise that code path for the first time.

#### Implementation Notes

- `ConflictSource::Paste` and `BulkKind::Paste` both gained `dst_dir: PathBuf` and `index: usize`
  alongside the existing `clip`/`dst`, so a paused-on-conflict batch knows exactly where to resume.
- New `try_continue_paste(clip, dst_dir, index) -> Option<Clipboard>` centralizes the "spawn the
  next item, or hand `clip` back for final cleanup" branch shared by the `Completed` and `Skipped`
  arms of `poll_bulk`, instead of duplicating the continuation check in both.
- The clipboard only clears (on a completed `Move`) once the *last* item finishes — an earlier
  item in a move batch that succeeded while a later one is later skipped leaves the clipboard
  holding paths that partially no longer exist at their original location. This is the same
  imprecision the single-item implementation already had for a skip (clipboard is never touched on
  `Outcome::Skipped` today either) — deliberately not solved here per this project's iteration
  rules against unrelated scope creep.
- Status line gained a `[i/N]` hint for `BulkKind::Paste` when `clip.paths.len() > 1`, and a new
  `batch_label` helper (single name, or `"N items"`) deduplicates what `Delete`'s status line and
  `Prompt::ConfirmDelete::display` were each already computing inline.

#### Verification

Confirmed against the real compiled binary via a scripted PTY session driving actual marks/yank/
cut/paste keystrokes — not just the unchanged `file_ops` unit tests, which never exercised the new
batch-continuation loop at all. Had to answer `ratatui-image`'s startup terminal-capability probe
first (`CSI c` / `CSI 16 t` / `CSI 5 n`), deliberately leaving its Kitty graphics query unanswered
so it falls back to "no Kitty support" the way any ordinary terminal would — the same synthetic-PTY
gap the Image Preview Concurrency entry documents, which otherwise leaves the probe's stdin-reading
thread blocked forever and silently swallows every keystroke sent afterward (this cost real debug
time this cycle before the cause was traced back to that exact prior entry).

**Scenario A (batch copy):** marked a directory plus two files, `y` to yank, navigated to an empty
destination, `p` to paste. All three landed with correct contents, including the recursed
subdirectory — confirming the multi-item queue correctly processes a `copy_with_progress` call that
is itself a full recursive directory copy, not just flat files.

**Scenario B (batch move with a real mid-batch conflict):** marked two files, `m` to cut, pasted
into a directory where the second file's name already existed. The first item moved immediately;
the batch correctly paused on the real overwrite/skip/abort prompt for the second. Pressing `s`
left that file physically untouched at its original location (not moved) and the pre-existing
destination file untouched (not overwritten), while the batch still finished cleanly afterward —
the first end-to-end exercise of `ConflictPolicy::Skip` actually continuing a batch past a
conflict, rather than ending the whole operation the way a single-item skip always has.

---

### Shell Pane Resize/Move Keybinding: `space`-Prefixed Chord on the Leader Key, Not a Modifier

**Date:** 2026-09-18
**Status:** Confirmed

#### Context / Background

Requested: let shell panes be resized and moved from the keyboard as well as the mouse, i3-style,
in a way quick enough to reach for "on a moment's notice." Until this cycle, `ShellPanes::set_ratio`
(divider resize) and the box's `(dx, dy)` offset (whole-box move) were only reachable by dragging
with the mouse.

#### Options Considered

**Option A: Always-live `Alt+hjkl`, no mode** *(rejected)*
- The most literally "instant" option — every keypress acts immediately, no mode to enter or
  leave. Rejected for two concrete reasons surfaced while scoping it: `KeyMap::resolve` takes a
  bare `KeyCode` with no modifier awareness at all today, so this needed new plumbing regardless
  of which modifier was picked; and `Alt`-prefixed `hjkl` specifically is real, commonly-configured
  `tmux`/`vim-tmux-navigator` pane-navigation input — exactly the kind of thing a real shell
  running *inside* one of these panes could legitimately have bound, and intercepting it at the
  app layer would silently steal it. This is the same shape of problem the Movable/Detachable
  Popup Shell entry's rejected Option B already ran into with `Alt+hjkl` for moving a single popup,
  now recurring for tiled panes.
- Note: that same entry's Option C — building on the `Leader` key for a chord — was *also*
  rejected at the time, but for a different, now-superseded reason: "more new surface than this
  feature strictly needs," when there was nothing yet a chord needed to invoke.

**Option B: A dedicated i3-style modal resize/move with real pane reordering** *(rejected)*
- Adds explicit resize/move modes plus actual tree-surgery to swap a focused leaf with a neighbor,
  matching i3's own semantics most closely. Rejected as reopening scope this project already
  closed deliberately: the Multi-Shell Layout Model entry's own note that "drag-to-reorder/swap
  panes" is "real tree-surgery complexity for a rarely-used interaction even in mature tiling
  multiplexers." Nothing about this request asked for pane reordering specifically — only resize
  and move — so taking on that complexity here would be solving a problem nobody raised.

**Option C: `space`-prefixed chord (`space r`/`space m`, then `hjkl`), reusing `Action::Leader`**
*(chosen)*
- Gives the previously-inert `Leader` key (see that entry above) its first real use. Since the
  chord is dispatched entirely at the app layer — the same tier `Esc`/`Tab`/`%`/`"`/`o` already
  preempt pty-forwarding at — it costs nothing from the shell's own keyspace: no modifier is ever
  claimed, so nothing a real shell might bind is ever shadowed. The one thing to get right is
  *where* it's reachable: `Action::Leader` (space) already only resolves in the same top-level
  dispatch match a literal typed space in a focused shell never reaches (a focused pane's `space`
  keypress forwards as a raw byte before that match ever runs), so entering the chord can never
  compete with typing an ordinary command with arguments into a live shell.

#### Decision & Rationale

Chose C. `Action::Leader` now sets a one-shot `pending_leader` flag when a shell pane is open
(still fully inert with none open, preserving the original entry's guarantee for that case); the
very next keypress — `r` or `m`, hardcoded rather than routed through `KeyMap::resolve` like the
existing `Esc`-to-close precedent — opens a `ShellChordMode::Resize` or `::Move`. While a chord is
active, `hjkl`/arrows are intercepted specially: `Resize` calls the new
`ShellPanes::resize_focused`, which walks up the pane tree from the focused leaf to the nearest
ancestor `Split` whose axis matches the pressed direction, then nudges that split's ratio by 5% in
whichever sign grows or shrinks the focused pane — regardless of which side of that split it's
actually on, so the key's direction always matches what visibly happens to the pane being looked
at, not an implementation detail of the tree's shape. `Move` just nudges the existing `(dx, dy)`
box offset by 2 cells per press, the exact same state the mouse-drag-the-title-bar interaction
already mutates — no new geometry logic needed there at all. `Esc` leaves either mode outright;
any other key also leaves it, but — unlike `Esc` — is *not* swallowed: it still gets dispatched
normally afterward, so e.g. pressing `q` while still inside a chord both exits the mode and quits
the app, rather than being silently eaten by it. This deliberately still does not add pane
reordering (Option B, still out of scope) — only resize and move, matching what was actually
requested.

#### Implementation Notes

- New `shell_layout::NudgeDir` (`Left`/`Down`/`Up`/`Right`) and a private `nearest_ancestor_split`
  search mirror the existing `dividers`/`focus_at`/`close_id` shape: generic over the leaf payload,
  unit-tested without spawning a real shell. Its one subtlety is that "found the leaf, but no
  ancestor of the requested axis exists yet" (`AncestorSearch::Located`) has to keep bubbling
  upward past non-matching splits rather than terminating — a plain `Option` can't distinguish
  that from "the leaf isn't in this subtree at all," which is why it's its own three-variant enum.
- `ShellChordMode`, `pending_leader`, and `apply_shell_chord` all live as local state in
  `main.rs::run`, next to `dragging_divider`/`dragging_shell`/`shell_offset` — the same
  precedent the Movable/Detachable Popup Shell entry set for keeping shell-overlay UI state out of
  `App`/`shell_overlay` and local to the event loop that actually owns the keystrokes.
- A pane can exit on its own (typing `exit` into it) between keystrokes; `run` now clears
  `pending_leader`/`shell_chord` whenever `shells` becomes `None` so a chord can never dangle with
  nothing left for it to act on.

#### Verification

Confirmed against the real compiled binary via a scripted PTY session reconstructed through
`pyte`, answering the startup Device Status Report probe per this project's established fix (see
the Image Preview Concurrency and Movable/Detachable Popup Shell entries): a command containing
spaces (`printf 'a b c\n'`) typed while a real shell had focus landed unchanged, proving the chord
can never steal input from a focused pty; opening a shell, splitting it, and unfocusing it, then
`space r` plus three `l` presses moved the shared divider between the two panes measurably left
(from column 49 to 37 at a fixed 100×40 terminal size) without moving the box's own top-left
corner; `space m` plus `lll`/`jj` moved the box's top-left corner by exactly 6 columns and 4 rows
(3×2 and 2×2 — the configured per-press step); and pressing `q` while still inside an active move
chord, with no `Esc` first, both left the mode and actually quit the process, confirming the
fallthrough dispatch rather than a swallowed keystroke. Also re-confirmed the pre-existing,
unrelated `o`-is-`ShellPaneNext`-before-pty-forwarding behavior while writing the harness (typing
`echo` into a focused shell drops every `o`) — a real but out-of-scope limitation for this cycle,
noted here rather than fixed, per this project's iteration rules against unrelated scope creep.

#### Follow-up: Resize Falls Back to the Box's Own Size When There's No Divider to Adjust

Reported from real usage, same cycle, before the commit above had even been made: with panes only
ever split one way (e.g. stacked via `"`, a height divider only), `h`/`l` in resize mode did
nothing at all — correctly, by the original design (there's no horizontal-axis ancestor split to
adjust), but indistinguishable from broken to someone who hadn't split in that direction and just
wanted "resize" to always do *something* sensible. Explicitly requested as a follow-up: "make it
possible to resize horizontally, also."

`ShellPanes::resize_focused` changed its return type from `anyhow::Result<()>` to
`anyhow::Result<bool>` — `true` when it actually found and adjusted a matching-axis divider,
`false` when there wasn't one. `shell_area` gained a second adjustment parameter, `size_adjust:
(i32, i32)` (a `(dw, dh)` nudge in cells from the default 80%/70%, alongside the existing `offset`
`(dx, dy)`), clamped the same "never itself clamped at the accumulator, only where it's consumed"
way `offset` already is — extreme values just saturate against `MIN_SHELL_BOX_WIDTH`/`HEIGHT` on
the small end and the full browser area on the large end, via `.clamp` on an intermediate `i32`
(explicitly not the original two-step `.max().min()` u16 chain, which would panic on a browser
area narrower than the stated minimum — a real, if narrow, difference worth keeping in mind next
time a `.clamp` gets introduced near a cast to a smaller unsigned type). `apply_shell_chord` now
tries `resize_focused` first when in resize mode; only when that reports `false` does it fall back
to nudging `shell_size` (a new piece of `run`-local state, reset to `(0, 0)` on every fresh `s`
spawn alongside `shell_offset`) by `SHELL_BOX_RESIZE_STEP` and re-`resize`-ing the pty tree against
the now-different box rect — unlike the move chord, this path *does* need that resize call, since
the box's own dimensions (not just its position) just changed.

**Verification:** confirmed against the real compiled binary via a scripted PTY session
(reconstructed through `pyte`, same fixes as above): a single unsplit pane and a vertically-split
(`"`) pane each grew the box's actual width (right border column minus left border column, not
just the raw right-border column, since growing re-centers and moves *both* edges) by exactly 6
columns on `space r` plus three `l` presses — confirming the fallback fires exactly when expected
and re-confirming (via the unchanged `verify_chord.py` re-run) that a horizontally-split pane's
divider resize was untouched by this change.

---

### Mouse Box-Width Resize + Split-Orientation Toggle: A Caught-and-Reverted `t` Keybinding Mistake

**Date:** 2026-09-18
**Status:** Confirmed

#### Context / Background

Two follow-up requests on the resize/move chord above, from actually using it: "make it possible
to make the mini-shells be resized horizontally using the mouse, too" (the box's own width, which
the previous cycle's keyboard fallback could already grow/shrink, had no mouse equivalent), and
"make it possible to change how they organized: one below the other, or side-by-side, using
keyboard" — flipping an existing split's orientation, which nothing could do before this cycle
short of closing a pane and re-splitting the other way.

#### Decision & Rationale: Mouse Box-Width Resize

The box's own right border (distinct from any internal pane divider, which is checked first and
never conflicts) is now a mouse-drag target, mirroring the title-bar-drag precedent: a mouse-down
on that exact column starts the drag, `Drag` events update `shell_size.0`, `Up` ends it. The one
subtlety is the mapping itself — since the box grows symmetrically from its centered position
(same model the keyboard fallback already established), moving the dragged edge by the mouse's
column delta `d` requires a `size_adjust` delta of `2d`, not `d`: growing `size_adjust.0` by `2d`
grows the width by `2d`, which centers by moving *each* edge outward by `d` — exactly matching the
mouse's own movement on the dragged side, with the *other* edge moving oppositely by the same `d`
to stay centered. Only width, not height, got a mouse target — height was never mentioned in the
request, and mirroring it symmetrically for the bottom border can wait until it's actually asked
for. Hit-test priority: the right-border check now runs *before* the title-bar check, since the
top-right corner cell would otherwise match both (the title bar's row check spans the box's full
width, including that column) — resize wins there, matching the general precedent that a precise
edge grab should take priority over a broader "anywhere on this bar" grab.

#### Options Considered: Orientation Toggle Keybinding

**Option A: A new configurable top-level `Action`, bound to a bare key (`t`) intercepted while
the shell is focused** *(implemented, then reverted before verification)*
- This was the first thing built, following the exact precedent `ShellSplitHorizontal`/
  `ShellPaneNext`/etc. already set: a dedicated `Action`, checked in the same `shell_focused`
  branch, intercepted before falling through to pty-forwarding. It compiled, passed
  `cargo clippy`/`cargo test`, and only fell apart once the PTY verification script actually tried
  to type a shell command containing the letter `t` — `touch`, `top`, `tar`, `test`, `git`, and
  countless others would have silently lost every `t` while a real shell had focus, the exact
  category of regression the `space`-prefixed leader chord was built specifically to avoid for
  resize/move (see the entry above). `%`/`"`/`o` already pay a smaller version of this cost
  (rarely-typed characters), and the existing `o`-eats-`ShellPaneNext` case is already a known,
  accepted, if narrow, limitation — but `t` is common enough that this would have been a real,
  frequently-hit regression, not a narrow edge case.
- Caught during this cycle's own verification pass, before ever reporting the feature done —
  writing the PTY test's cleanup step (which needed to type `exit` into a real shell) is what
  surfaced it, since `exit` doesn't contain a `t` but a broader manual check of ordinary command
  typing would have. Reverted in full: the `Action::ShellToggleOrientation` variant,
  `RawKeyMap::shell_toggle_orientation` field/default, its `bind_all` call, the keymap test
  assertion, the `config.example.toml` line, and the `main.rs` dispatch branch were all removed
  again in the same cycle, before commit.

**Option B: A third branch of the existing leader chord (`space t`)** *(chosen)*
- Reuses the exact mechanism that already solves this class of problem: `Action::Leader` only
  ever resolves outside the pty-forwarding path (a focused pane's own keystrokes never reach that
  dispatch at all), so nothing typed into a real shell can ever be intercepted by it, regardless
  of which letter follows `space`. Unlike `r`/`m`, the toggle is a single immediate action with
  nothing to repeat, so it never sets `shell_chord` at all — it fires once and the chord ends
  immediately, no `Esc` needed.

#### Implementation Notes

- New `shell_layout::parent_split_id` (the *immediate* parent split of a leaf, unlike
  `nearest_ancestor_split`, which has no axis to match here) and `flip_direction_in` (mirrors
  `set_ratio_in`'s search-and-mutate shape exactly) back `ShellPanes::toggle_focused_orientation`,
  which flips a `Split` node's `direction` field in place — the two children and their ratio are
  completely untouched, so this only ever changes *how* the same two panes are arranged, never
  *which* panes they are. Still not the pane-reordering this project ruled out in the Multi-Shell
  Layout Model entry; toggling a two-pane split's axis doesn't reorder anything.
- `dragging_shell_width: Option<u16>` joins `dragging_divider`/`dragging_shell` as the third
  mouse-drag state variable in `run`, following the same "store the last position, apply the
  delta, clear on `Up`" shape as `dragging_shell` already does for the title bar.

#### Verification

Confirmed against the real compiled binary via scripted PTY sessions (`pyte`-reconstructed, same
fixes as prior entries). **Mouse resize:** real SGR mouse escape sequences (press, five drag
steps, release) on the box's right border grew its actual width by exactly 10 columns — 2× the
5-column drag delta, matching the symmetric-growth mapping. **Orientation toggle:** opened a
shell, split it side by side (`%`, sent while still focused — `ShellSplitHorizontal` needs that,
same as before), unfocused (`tab`), then `space t`: the internal vertical divider present at a
row through the panes beforehand was completely gone afterward (confirming a stacked, not
side-by-side, layout — a stacked split has a horizontal divider, not a vertical one, so its
absence is the actual signal, not some specific replacement character), and a second `space t`
restored it. A lone (unsplit) pane's `toggle_focused_orientation` returned `false` rather than
panicking, surfaced in the status line as "only one pane — nothing to toggle." The reverted-Option-A
mistake above was never itself PTY-verified as a mistake in the shipped sense — it was caught
during this same verification pass, before any binary built with it was reported working.

---

### Quit Into the Current Directory: `--cwd-file` Plus a Generated Shell Wrapper (Ranger's `--choosedir` Model)

**Date:** 2026-09-19
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

Request: capital `Q` should close minuteman and leave the shell that launched it in the directory
minuteman was browsing, while lowercase `q` closes normally in the directory it started in. A
child process cannot change its parent shell's working directory, so the real question was how
the shell finds out where to go.

#### Options Considered

- **A: `--cwd-file` flag plus a wrapper the user writes by hand.** Ranger's `--choosedir`, yazi's
  `--cwd-file`, lf's `-last-dir-path`. Robust, works in any shell, but the user pastes a function.
- **B: A plus `minuteman init <shell>` printing that wrapper.** Chosen. Same mechanism, but the
  wrapper is versioned and tested with the binary, and setup is one `eval` line.
- **C: `exec $SHELL` in the browsed directory on `Q`.** Rejected: it starts a nested shell with
  the original still waiting underneath, so it doesn't do what was asked. Injecting `cd` through
  `TIOCSTI` was ruled out as well: modern kernels disable it, and it is a security hazard.

#### Decision & Rationale

`Action::QuitToCwd` (default `Q`) writes `browser.current_dir()` as raw bytes to the `--cwd-file`
path and quits; `q`/`:q` never write, so the wrapper's `[ -s file ]` test makes them a no-op.
Without `--cwd-file`, `Q` is a plain quit. Arg parsing moved into `tui/src/cli.rs` as a typed
`Command`/`CliError` instead of `args().nth(1)`, and the wrapper text lives in
`tui/src/shell_init.rs`. The wrapper is named `mm` so the bare `minuteman` binary stays runnable.

#### Implementation Notes

- `theming::keymap::parse_key` lowercased every key string, which would have silently turned a
  `"Q"` binding into `q`. Single-character keys now keep their case; named keys (`"Enter"`,
  `"space"`) still match case-insensitively. Anyone who had written an uppercase letter in their
  config expecting it to mean the lowercase one will see it change meaning.
- Unknown `--flags` and a second positional argument are now errors (exit code 2). Before, any
  first argument was taken as the start directory.
- No property tests or benchmark stub: nothing here is a hot path, and the repo has no `proptest`
  dependency yet, so example-based unit tests cover parsing, key resolution, and the file round
  trip.

#### Verification

Scripted PTY session with a real `zsh -f -i`, the real binary on `PATH`, and the wrapper loaded
via `eval "$(minuteman init zsh)"`. After running `mm` and descending, `Q` left the shell's `$PWD`
at the directory the TUI was in (`root/alpha/beta` after three presses) and `q` left it at the
starting `root`. `minuteman init bash` and `init zsh` output passed `bash -n`/`zsh -n`; the fish
wrapper was not run, because `fish` isn't installed here. `:q` through the wrapper was not
confirmed (the PTY run was inconclusive), but that path is unchanged and never writes the file.
Also observed, not investigated: in this harness the first keystroke after launch never
registered (n presses reached depth n-1, and an extra throwaway key first restored the count). It
is consistent with the startup terminal probe, but I did not confirm whether the previous build
behaves the same.

---

### Single-Leader Mini-Shell Keys: `Space` for Everything, `Esc` the Only Way Out of Typing

**Date:** 2026-09-19
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

Feedback from using the shell panes: the `Tab` focus toggle and the `Space` leader together were
"uncomfortable and unintuitive", and the request was to keep `Space` as the leader, drop `Tab`,
and have only one leader key. Reading the code also turned up a bug behind part of that
discomfort: while a pane was focused, `Tab`, `o`, `%` and `"` were intercepted for pane control,
so none of them could be typed into the shell (`Tab` completion included).

#### Options Considered

- **A: a tmux-style prefix, usable while typing** (e.g. `Ctrl+\`). One key taken from the shell.
- **B: direct Alt chords.** Fastest, but fights readline and vim/tmux-navigator inside the pane,
  which an earlier cycle already rejected.
- **C: a sticky vim-style command mode.**

The user narrowed it: keep `Space`, no `Tab`, one leader. That can't be done while typing, since
a shell needs spaces, so exactly one key has to leave typing mode. `Esc` was chosen for that.

#### Decision & Rationale

Two modes. In typing mode every key goes to the shell except `Esc`, which leaves it. In browsing
mode `space` is the leader: `space space` returns to typing, `hjkl`/arrows move pane focus, `|`
and `-` split (side by side, stacked), `x` closes, and `r`/`m`/`t` are unchanged. Pressing `space`
puts the whole list in the status bar, since one leader hides every binding behind it. Splitting
also returns to typing, because the new pane is where you'd type. `Esc` no longer closes a pane;
that moved to `space x`. Needing no modifier keys avoids the modifier-aware keymap that A
would have required.

#### Implementation Notes

- `theming`: removed `Action::ShellFocus`/`ShellSplitHorizontal`/`ShellSplitVertical`/
  `ShellPaneNext` and their config keys. The chord's second keys are hardcoded in `main.rs`, like
  `r`/`m`/`t` already were. Old configs that still set the removed keys keep loading (unknown
  fields are ignored). The "leader twice" check goes through the keymap, so a rebound leader
  still works.
- `shell_layout`: new `leaf_rects` and a pure `neighbor(rects, from, dir)`: among panes lying
  entirely past the focused pane's edge and sharing part of its perpendicular span, the nearest,
  with ties broken by the closest start edge. It returns `None` at the box's edge rather than
  wrapping. `ShellPanes::focus_direction` wraps it. `focus_next`/`cycle_focus` were removed as
  dead code.
- The literal `Esc` can no longer reach a program inside the pane (vim, for one). It couldn't
  before either, since `Esc` used to close the pane. Left as is; a pass-through can come later if
  it turns out to matter.

#### Verification

`cargo test --workspace` passes (103 tests, 5 new for `neighbor`); clippy is clean. Scripted PTY
session against the real binary with `bash` as `$SHELL`, screen read through `pyte`: `echo o"x"%
a   b` printed `ox% a b` (the freed keys and multi-space input reach the shell); `ech<Tab>` then
` TABOK` ran `echo` (Tab completion works); `Esc` showed the browsing status; `space` showed the
hint line; `space |` produced a second pane that took typing focus; `Esc`, `space h`,
`space space` then typing landed in the left pane, which rendered left of the right one;
`space x` closed it and the right pane remained. Directional focus for `Up`/`Down` and the
`space -` split are covered by the `neighbor` unit tests and by sharing the `|` code path, but
were not separately driven through the PTY.

---

### Neon Theme Engine: Truecolor Hex, a Glowing Active Pane, and File-Type Colors

**Date:** 2026-09-19
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

Request: make the UI less "bland", better than Ranger's, with a cyberpunk-neon, hacker feel. What
was actually plain: only the 16 basic terminal colors, identical square frames on every pane (so
nothing marked the active one), directories blue and everything else white, and a flat selection
bar.

#### Options Considered

- **A: theme engine first.** Truecolor, palettes, per-kind colors, border styles, an active-pane
  glow, a selection stripe. No layout change.
- **B: HUD layout redesign** (header, powerline status bar, columns, scrollbars, icons).
- **C: a cinematic layer** (splash, animation, gradients, system HUD).

A was chosen to go first: it's the foundation B and C reuse, and the biggest visual change for the
least risk. B and C are queued as separate loops.

#### Decision & Rationale

Colors stay strings in `Theme`, so a config can mix basic names and `#rrggbb`/`#rgb`, and the
example config stays a readable spelling-out of the defaults (a test requires it to resolve to
exactly `Theme::default()`). The default became the neon palette; the old look is kept as
`classic` rather than deleted. The active pane is the current directory's column (or the focused
shell) with a bright border and an accent-bold title; the rest are dim. The selected row draws its
stripe and text per span and the list highlight sets only a background, so the stripe and the
file-type color survive the highlight (`selection_fg = "keep"`). Marks stay a `*` — now in the
accent color — rather than a new glyph, so behavior and docs are unchanged. Terminals without
truecolor get hex quantized to the nearest xterm-256 entry (the closer of the color cube and the
grayscale ramp) instead of garbled colors.

#### Implementation Notes

- New `tui/src/style.rs` owns color parsing (`parse_color`, `quantize_256`, `truecolor`), border
  types, `themed_block` (the frame every pane shares, including the mini-shell), and
  `FileKind::classify`. `main.rs::color_from_name` is now a one-line delegate, so existing call
  sites are unchanged. `themed_block` moved out of `main.rs`, and `popup_shell` now uses it, so a
  focused shell is outlined in `border_focused_fg` (before: `selection_bg`).
- `Theme` grew from 7 to 15 fields. Old configs keep working: every field is optional and falls
  back to the chosen palette. A config that set only `selection_bg`/`border_fg` etc. now gets neon
  for everything it didn't set, since `default` is neon.
- Every current-pane row now has a two-cell gutter (stripe, mark), so names no longer shift when
  a row is marked. Parent and preview lists have none.
- Known limitation: quantizing loses hue in dark, low-saturation colors. The dim indigo frame
  (`#3d4270`) becomes a neutral grey (`4e4e4e`) without truecolor. Ok for a border; noted so a
  custom theme for 256-color terminals can pick colors that survive it.

#### Verification

`cargo test --workspace` passes (115 tests, including new ones for hex parsing, quantization,
border-type fallback, file-kind classification, `keep`, and the palettes); clippy is clean. Ran the
real binary in a PTY against a directory of varied files, reading per-cell colors through
`pyte`. With `COLORTERM=truecolor`: rounded corners; the current pane's border `#00f0ff` vs the
parent's `#3d4270`; `alpha/` `#00d9ff`, `main.rs` `#39ff88`, `Cargo.toml` `#ffd60a`, `README.md`
`#b69cff`, `a.zip` `#ff7a3d`, `cat.png` `#ff5cf0`, an unclassified file `#c8ccff`; the selection
stripe `#ff2bd6` on `#2b1a4f`. Without it, the same cells came back as nearby xterm-256 colors
(e.g. `00ffff`, `5fff87`). The mini-shell box showed a rounded `#00f0ff` border and a bold
`#ff2bd6` title. Not verified: how it looks to a human eye on your terminal and font, since I can
only read the cells, not see them, and Nerd Font icons are out of scope until phase B.

---

### HUD Layout: Header, Columns, Scrollbar, and a Mode-Aware Powerline Status Bar

**Date:** 2026-09-19
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

Phase B of the UI overhaul (after the neon theme): make the layout itself richer than Ranger's.
Requested pieces: a header with a breadcrumb path, a powerline-style status bar, size/date
columns, and scrollbars.

#### Decision & Rationale

- **Data first.** Columns and the status bar need size, modified time and permissions, which the
  listing didn't carry. `DirEntryInfo` gained them, filled from the `stat` that `list_dir` already
  made to decide `is_dir`, so this costs no extra syscalls. A broken symlink lists as a
  zero-sized non-directory, as before.
- **A pure core.** Formatting and layout decisions live in `hud.rs` as pure functions
  (`format_size`, `format_age`, `format_perms`, `breadcrumb`, `plan_columns`, `hints`), so they're
  unit-tested without a terminal. Rendering is a thin layer on top, tested by drawing into a
  `TestBackend`.
- **Mode drives the bar.** The status bar's pill and hints follow a `Mode`, computed each frame
  in the same order the key handling checks its states (leader, resize/move, shell typing,
  prompt, busy). That let the ad-hoc status strings for leader/resize/typing go away. Hints are
  looked up from the user's own keymap (`KeyMap::keys_for`), not hardcoded, so a rebound key
  is shown correctly, and whole hints are dropped from the end when the bar is narrow.
- **Columns never cost the name.** `plan_columns` drops age, then size, before letting a name
  fall under 12 cells.
- **Flat by default, arrows opt-in.** Powerline arrows need a patched font and render as
  missing-glyph boxes without one, so `[theme] separator = "arrow"` is opt-in. Same-background
  neighbours get the thin Powerline arrow, since the solid one would match both sides and vanish.
- **Messages expire.** Status text used to stay until replaced. Assignments are spread over ~40
  sites, so `StatusClock` watches the text for change and clears it five seconds after it last
  changed, rather than timestamping each site.

#### Implementation Notes

- Directories show `—` in the size column, and their item count appears in the status bar once
  selected. Counting every directory's entries on every listing would cost a `read_dir` per
  directory, which is slow in big trees. The selected directory's listing is already read for the
  preview pane, so `draw` reads it once and shares it (before, it was read only in the preview
  branch).
- The scrollbar is drawn over the current pane's right border rather than inside it, so it takes
  no width from the list.
- The mini-shell box can still be dragged over the header row: `shell_area` only reserves the
  status bar. Left alone; harmless, and changing it would move the box's tested geometry.
- Not done, by decision: Nerd Font icons (need a patched font, unlike the rest), and progress for
  a single large directory copy beyond a file count (its total isn't known up front).
- The sweep test (every width from 1 to 140, flat and arrow, every mode) caught nothing in the
  header/status layout; the size test did catch that a value just under 1 PiB printed `1024T`,
  fixed by adding a `P` unit.

#### Verification

`cargo test --workspace` passes (137 tests); clippy is clean. Ran the real binary in a PTY over a
directory of 49 entries, read through `pyte` (with the terminal's graphics-probe reply filtered
out, as a real terminal swallows it). At 120 columns: the header showed `~ › hud`; the list
showed right-aligned sizes and ages (`20K 1mo`, `8B 2h`); the scrollbar thumb appeared on the
right border in the accent color; the status bar showed `NORMAL`, name, `rwxr-xr-x`, `1 item`,
`dir`, hints from the default keys, and `1/49`; marking and yanking added `◆ 1 marked` and
`⧉ 1 yanked` pills; `/` showed a `SEARCH` pill with a cursor and `enter ok / esc cancel`; `d`
showed a `DELETE` pill on the danger color (`#ff3860`); opening a shell showed `SHELL`, `Esc`
returned to `NORMAL` with the leader hint, `space` showed `LEADER` with the pane commands, and
`space r` showed `RESIZE`. At 60 columns the age column dropped and hints truncated to fit.
With `separator = "arrow"` in a real config file, the arrow glyphs appeared with the pill's
color as their foreground. Not verified: how it looks to a human eye on your terminal and font.

---

### Glyph Sets, File Icons, and Terminal Snippets: Matching the UI to the Font

**Date:** 2026-09-19
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

Feedback: the letters and symbols "seem out of place" against the neon look. A terminal app
cannot choose its font — the terminal draws every cell in its own — so the fix could only be to
control which symbols the UI uses, and to help the user set the terminal up to match.

#### Options Considered

- **A: a typographic system inside the app** (casing, weight, spacing). Works anywhere, but is
  limited by whatever font is present. Queued as follow-up polish.
- **B: a glyph set plus a recommended font and terminal config.** Chosen.
- **C: draw headings as images in a bundled typeface.** Only on graphics terminals, hard to align
  to the grid, and not selectable; a possible showpiece later.

#### Decision & Rationale

`[ui] glyphs = unicode | nerd | ascii`, defaulting to `unicode` because the app can't detect
whether a Nerd Font is installed, and a missing icon glyph shows as a broken box. Everything the
header, list, scrollbar and status bar used to hardcode moved into one `Glyphs` table
(`tui/src/glyphs.rs`), so a set is data, not scattered conditionals. `nerd` adds file-type
icons and Powerline arrows; `ascii` replaces every frame and symbol, including the pane borders
(via a custom `border::Set`), for a Linux console. The theme's `separator` became `auto`: arrows
exactly when the set is `nerd`, so opting into the font doesn't need two settings, while
`"flat"`/`"arrow"` still force a style. The user gets terminal help rather than being told to
"install a font": `minuteman glyphs` shows what each set looks like in *their* terminal, and
`minuteman init-terminal <kitty|alacritty|wezterm>` prints a font-and-palette snippet. The snippet
palette equals the theme's own neon colors (a test asserts this), so anything drawn in the
terminal's 16 ANSI colors matches the hex theme. Snippets are only printed, never written to the
user's config files.

#### Implementation Notes

- Icons use Font Awesome codepoints (`U+F07B` folder, `U+F121` code, `U+F013` cog, ...), which
  Nerd Fonts v2 and v3 both keep in place, unlike the Material Design range that v3 moved. The
  snippets ask for the *Mono* variant so icons stay one cell wide; the non-Mono variants draw
  them double-width and would misalign the list. An icon adds two cells to the gutter, so
  `plan_columns` now takes the gutter width, and a borderline pane drops a column sooner.
- Icons show on every list, including the parent and preview panes, in the kind's color.
- Kept deliberately small: one icon per file *kind* (7), not per language or extension.
- A first version of the kitty and wezterm snippets came out indented, because an unquoted shell
  heredoc consumed the Rust `\`-newline continuations when writing the file. Caught by running
  the built binary, then rewritten to build lines explicitly, with a test that no line is
  unexpectedly indented.
- The ASCII set still shows `…` when a name or path is truncated, and any non-ASCII file name
  as it is: only the chrome is guaranteed ASCII.

#### Verification

`cargo test --workspace` passes (157 tests); clippy is clean. `minuteman glyphs` printed all
three sets. The alacritty snippet parsed with Python's `tomllib` and the wezterm one loaded as
Lua under LuaJIT; the kitty one has no stray indentation and all 16 colors. On a PTY read through
`pyte`: with `glyphs = "nerd"` the list showed file-type icons in each kind's color (folder
`U+F07B` in the directory color, code, cog, document, archive and file icons), Powerline arrows
between status-bar segments, symbols on the yanked and marked pills, and columns still aligned;
with `glyphs = "ascii"` the entire screen contained no non-ASCII character (`>` stripe, `+--+`
frames, `#` scrollbar thumb, `|` dividers); the default `unicode` set was unchanged at 60 columns.
Not verified: how the icons actually look in any specific Nerd Font on your machine — I can read
the codepoints and colors on screen, not see the glyph shapes.

---

### One Appearance File: Text Styles per Element, a Font Table, and Layering Over the Old Config

**Date:** 2026-09-19
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

Two requests in a row: why does Ranger's text look bold with "its own font", and then "make the
entire TUI appearance (neon, font, bold, italic, ...) configurable through a file specific to
appearance". Checking Ranger's installed `default.py` showed the answer to the first: it has no
font of its own. It sets the bold attribute on directories, executables, tagged and cut/copied
entries, and adds `BRIGHT` to the color — so directories are bold bright blue, drawn in the
user's terminal font. Minuteman's directories were plain, and since terminals only brighten bold
for the 16 basic colors, Minuteman's hex colors got none of that weight for free.

#### Decision & Rationale

- **One file, four tables.** `appearance.toml` holds `[theme]`, `[ui]`, `[style]` and `[font]`.
  Keybindings stay in `config.toml`. `minuteman init-appearance` prints the commented default
  (the same file the tests check against), in the style of the other `init-*` commands: it prints,
  and never writes into the user's home.
- **Styles as data, per element.** Every place that hardcoded a bold now reads a `Mods` set for a
  named element (22 of them). A list *replaces* the element's default rather than merging, so
  there is a way to turn a default off (`dir = []`). The struct, its deserializable twin, the
  defaults, the layering and the list of element names come from one `element_styles!` macro
  invocation, so adding an element is one line and can't drift between those five places. A test
  requires every element to be documented in the example file.
- **Directories and executables bold by default**, matching Ranger. Executable is a modifier
  layered on the kind's style (from the mode bits `list_dir` already reads), not a new color,
  so a `.sh` script stays green and gains weight.
- **The font is honest.** The terminal draws every character, so `[font]` cannot change anything
  inside Minuteman. It exists to feed `init-terminal`, which now prints the user's own family
  and size in the kitty, alacritty and wezterm snippets. Quotes in a family name are escaped so a
  font name can't break the printed config.
- **Layering rather than a break.** `[theme]` and `[ui]` used to live in `config.toml`.
  Moving them would silently reset anyone's look, so they still work there and
  `appearance.toml` wins field by field (`RawTheme::overlay`, `RawUi::overlay`). Each file is
  parsed on its own, so a malformed appearance file keeps the working keybindings and just falls
  back to the default look, with a warning on stderr.

#### Implementation Notes

- New `theming/src/appearance.rs` (`Mods`, `Styles`/`RawStyles`, `Font`/`RawFont`);
  `Config::from_sources(config, appearance)` is the pure, testable core of `Config::load`.
- Unknown modifier names are skipped and a nonsense font size (zero, negative, NaN, absurd)
  falls back to the default, in keeping with the rule that a typo never blocks startup.
- Not done: reloading appearance while the app runs (edit the file, restart), and a
  `[font]` bold/italic face or line height — those vary too much between terminals to print
  correctly without testing each one.

#### Verification

`cargo test --workspace` passes (178 tests); clippy is clean. Ran the real binary on a PTY through
`pyte` and read each cell's attributes: with no appearance file, directories (`alpha/`) and an
executable (`main.rs`, mode 755) were bold and other files plain, with the neon colors; with a
custom file (`dir = ["italic"]`, `doc = ["italic", "underline"]`, `executable = []`, a new focused
border color) a directory was italic and *not* bold (bold only returned on the selected row, from
`selection`), the document was italic and underlined in its new color, `main.rs` lost its bold,
and the border took the new color; with `config.toml` naming the dracula palette and
`appearance.toml` overriding one field, the dracula colors applied and the appearance field won;
with a malformed appearance file the app started with the default look and printed a parse
warning. `init-terminal kitty|alacritty|wezterm` printed `Iosevka Term` at `13.5` from a `[font]`
table, and with a malformed file warned and used the default font. `init-appearance` output is
byte-identical to `appearance.example.toml` and parses as TOML with the four tables. Not
verified: how the italic and underline look in your terminal font, since that depends on the
font having those faces.

---

### Alt Layer for the Mini-Shell Box: One Held Modifier, the Existing Single Box (COA A)

**Date:** 2026-09-20
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

The request was to make `Alt` the leader for every mini-shell command: `Alt`+left-drag moves the
shell and `Alt`+right-drag resizes it; `Alt+hjkl` moves it, `Alt+asdf` resizes it toward
left/bottom/top/right (later changed, see below), `Alt+zxcv` cycles between shells in those directions, `Alt+t`/`Alt+b`
snap to the top/bottom centre, `Alt+q` closes all and `Alt+e` closes the hovered one, and
`Alt+S` makes a new shell. As written it had three problems. `h` and `z` were both described as
"right" (a typo for left). `Alt+s` was both "resize to the bottom" and "new shell". And it talked
about several individually movable shells, while the code has one box holding a tiled split
tree.

It also reverses a deliberate choice. The `space` chord entry in `ROADMAP.md` explicitly refused
`Alt+hjkl` because a shell running inside a pane may use it (tmux navigation), and `Alt+b/f/d/t`
are readline's word motions. Making `Alt` the leader takes all of those away from the shell.
That was accepted, in the user's words "trying out", with the cost stated up front.

#### Options Considered

- **A: keep the single box with tiled panes and add the `Alt` layer over it.** Smallest change,
  nothing shipped is lost. Cost: "the shell" in a move/resize is the whole box, not one pane.
- **B: independent floating windows** (`Vec<FloatingShell>`, per-window rect and z-order,
  tiling removed). The most literal reading of the request, but a rewrite that deletes about
  1,350 lines of tiling code and its tests, and needs overlap hit-testing.
- **C: several floating boxes, each keeping its own split tree.** Independent windows without
  losing tiling, but two levels of "shell" make `Alt+e` and `Alt+zxcv` ambiguous.

I recommended C; the user chose **A** as the easiest and the easiest to revert, which is the
right call for an experiment with a real cost attached.

#### Decision & Rationale

- **`Alt` is parsed in one pure place.** `tui::alt_keys::parse` maps a `KeyEvent` to an
  `AltCommand` and nothing else, so the whole key table is unit-tested without a terminal.
  Hardcoded rather than routed through `KeyMap`, which has no modifier awareness — the same
  precedent as the leader chord's own keys. It is checked before the leader and the typing mode,
  so it works in every mode, and it is skipped only under a text prompt.
- **`Alt+Shift+S` is the split, not `Alt+s`.** Both were asked for; `Alt+s` stays "grow
  downward" (it sits in the `asdf` group) and the split moves to the shifted key, told apart by
  the character's case. One place to change if a different key is preferred. With no shell open
  it opens the first one, so it also replaces `s` from any mode.
- **Revised the same day: `Alt+f` and `Alt+s` shrink, `Alt+a` and `Alt+d` grow.** After trying
  it, the user asked for `f` to make the box narrower and `s` to make it shorter. They reuse the
  mouse's corner resize (`resize_box_corner`), so the top-left stays put, a press is 2 cells,
  and the box stops at its minimum size. Keyboard growth to the right and downward is gone;
  `Alt`+right-drag still does it. The next two paragraphs describe the original all-grow design.
- **`Alt+asdf` grow the box's edge, never a pane divider.** I had said the chord's
  divider-first-then-box fallback would be kept. On reflection that is incoherent for a
  directional edge key: `a` would sometimes move the box's left side and sometimes an internal
  divider depending on the pane layout. Dividers stay reachable by mouse and by `space r`. The
  keys only grow; shrinking is `Alt`+right-drag.
- **Edge growth needed the inverse of `shell_area`.** The box was only ever "centered, then
  nudged by `offset` and `size_adjust`", so growing one side, anchoring a corner or snapping to
  an edge had no direct representation. `shell_params_for` computes the `(offset, size_adjust)`
  that makes `shell_area` return a target rect, and grow, corner-resize and snap all build on
  it. Snapping computes the exact offset instead of pushing it to a huge value and letting
  `shell_area` clamp, because the offset accumulator is never clamped and an inflated value
  would take as many presses to undo.
- **`Alt+e` falls back to the focused pane** when the pointer is not over the box (or has not
  moved yet), so a keyboard-only user is not left unable to close anything.
- **Split direction is picked from the pane's shape.** A pane at least twice as wide as tall
  (cells are about twice as tall as wide) splits side by side, otherwise stacked, so repeated
  `Alt+Shift+S` does not just carve ever-narrower slivers.
- **`Alt+left-drag` reuses the title-bar drag state** (`dragging_shell`), and is matched before
  the divider and border hits, which would otherwise swallow a grab that lands on them. Releasing
  `Alt` mid-drag does not drop the box.

#### Implementation Notes

- `ShellPanes::close_all` is new because dropping a `ShellPanes` does not stop its children:
  `PopupShell` has no `Drop`, only an explicit `close`. It iterates over a snapshot of the leaf
  ids so it terminates even if a close reported `NotFound`. `ShellPanes::focused_rect` is new
  for the split direction.
- `Alt+q` used to be a plain `q` in browse mode and quit the app, because `KeyMap::resolve`
  ignores modifiers. It now reports "no shell open". Not fixed: other `Alt`+letter keys in
  browse mode that are not part of this scheme still resolve as the plain letter.
- Not done: shrink keys, and an `Alt` hint in the status bar (the leader's hints list its keys;
  these are only in the README for now).
- Known risk, not something the code can fix: several window managers (KDE, and i3/sway with
  `Alt` as the modifier) grab `Alt`+drag before the terminal sees it.

#### Verification

`cargo test --workspace` passes (191 tests, 13 new in `tui`); clippy is clean. Ran the real
binary on a PTY through `pyte`, sending real `ESC`-prefixed keys and SGR mouse sequences with the
`Alt` bit set. `Alt+l` ×3 moved the box by 6 columns and back with `Alt+h` ×3; `Alt+j` ×2 and
`Alt+k` ×2 moved it by 4 rows and back. Two presses of each of `Alt+a/d/f/s` moved only the left,
top, right and bottom edge respectively, by 4 cells, with the other three edges unchanged. `Alt+S`
split a 96×27 box side by side, the new pane took focus, `Alt+z`/`Alt+v` moved typing between the
left and right pane (and `Alt+v` at the edge said "no shell that way"), and a second `Alt+S` on
the narrower pane stacked; `Alt+c`/`Alt+x` moved between the stacked panes. With the pointer over
the left pane, `Alt+e` closed that pane and left the others. `Alt+t` pinned the box to row 0 and
`Alt+b` to the row above the status bar, both horizontally centred with the size unchanged, and a
following `Alt+k` moved exactly 2 rows, so no offset slack was left behind. `Alt`+left-drag by
(+7, +3) from the middle of a pane moved the box by exactly that. `Alt`+right-drag by (−10, 0),
(0, −5) and (+6, +3) resized it with the top-left fixed, and `stty size` inside the shell
reported the matching pty size (23×90). With no shell open, `Alt+q` kept the app alive and said
"no shell open" instead of quitting, `Alt+S` opened the first shell, and `Alt+q` with shells open
closed them all. Plain `t b q e h j k l a s d f z x c v` typed into a focused shell arrived
untouched. One harness detail: the first key after startup is swallowed by the terminal
capability probe because nothing on the PTY answers it, so the harness sends one throwaway key
first; a real terminal answers the probe and this does not happen. Not verified: `Alt`+drag in a
real terminal under a window manager that grabs it, and how the keys feel in a real terminal
emulator.

---

## 🧠 Usage Guidelines

Write a new entry here before committing to a major design choice (new dependency, new crate
boundary, UI library, etc.), or when revisiting/reversing a previous decision.
