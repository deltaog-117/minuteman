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

## 🧠 Usage Guidelines

Write a new entry here before committing to a major design choice (new dependency, new crate
boundary, UI library, etc.), or when revisiting/reversing a previous decision.
