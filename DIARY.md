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
| 2026-09-20 | Alt Tap Switches Shell/Browser | Kitty keyboard protocol, enabled only when supported, with an `alt_tap` off switch | ✅ Confirmed |
| 2026-09-20 | Browser Mouse Support | One shared `BrowserLayout` plus pure hit-testing in `browser_mouse.rs`, list state persisted across frames (COA A) | ✅ Confirmed |
| 2026-09-20 | Launch Command | Executable built as `mman` via the `[[bin]]` name; project, crates and config folder keep the Minuteman name | ✅ Confirmed |
| 2026-09-20 | Hidden Files, `:` Commands, Live Refresh | Filter where a listing is stored (COA A); built-ins plus `sh -c` fall-through (COA C); poll by re-listing and comparing, no new dependency | ✅ Confirmed |
| 2026-09-20 | `:` Terminal Handover, `c` Cancel-All, Arrow Keys, Nearest-First Search | Suspend and hand over the real terminal (COA A, name list plus `!`); cancel means everything (COA B); arrows in the default bindings (COA A); breadth-first walk on the blocking pool (COA A) | ✅ Confirmed |
| 2026-09-21 | Right-Click Menu, Inspect Panel, Open With | Menu and modal panel inside `tui` with pure geometry, config-driven Open with, detached launch (COA B); `.desktop` discovery, multi-select clicks and drag and drop deferred | ✅ Confirmed |
| 2026-09-21 | Richer Status Line: Marked Size and Git | Marked total in the header pill via the Inspect walk; git segment from `git status --porcelain=v2` run off-thread with `--no-optional-locks` (COA A); `gix` and a marked-size-only cut rejected | ✅ Confirmed |
| 2026-09-21 | Preview Extras, Stage 1 | Scrolling preview (`J`/`K`, wheel), hex view of the first 64 KiB, in-process zip/tar/tar.gz listing with every read bounded (COA A); shelling out to `bsdtar` and hand-parsing rejected | ✅ Confirmed |
| 2026-09-21 | Disk Usage View | Modal `du`-style view scanning one folder at a time off-thread, on-disk size by default, hard links once, same filesystem only, symlinks not followed (COA A); a size column (B) and a cached tree with delete (C) rejected | ✅ Confirmed |
| 2026-09-22 | Adaptive Default Theme | OSC 11 terminal-background query via `ratatui-image`'s existing probe, resolved to Catppuccin Mocha/Latte (COA A); reading the OS/DE's light/dark setting (B) and a `$COLORFGBG` heuristic (C) rejected | ✅ Confirmed |
| 2026-09-22 | Appearance Popup Scope and Mouse Model | Curated 6 colors + 3 cycle fields, real text entry, `ContextMenu`-style click/hit-testing (COA C); growing the settings popup's cycle-only rows (A) and a full category-submenu editor for every field (B, deferred to the roadmap) rejected | ✅ Confirmed |
| 2026-09-22 | Persisting the Settings/Appearance Popups | A third, program-owned `local.toml` layered above `config.toml`/`appearance.toml` (COA B); an in-place `toml_edit` rewrite of the hand-edited files (A, deferred) and a stripped-and-reappended generated block (C) rejected; Theme row moved from the settings popup into the appearance popup | ✅ Confirmed |
| 2026-09-23 | Plugin System Transport | Out-of-process, line-delimited JSON-RPC over stdio, any language, no compile step (COA B); a sandboxed WASM/Extism host (A) deferred to sit alongside it later, an in-process Lua-only tier (C) rejected as too narrow | ✅ Confirmed |
| 2026-09-23 | Appearance Popup: Color Picker + Saved Themes | Extend the existing popup and `local.toml` with an HSV picker mode and named full-snapshot theme saves (COA A); a separate Theme Manager popup plus standalone picker overlay (B) and a swatch-grid-only version with no automatic update/new detection (C) rejected | ✅ Confirmed |
| 2026-09-23 | Built-in Trash | Delegate to the real OS/desktop trash via the `trash` crate (COA B), at the user's direction; a hand-rolled trash folder over `Vfs`/`file_ops::mv` (A) and a full restore-by-id subsystem with its own listing popup (C) rejected; a non-local `Vfs` falls back to permanent delete, since the desktop trash has no remote equivalent | ✅ Confirmed |
| 2026-09-25 | Preview Syntax Highlighting | A curated seven-`TokenKind` classification of `syntect` scopes (one small shared palette, not one theme color per possible scope), tokenized off the render thread alongside the file read | ✅ Confirmed |
| 2026-09-25 | External Preview Hooks | A new `tui::preview_hook` layer (COA A) reusing `ImagePreview`/`TextPreview`'s existing pipelines, at the user's direction — over pushing hook config/execution into the `preview` crate (B) or an image-only stage deferring text hooks (C) | ✅ Confirmed |
| 2026-09-26 | UI Overhaul, Phase C (Start) | Animation tick plus animated shell-pane focus-border transitions (COA A); a pulsing selection was built, verified, and cut on taste alone before committing | ✅ Confirmed |
| 2026-09-27 | UI Overhaul, Phase C (Remainder) | Boot splash, gradient borders/titles, typewriter preview reveal, system/git HUD off `/proc` with no new dependency (COA A, measured against the `sysinfo` crate's real +276 KiB/8-crate cost first); pulsing selection dropped from scope for good | ✅ Confirmed |
| 2026-09-27 | Caps Lock Header Pill | Static header pill kept in sync from every key event's `KeyEventState` (COA A); a blinking/pulsing version, matching the plain reading of "loop it," was raised and rejected at the user's own clarification, consistent with the pulsing selection already cut for taste | ✅ Confirmed |
| 2026-09-28 | Bookmarks / Marks | A `main.rs`-local pending-chord (mirroring `shell_chord`) plus a new `RawLocal::bookmarks` field reusing `local.toml` (COA A), over a heavier `Prompt`-variant UI (B) or a whole new file format and picker overlay (C) | ✅ Confirmed |
| 2026-09-28 | Full Appearance Editor | Category submenu plus a real two-axis gradient-square/hue-strip color picker (COA A), over a single long scrolling list with the same picker (B) or a category menu with H/S/V slider bars (C) | ✅ Confirmed |
| 2026-09-28 | Saved Custom Themes | A "Saved themes" category listing saved themes, with apply, rename and delete (COA A), over making the Theme row cycle them (B) or a manage view opened from the Save row (C) | ✅ Confirmed |
| 2026-09-28 | Archive Compress/Extract | In-process `zip`/`tar`/`flate2` in `file_ops::archive` with a `SafePath` newtype and extraction limits (COA A), over system tools (B) or a hybrid (C) | ✅ Confirmed |
| 2026-09-28 | Compress Form | A modal form popup on the right-click menu with pure state and a validated `Request` (COA A), over a format submenu into the prompt bar (B) or a chain of prompts (C); password deferred | ✅ Confirmed |
| 2026-09-28 | Extract Form | A modal form popup on an archive's right-click menu with pure state and a validated `Request` (COA C, the user's pick of the compress-form shape), over a single "Extract" item (A) or an "Extract here / to…" submenu (B) | ✅ Confirmed |
| 2026-09-29 | Mouse Stage 2 | `Ctrl`-click toggles, a configurable range modifier (`Shift` or `Alt`, both by default) marks a span, header path segments and middle-click navigate (COA B), over the roadmap's `Shift` only (A) or a two-cycle split (C) | ✅ Confirmed |
| 2026-09-29 | Mouse Stage 3 | Drag and drop with a pure state machine, a pointer label and lit target, drops on both file columns, up a level, and shell panes, plus edge auto-scroll (COA C), over a release-only drop (A) or feedback without shell and scroll (B) | ✅ Confirmed |
| 2026-09-29 | Open With | Read the shared MIME database, `mimeapps.list`, `mimeinfo.cache` and `.desktop` files in-process, with `[[open_rule]]`, `enter`-to-open and "Other…" (COA A), over shelling out to `xdg-mime`/`gio` (B) or a hybrid launching through `gio` (C) | ✅ Confirmed |
| 2026-09-30 | VFS Hardening | Complete `Vfs` (metadata, links, reads, scans, `local_path`), move the readers, prove it with an in-memory backend and a conformance suite, gate new direct disk use (COA A), over archives through `Vfs` (B) or guardrails only (C) | ✅ Confirmed |
| 2026-09-30 | Archives Remainder | Keys, save-in folder, live warnings, `:` conflict prompts and `-p`, zip mtime, directory bound, all without new dependencies (COA A), over extra codecs (B) or browsing inside archives (C) | ✅ Confirmed |
| 2026-09-30 | Encrypted Archives | Zip AES-256 through the `zip` crate, a password prompt on extract, a wiped-on-drop `Password` type (COA A), over 7z with header encryption (B) or a standalone `age` action (C) | ✅ Confirmed |
| 2026-09-30 | Undo History | Inverse-operation journal in `file_ops::history`, batches as one step, conservative reverts, trash restored through the `trash` crate (COA A), over a holding area for deleted data (B) or a reverse-only popup (C) | ✅ Confirmed |
| 2026-09-30 | Default Key Remap | `d` cut, `x`/`X` delete, `s` unbound, `y`/`d`/`x`/`X` act on marks only | ✅ Confirmed |
| 2026-10-01 | Archives Through Vfs | Four write primitives on `Vfs` (`create_write`, `create_symlink`, `set_mode`, `replace`) and archives read and written through them, with a conformance suite for each | ✅ Confirmed |
| 2026-10-01 | Remote Browsing | Drive the system `ssh -s sftp` through `openssh` and `openssh-sftp-client`, route by a path prefix in one `Vfs`, and stream copies between backends (COA C), over `ssh2` and libssh2 (A) or `russh` (B) | ✅ Confirmed |
| 2026-10-02 | Browsing Archives | A read-only `Vfs` wrapper (`vfs_archive`) addressed by a `!` suffix on the archive path (COA A), over extracting to a temporary folder (B) and a zip-only first cut (C) | ✅ Confirmed |
| 2026-10-02 | Filmstrip View | A `V` view cycle with a thumbnail strip drawn from a pool-filled cache of finished protocols (COA B, filmstrip first, grid second), over a grid only (A) and a bigger preview only (C) | ✅ Confirmed |
| 2026-10-02 | Grid and Details Views | A grid of framed tiles and a flat details table added to the `V` cycle, the filmstrip kept and its strip rebuilt from the same tile (COA A, with the filmstrip kept as asked) | ✅ Confirmed |
| 2026-10-04 | Trash Browser and Marks-Only Operations | A popup view on `T` that acts only on `v` marks (COA B), over a virtual folder (A) and `trash-cli` (C); every operation reads marks alone and spends them | ✅ Confirmed |
| 2026-10-04 | Instant Start | Splash off by default; with it on the browser is drawn underneath and a key passes through | ✅ Confirmed |
| 2026-10-04 | Fuzzy Search | Own subsequence scorer; nearest level first, best matches within it, top 20 to step through (COA A), over ranking the whole walk (B) and a matcher crate (C) | ✅ Confirmed |
| 2026-10-04 | Sorting, Local Time, Icons, Page Keys, Prompt Editing | Sort kept in `BrowserState` and applied when a listing is stored; `chrono` for local time (A); a shared `LineEdit` for prompts (A) | ✅ Confirmed |
| 2026-10-04 | Background Folder Moves and Remembered View | A planned-then-applied move loaded on the blocking pool with a 40 ms grace, old folder kept until it arrives (COA A), over an empty target shown at once (B) and a worker-thread browser (C); the view saved in `local.toml` | ✅ Confirmed |
| 2026-10-04 | Default Theme Source | The terminal's own palette (OSC 4/10/11), mapped with contrast lifting, over a static palette (A) and a palette file (C) | ✅ Confirmed |
| 2026-10-04 | Plugin Triggers (stage 1.5, first cycle) | Several keys, declared `:` commands and `:plugin <name>` on the existing process host, plus a Python example (COA A); state/events (B) and action names (C) deferred | ✅ Confirmed |
| 2026-10-05 | Appearance Shape and Paged Popup | A typed `Shape` inside `Theme`, gaps applied in the shared `browser_layout`, and the popup rebuilt as a page list beside rows (COA B); a Shape category in the old popup (A) and shape presets only (C) rejected | ✅ Confirmed |
| 2026-10-05 | Configurable Border Gradient | Colors with `accent`/`focused` keywords in `[theme]`, scope and direction in `[shape]`, the old switch kept beneath (COA B); a moved switch alone (A) rejected, an animated gradient (C) deferred to the roadmap | ✅ Confirmed |
| 2026-10-06 | Plugin `run_action` | One more turn of the render loop that enters the browser's own key handling as a `Browse` step, not a second dispatcher for plugins | ✅ Confirmed |
| 2026-10-06 | Plugin Cost Benchmarks | Ignored benchmark tests beside the integration tests, run by hand with `--release`, no stored baseline (COA A); a criterion bench (B) and a docs-only close-out (C) rejected | ✅ Confirmed |
| 2026-10-09 | Atlas Storage Map | Lazy zoom on the disk usage scanner with a pure whole-cell treemap (COA A); a cached whole-volume tree with a minimap (B) and a volumes-only map (C) not taken | ✅ Confirmed |
| 2026-10-09 | Wiring Diagram Fit | Fit by budget: a cap on siblings and a squeeze on box width chosen so the whole diagram fits (COA A); a selection-dependent focus layout (B) and clipped panning (C) not taken | ✅ Confirmed |
| 2026-10-10 | Operation Keys on Marks and Cursor | Marks act where you stand, the cursor row is the fallback, `space x` for the trash (COA A); a cursor fallback alone (B) and a same-folder guard (C) not taken | ✅ Confirmed |
| 2026-10-10 | Deck Rail | A grouped places-style rail at the left, entered with `Tab` and `↑`/`↓`, `hjkl` left to the files (COA A); stacked cards (B) and a rail that folds to icons (C) not taken | ✅ Confirmed |
| 2026-10-10 | Browser File Chooser | A native portal backend, `mman-portal`, opening `mman --chooser-file` in a terminal (COA B); `xdg-desktop-portal-termfilechooser` with a wrapper script (A) and only a default folder handler (C) not taken | ✅ Confirmed |
| 2026-10-11 | Core and Bundled Plugins | Move features out one at a time into bundled plugins, starting with a scaffold that changes no behavior (COA C: in-process traits and the plugin protocol); in-process traits alone (A) and everything over the protocol (B) not taken | ✅ Confirmed |
| 2026-10-11 | Fuzzy Finder as a Plugin | The interface walks the tree and draws the list, the plugin ranks (COA B); a plugin that walks the tree itself through `read_dir` (A) and keeping the finder in-process behind a trait (C) not taken | ✅ Confirmed |
| 2026-10-11 | Structured Search Plugin | A second bundled plugin on the picker slot, filters over the walk (COA C, delivered as B first); `/` and `f` kept as they are; content search (A) deferred | ✅ Confirmed |
| 2026-10-11 | Inspect as a Plugin | A `stat` request, a `tally` job and a `set_inspect` document, rows worded by the plugin (COA A); a plugin that only formats what the core gathered (B) and a generic multi-line pane view (C) not taken | ✅ Confirmed |
| 2026-10-11 | Archives Served by the Plugin | The host asks the archive plugin one request per `Vfs` call and the plugin answers; read-only, files read through handles tied to the plugin's process | ✅ Confirmed |
| 2026-10-12 | Ssh Served by a Plugin (M7b) | `mman-ssh` serves reads and writes on the archive plugin's wire, staged in three steps so remote writes never regress (COA A); keeping `vfs_ssh` in the core behind a feature (C) not taken | 🟡 Step 1 of 3 |
| 2026-10-12 | Ssh Plugin Built, Not Wired (M7b step 2) | The `vfs_ssh` engine copied into `plugins/ssh` behind a `Server`, a host-side `SshVfs` in a new core crate, hex-over-JSON windows kept and measured (COA A); binary frames (B) and a rewrite on a lighter ssh stack (C) not taken | ✅ Confirmed |
| 2026-10-12 | Ssh Cut-Over (M7b step 3) | One cut-over: `SshVfs` wired in, `vfs_ssh` deleted, the install line shown when `mman-ssh` is absent (COA A) | ✅ Confirmed |
| 2026-10-12 | Canvas Primitive for Whole-Body Views (M7c step 1) | A plugin draws atlas and wiring through a `canvas` of semantic parts and claimed keys; grid, tiles, details and filmstrip stay in the core (COA C); the core atlas stays as the fallback until the plugin exists | ✅ Confirmed |
| 2026-10-12 | The Wiring Diagram as a Plugin (M7c step 3) | The plugin owns the scan, the folders, the layout and the device actions on the local disk, with the load and the browsed folder sent as a `context` (COA A); snapshots fed from the core (B) and keeping the diagram in the core (C) not taken | ✅ Confirmed |
| 2026-10-13 | Folder Listings Off the Render Thread | A background `DirPreview` for the cursor folder and `App::resync`, which re-reads a local folder on the spot and hands a slow one to the live refresh (COA B); a plugin-side refreshing index (A) and a progress gauge with an on-disk index cache (C) not taken | ✅ Confirmed |
| 2026-10-13 | History Popup | Go back to a chosen entry by walking the existing undo worker, never undoing one entry out of order (COA B) | ✅ Confirmed |
| 2026-10-14 | Opening a Remote File | Fetch a copy into an app-owned cache folder on the background worker, then open the copy; edits stay in the copy (COA 1); write-back (2) and an `sshfs` mount (3) not taken | ✅ Confirmed |
| 2026-10-15 | Giving Up on a Remote Read | A per-thread cancel token, a `vfs_cancel` line, and one shared connect per machine that only read-only work may abort (COA B); a plugin-only shared connect (A) and killing the connecting `ssh` (C) not taken | ✅ Confirmed |
| 2026-10-16 | Answering `ssh` When It Asks | A plugin -> host `ask` request and a modal, with `ssh` run by the plugin without batch mode and `mman-ssh` itself as `SSH_ASKPASS` (COA A, route A1); `-o` options only (B) and a hand-written `known_hosts` with a pty password (C) not taken | ✅ Confirmed |
| 2026-10-18 | Plugin Menu, First Slice | A core popup (`space p`) that switches any plugin on and off live and keeps the switched-off names in `local.toml` (COA B, step one); installing and fetching plugins from the menu rejected (COA C) | ✅ Confirmed |
| 2026-10-18 | Plugin Menu, `git` and `hud` | Four small cycles in a set order (COA A); the menu switches `git` and `hud` through `local.toml`, with `git_status = false` kept as a veto; one cycle for all of it (C) and own plugins first (B) not taken | ✅ Confirmed |
| 2026-10-18 | Plugin Menu, Own Plugins in `local.toml` | `[[plugin]]` tables in `local.toml` merged after `config.toml`'s, `d` removes them with a question; one approach, no COAs asked (the roadmap fixed it) | ✅ Confirmed |
| 2026-10-18 | Plugin Menu, the Add Form | A multi-row text form modelled on the compress form, refusing clashes and unstartable programs before anything is written (COA A); a one-question-at-a-time prompt (B) and a `:plugin add` command (C) not taken | ✅ Confirmed |
| 2026-10-18 | Plugin Menu, Deleting a Program | A `d` key and an inline `y`/`n` question naming the file, its size and the reinstall command; only a plain file of the running user (COA A); moving it to the trash (B) and printing the commands only (C) not taken | ✅ Confirmed |
| 2026-10-18 | Plugin Authoring, Manifest and Drop-ins | A `plugin.toml` with a `[[plugin]]` table's keys, found in `plugins/<name>/` of the config directory and listed off until switched on (COA B); the manifest alone (A) and the full `mman plugin` CLI first (C) not taken | ✅ Confirmed |
| 2026-10-18 | Plugin Menu, `plugin_dirs` | A read-only `plugin_dirs` list in `config.toml`, scanned one level deep after the browsed folder (COA A); a list the picker adds to (B) and background scanning into the roster (C) not taken | ✅ Confirmed |
| 2026-10-18 | Large `.tar.gz` Archives | A 120 s time budget instead of the 512 MiB cap, and the listing of a slow one kept in the cache folder (COA A) | ✅ Confirmed |
| 2026-10-18 | Plugin Start Check | A passive half-second probe shared by the review card and `mman plugin check`, plus an optional `api_version` in the manifest (COA A); an acknowledgement message (B) and a static-only check (C) not taken | ✅ Confirmed |
| 2026-10-19 | Plugin Quick-Start Page | One short `docs/plugins.md` written from the commands as run, linked from the README (COA none: documentation only) | ✅ Confirmed |
| 2026-10-22 | Rust Example WASM Plugin | A standalone crate outside the workspace with only `serde_json`, run by a test that skips when the `wasm32-wasip1` target is missing (documentation and test only, no COAs) | ✅ Confirmed |
| 2026-10-23 | Undo for Compress and Extract | A new `Change::Produced` that undoes by trashing the output and redoes by restoring it, not `Created` (which only removes empty files) | ✅ Confirmed |
| 2026-10-25 | Undo History Between Sessions | A hand-written line format in `file_ops::history`, saved atomically by one writer thread in the state folder and checked against the trash on load (COA A) | ✅ Confirmed |
| 2026-10-21 | Sandboxed WASM Plugins | `wasmi` behind an off-by-default `wasm` feature, one launch decision, a read-only trust check with no wildcard arm, and memory and idle-time limits on a thread of its own (COA A); `wasmtime`/Extism (B) and a permission manifest with writes (C) not taken | ✅ Confirmed |

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

> **Later the same day:** new shell became `Alt+n` and close-pane `Alt+m`; `Alt+Shift+S` and
> `Alt+e` below are the original names. See the Alt-tap entry after this one.

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

### Tapping Alt Alone Switches Shell and Browser: the Kitty Keyboard Protocol, Behind an Off Switch

**Date:** 2026-09-20
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

The request: pressing `Alt` alone alternates between the mini-shell and the file manager; `Alt+n`
opens new shells; `Alt+m` closes them. The second and third parts are key changes. The first
is not, because an ordinary terminal sends *nothing* when a modifier is pressed by itself — there
is no byte to read. The only mechanism is the kitty keyboard protocol, which reports modifier
keys and key releases. The user's terminal is kitty, so it is available to them; it is not
universal (kitty, foot, wezterm, ghostty and recent alacritty implement it).

#### Decision & Rationale

- **The tap is a release, not a press.** Toggling when `Alt` goes down would also fire on every
  `Alt+n`. Instead an `Alt` press arms a flag, any other key press or mouse press disarms it, and
  an `Alt` release that still finds it armed toggles. So `Alt+n`, an `Alt`-drag, and `Alt` then
  `x` never switch modes; only a lone press-and-release does. Plain mouse motion does not
  disarm it. There is no time limit, so holding `Alt` for a while and releasing it still counts.
- **What "alternate" means:** it flips whether keys go to the shell or the browser, the switch
  `Esc` and `space space` already make, and does nothing with no shell open or a prompt showing.
- **Ask first, then enable, and give it back.** The terminal is asked whether it supports the
  protocol; only then are the flags pushed, after the graphics-protocol probe so neither reads
  the other's reply, and popped again when the terminal guard drops, including on a panic.
  On a terminal without it nothing is enabled, the ESC-prefixed `Alt` keys keep working, and the
  tap simply does nothing.
- **All four flags are required, and each has a reason.** Reporting every key as an escape code
  is what makes a bare modifier visible at all; event types supply the release that ends a tap;
  alternate keys make `Shift`+letter arrive as a capital, since without them crossterm reports a
  lowercase letter plus a shift bit and capitals typed into a shell would come out lowercase;
  disambiguate is the baseline the others build on.
- **Repeats are typing.** With event types on, a held key produces repeat events and every key
  produces a release. The event loop used to drop anything that was not a press, which would
  have stopped held keys from repeating. It now ignores releases only.
- **`Alt+n` is the split and `Alt+m` closes the focused or hovered pane.** They replace
  `Alt+Shift+S` and `Alt+e` outright rather than adding aliases, taking "responsible for" to
  mean the key moves. `Alt+q` still closes everything. If `Alt+m` was meant to close all, it is
  one line in `alt_keys::parse`.

#### The Risk, and Why There Is an Off Switch

Crossterm cannot request the protocol's associated-text flag, so with every key reported as an
escape code the application receives key codes, not composed text. A dead key or `AltGr`
sequence (typing `ã`, `ç` or `é` on some layouts) may therefore arrive as its base letter. I
could not test this against a real kitty, and the user is likely to type such characters. The
top-level `alt_tap` option in `config.toml` (default `true`) skips the protocol entirely, and
its comment in `config.example.toml` says why. The fallback is complete: `Esc` and the `space`
leader still switch modes.

#### Verification

`cargo test --workspace` passes (`theming` 33 tests, `tui` 114); clippy is clean. The PTY harness
now answers the protocol query the way kitty does and sends real protocol events. A bare `Alt`
press and release toggled between the SHELL and NORMAL modes in both directions; `Alt+l` and
`Alt+h` as full press-key-release-release sequences, an `Alt`-held mouse drag, and `Alt` then `x`
then `Alt` released each left the mode alone; `Esc` still left typing; a tap with no shell open
did nothing and the app stayed alive. Text with capitals, including `Zz` and `Q`, reached a real
bash intact, and a held key sent as three repeat events typed three extra characters. `Alt+n`
with no shell opened the first one and with one open split it; `Alt+m` over the left pane closed
just that pane (2 to 1) and on the last pane closed the box; the retired `Alt+e` did nothing.
The protocol was pushed exactly once, as flags 15, and popped on exit. With a harness that never
answered the query no flags were pushed and ESC-prefixed `Alt+l` and `Alt+n` still worked; with
`alt_tap = false` no flags were pushed either. Not verified: a real kitty, and above all whether
composed characters survive the protocol there.

---

### Mouse in the File Browser: One Shared Layout and a Pure Hit-Test Module (COA A)

**Date:** 2026-09-20
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

Mouse capture was already on, but only for the mini-shell box: with no shell open the event
handler returned at once, and with one open it handled box gestures only. Making the three file
columns clickable had two obstacles. The column rectangles were computed inside `draw`, so the
event loop could not hit-test them, and the middle column's `ListState` was rebuilt on every
frame, so nothing remembered which entry was at the top of a scrolled list. Without that, a click
on screen row 5 cannot be turned into an entry index.

#### Options Considered

- **A: a pure layout function and hit-test module, list state persisted.** `draw` and the mouse
  handler both call `BrowserLayout::split`, so there is one source of truth for where things are.
  Hit-testing is a pure function and can be fuzzed. Costs a small change to `draw`.
- **B: `draw` records the rectangles and offset it used, and the event loop reads them.** Always
  matches the screen, smaller diff, but the click lags a frame behind, `draw` gains a side
  effect, and it is hard to test without a terminal.
- **C: a general hitbox layer every region registers with.** Extensible, but far too much for
  three panes and would have rewritten the working shell-mouse code.

**Choice: A**, as recommended. Double-click was to open directories only, since there is nothing
to open a file with yet; open-with was added to the roadmap for that.

#### What Was Built

- `crates/tui/src/browser_mouse.rs`: `BrowserLayout::split` (the same header / 20-40-40 columns /
  status split `draw` used before), `hit_test` (returns a `Hit`: a parent row, a current row, a
  blank part of a pane, or elsewhere), `ClickTracker` (400 ms window, time passed in so the edge
  is testable), `wheel_target` (three entries a notch, clamped) and `apply_click`.
- `main.rs`: the mouse arm now offers the event to the browser first, but only when the pointer
  is not over the shell box, no drag is in progress (a drag that leaves the box must still end on
  it) and no prompt is open. `draw` takes the layout from `BrowserLayout` and the list state from
  `Overlay`, which already existed to keep `draw`'s argument list from growing.
- `theming`: a top-level `browser_mouse` option (default `true`), the same shape as `alt_tap`,
  so the whole feature can be switched off without a rebuild. The shell box's own mouse
  gestures do not depend on it.

#### Decisions Inside the Decision

- **A click in the left column goes up and selects the entry clicked**, as proposed, rather than
  jumping into it. A double-click there is ignored: the first click shifts every column, so the
  second lands on a different entry than the one aimed at, and acting on it would climb twice.
- **Clicking anywhere in the browser takes the keyboard back from a focused mini-shell.** Not
  discussed in advance. Without it a mouse user would click a row and then find `j` typing into
  the shell. It matches what tapping `Alt` does.
- **The wheel over the preview column does nothing yet.** That column is about to get its own
  scrolling (preview extras, stage 1), and the wheel belongs to it there.
- **Failures go to the status bar.** Entering a directory that cannot be listed sets a status
  message instead of returning the error, which would have ended the program. The key handler
  for `enter` still propagates the same error; I left that alone as out of scope.
- **No benchmark stub and no tracing.** The manifesto asks for both. `tui` is a binary crate, so
  a benchmark cannot import the module, and the hit test is a constant-time computation on a
  handful of rectangles. The workspace has no tracing dependency and adding one for this alone
  would be the wrong place to start.

#### A Behaviour Change to Know About

Persisting the list state changes keyboard scrolling slightly. The state used to be rebuilt each
frame, which by ratatui's rules should keep the selection pinned to the bottom edge once past the
first screen; I inferred that and did not run the old binary to confirm it. Now the list scrolls
only when the selection reaches an edge: after scrolling down and pressing `k` three times the top
row stayed where it was, and three `j` presses at the bottom edge scrolled it by three.

#### Verification

`cargo test --workspace` passes (`theming` 34 tests, `tui` 135, up from 33 and 114). The 21 new
`tui` tests include five property tests: the layout regions stay inside the area and never
overlap; a hit never names an entry the list lacks and always the one drawn under the pointer;
every visible row is clickable; the wheel stays in range and heads the right way; and a
double-click needs the same row inside the window and never chains. Clippy is clean.

The real binary was driven on a PTY at 100 by 30 with synthetic SGR mouse events, and the screen
read back through `pyte`. A click on row 4 selected `f03.txt` and its contents appeared in the
preview; a double-click on a directory entered it and the header named it; a double-click on a
file left it selected and stayed put; the wheel over the middle column moved the selection by 3,
and down-down-up ended on index 3; the wheel and clicks on the preview column and the header did
nothing. A click on `f03.txt` in the left column, from inside a directory, went up and selected
it, and a double-click in the left column two levels down climbed exactly one. In an 80-entry
directory scrolled so the top row was `g11`, a click on the fourth visible row selected index 14,
so the hit-test agrees with what is drawn once the list has scrolled. With a shell open and
focused, a click on the preview column outside the box moved the keyboard back so `j` moved the
selection, while without the click `j` went into the shell. With `browser_mouse = false` the same
click did nothing.

Harness detail worth keeping: the first attempt failed every "does something" check and passed
every "does nothing" one. The harness was not answering the startup probes (keyboard protocol and
graphics), and the app's probe readers swallowed the first bytes sent. This is the same effect
the previous entry recorded as "the first key after startup is swallowed". Answering the two
queries the way a kitty-protocol terminal does removed it, and a real terminal answers them
anyway. Not verified: a real mouse in a real terminal, and terminals that report the wheel or
button events differently from the SGR encoding used here.

---

### The Launch Command: Build the Binary as `mman`, Keep Everything Else Minuteman

**Date:** 2026-09-20
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

`minuteman` is a mouthful to type every time a file manager is opened, and the goal was a short
command that works for every user, not an alias in one person's `~/.zshrc`. I proposed three ways
to ship one: a second `[[bin]]`, a symlink made at install time, or a second name defined by the
shell wrapper. Each was rejected in turn, and the reasons are worth keeping.

#### What Was Tried and Measured

I tried the alternatives on a throwaway copy of the repo instead of arguing from memory.

- **Two `[[bin]]` entries on one `main.rs`** builds both binaries, but Cargo warns on every build
  that the file is "present in multiple build targets", and compiles the crate once per binary.
- **A library plus two three-line binaries** (`main.rs` becomes `lib.rs`, one line changes from
  `main` to `run_cli`) builds with no warnings and all 135 tests passed unchanged. It is the
  layout the Rust template asks for, but each installed binary carries a full copy of the app.
- **Sizes, release build:** 4.48 MB for `minuteman`, so about 9 MB for two, against 0.36 MB for a
  launcher shim that only runs the sibling binary. This corrects my first estimate, where I
  said the library split would avoid doubling: it saves compile time, not installed size.
- **The symlink and wrapper-only options** cost nothing in size, but `cargo install` cannot make a
  symlink, and a shell function is not a real executable.

#### Decision

Name the binary `mman`, and only that. Cargo keeps the binary name independent of the package
name, which is the same route ripgrep (`rg`), Helix (`hx`) and Nushell (`nu`) take. There is one
executable, no duplicate, no warning, `main.rs` stays the entry point it was, and
`cargo install --path crates/tui` installs exactly `mman`. That last point was checked with a
real install into a scratch directory.

`mman` over `mm`: it reads as Minuteman, and a two-letter name is the likelier one to collide
with a user's own alias or another package. The cost is that a web search for "mman" lands on
`sys/mman.h`. I did not check the distro repositories for a package called `mman`; someone should,
with `pacman -F bin/mman` or `apt-file search bin/mman`, before this is published.

#### What Changed, and What Deliberately Did Not

Changed: the `[[bin]]` name in `crates/tui/Cargo.toml`; the shell wrapper, which is now a function
called `mman` that reaches the binary with `command mman` (a function that shares the binary's
name would call itself forever without `command`, so a new test pins this, and pins that the old
name is gone); the README, wherever it tells a person to run a command; and two lines of
`config.example.toml` that named the wrapper. The `mm` wrapper and the `minuteman` executable no
longer exist, which matters to nobody yet since nothing was released.

Not changed, on the instruction to leave mentions of the name in the code alone: the project
name, the crate layout, the license headers, and the config folder `~/.config/minuteman/`, so no
one has to move a file. Code comments, help strings and the appearance example still say
`minuteman init`, `minuteman glyphs` and `minuteman init-terminal` in `cli.rs`, `keymap.rs`,
`appearance.rs`, `glyphs.rs`, `terminal_init.rs`, `shell_init.rs`, `main.rs` and
`appearance.example.toml`. Those are now stale as instructions and are the obvious follow-up if
the rule is ever relaxed. The error prefixes (`minuteman: ...`) also still name the app.

#### Verification

`cargo test --workspace` passes (`tui` 136 tests, one new); clippy is clean. In a real zsh with
only `mman` on `PATH`, driven through a PTY, `whence -w mman` reported a function, `Q` after
entering a directory left the shell in that directory, and `q` left it where it started. Not
verified: the bash and fish wrappers were only checked as text by the unit tests, never run in
those shells, and `mman` has not been tried outside the test harness on a real desktop.

---

### Hidden Files, `:` Commands and Live Refresh: Filter Where It's Stored, Built-ins Plus `sh`, Poll by Re-listing

**Date:** 2026-09-20
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

Three requests arrived together: a key to show or hide hidden files, running commands such as
`mkdir`, `touch` and `cd` from the `:` prompt without opening a mini-shell, and lists that update
by themselves when something is created by a shell or a command instead of only after the user
visits another panel. Before this the browser listed every dot-file, had no filter at all, and
only reloaded after its own paste, delete, rename and create.

#### Courses of Action Considered

**Hidden files.** (A) Filter in `BrowserState` when a listing is stored. (B) Filter inside
`Vfs::list_dir`, which changes the trait for every backend. (C) Filter while drawing, which
leaves `selected`, `/` search and mouse hit-testing indexing a list the user does not see.
Chose A. The key is `.`, not `s`: `s` already opens the shell, and moving it would have broken
muscle memory for a feature that had a free key.

**Commands.** (A) Built-ins only, through `Vfs`. (B) Everything through `sh -c`. (C) Built-ins
first, anything else to `sh -c`. Chose C. `cd`, `mkdir [-p]`, `touch` and `q` are built in;
`rm` and `mv` are deliberately not, so nothing implies a confirmation the shell path does not
give. A built-in defers to the shell when given syntax it cannot honour (`*`, `|`, `&&`, `$`,
`~`, an unknown flag such as `mkdir -m 700`), because doing the wrong thing quietly there is worse
than running it.

**Refresh.** (A) Poll the directory's mtime. (B) `notify`/inotify. (C) Reload when a command
finishes and after mini-shell activity. I recommended A first, and it was wrong: a directory's
mtime changes when an entry is added, removed or renamed, not when a file inside it is written,
so editing `notes.txt` would never have refreshed its size or its preview. I said so when asked,
and the design changed to re-list the directory about twice a second on the blocking pool and
compare the whole listing (`DirEntryInfo` already derives `PartialEq` and carries name, size,
modified time and mode) with what is on screen. That needs no dependency, no mtime logic and no
second stat path, and it goes through `Vfs::list_dir`, so a future backend inherits it.
`notify` stays a roadmap item, for the case where polling is noticeable in a huge directory.

#### What Changed That Was Not Asked For, and Why

- `reload` now keeps the cursor on the same entry by path. With a background refresh a new file
  sorting above the cursor would otherwise move the selection every half second.
- The previews carry a generation counter. Re-reading a changed file starts a second read of the
  same path, and without it a slow first read could finish last and overwrite the newer content.
- The parent column always lists the directory being browsed, even a hidden one, or it would have
  nothing to highlight after toggling inside `~/.config`.
- `Vfs::touch` exists because coreutils `touch` on an existing file sets its modified time, which
  `create_file` (fails if present) cannot do.
- The refresher lives in `App`, not as another parameter to `run`, which was already at clippy's
  limit of seven.

#### Decision

Shipped as chosen. Hidden files default to hidden (`show_hidden = false`), which is a change in
behaviour from listing everything; `show_hidden = true` restores it. Shell commands reuse the
existing background-operation slot, so the UI is `BUSY` while one runs and `Esc` cancels it. The
header pill counts seconds for a command, since it has no item count.

#### Known Limits

- A refresh can lag a change by up to about half a second, and costs one `readdir` plus one `stat`
  per entry on each pass. Not measured in a directory of tens of thousands of entries.
- `Esc` kills the `sh` a command started, not a process it had already forked into the background.
- `sh -c` is not an interactive shell: no aliases or functions, and a `cd` inside the command does
  not move the browser. Output shows eight lines at most, for five seconds. A program that reads
  stdin sees end-of-file rather than waiting.
- If the browsed directory itself is deleted underneath the app, the list keeps what it had.
- Not verified: a real terminal emulator (only `pyte` on a PTY), and any filesystem other than
  the local one.

#### Verification

`cargo test --workspace` passes (`tui` 160 tests, up from 136) and clippy is clean. New tests cover
the hidden filter in every listing, path-preserving reload, `apply_listing` including a stale result
for a directory that was left, the command parser (with a property test that quoting any words and
splitting them returns the same words), the runner's cwd, stderr merging, output cap, cancellation
and a backgrounded child, the built-ins and shell path through `App`, and the refresher seeing a
create, an in-place write and a delete. Against the compiled binary, three scripted PTY sessions
through `pyte`, 28 checks in all: `.` toggling; `:mkdir -p`, `:touch` with a quoted name and
`:echo ... > file` producing files that are listed at once; the failing-command exit code; `:cd`;
`sleep 30` showing BUSY and `Esc` cancelling it within seconds; files created, rewritten and
deleted by another program appearing untouched, with the selected file's preview showing the new
text; a file made inside a mini-shell showing up in the list behind the box; and a new hidden file
staying hidden through refreshes until `.` is pressed.

The harness needed three fixes of its own, none of them app bugs. It answered `ESC[6n` but the
probe sends `ESC[5n` and `ESC[16t` too, so the screen stayed blank until those were answered. An
empty `$HOME` made zsh start its new-user wizard, which ate the first letter typed into the
mini-shell, so the session sets `SHELL=/bin/sh`. And a check assumed the cursor was on the first
row after `:mkdir`, when it had stayed on the same file, which is the path-preserving reload
working. My first version of the `mkdir` check also passed on the status line alone, because
`mkdir made/deep` puts "made" there; the retest uses names that never appear in a message.

### `Esc` Goes to the Mini-Shell: Remove the Typing-Mode Exit, Keep the `Alt` Ways Out

**Date:** 2026-09-20
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

`Esc` was the one key Minuteman took from a focused mini-shell: it dropped keyboard focus back to
the browser. A program inside the pane that uses `Esc` itself, such as `vim` or `fzf`, lost focus
the first time it was pressed. Since the `Alt` layer and the `Alt` tap already cover leaving and
closing, `Esc` no longer needed to be special.

#### Decision

Typing mode forwards every key, `Esc` included (`popup_shell::encode_key` already encoded it as
`0x1b`), so the only change is deleting the intercept in `main.rs`. Ways out that remain: tap
`Alt`, click the browser, `Alt+m` (closes the pane under the pointer, or the focused one) and
`Alt+q` (closes all). `Esc` is untouched where it means something to the browser itself: text
prompts, cancelling a running command, and the resize/move chord.

#### Consequences

- On a terminal without the kitty keyboard protocol the `Alt` tap does nothing, so the mouse or
  `Alt+m` is the only way out of typing mode there.
- The old README and `config.example.toml` text saying `Esc` closes a pane or stops typing was
  stale or now wrong, and was rewritten.

#### Verification

`cargo test --workspace` passes. Not run against a real terminal emulator.

### `Q` Under the Keyboard Protocol: Normalise Shifted Letters Once, at the Top of the Key Path

**Date:** 2026-09-20
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

`Q` still did not move the shell after the wrapper was installed and the binary was run as `mman`.
A scripted zsh session through the real wrapper worked for a plain `Q` and for the report
`CSI 113:81;2u` (key `q`, shifted key `Q`), but failed for `CSI 113;2u`, where the terminal names
only the unshifted key and the Shift flag. `crossterm` then yields `Char('q')` with `SHIFT`, and
`keys.resolve(key.code)` ignores modifiers, so it resolved to `Quit`.

#### Decision

One function, `with_shifted_letter_uppercased`, runs on every key press before any binding, `Alt`
command or shell input sees it. Resolving on `code` plus `modifiers` inside the keymap was
rejected: it touches every binding and the config format for one case. Which report the user's
kitty sends was not confirmed, so this fixes the reproduced failure, not a proven cause.

### Caps Lock Under the Keyboard Protocol: Flip the Case After the `Alt` Layer

**Date:** 2026-09-20
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

`Q` still did nothing after the Shift fix. Running `mman --cwd-file` by hand showed the file was
never created, so the key was not reaching `QuitToCwd`. A probe script run in the user's kitty
printed the raw reports: every one carried modifier mask 65 or 66, that is Caps Lock (64) with
or without Shift (1). With Caps Lock on, `q` arrives as `CSI 113;65u`: lowercase codepoint, no
Shift, and Caps Lock only as a flag that `crossterm` puts in `KeyEvent::state`. Nothing read it.

#### Decision

`with_caps_lock_applied` flips a letter's case when that flag is set, matching what a terminal
without the protocol already sends, so Caps Lock+Shift+`q` is lowercase as everywhere else. The
alternative, treating either Shift or Caps Lock as "capital", was rejected because it would type
`Q` for Caps Lock+Shift+`q` in a mini-shell. It runs after the `Alt` commands so `Alt+h` and the
others keep working with Caps Lock on.

#### Consequences

With Caps Lock on, `hjkl` stop navigating, exactly as in a terminal without the protocol and in
ranger and vim, because the letters now arrive as `HJKL`.

#### Verification

`cargo test -p tui` passes (162 tests). Through the real zsh wrapper on a PTY, with the exact byte
sequences from the probe: Caps Lock+`q` and Shift+`q` moved the shell, plain `q` and Caps
Lock+Shift+`q` did not. Not verified with the physical keyboard.

### `:nvim`, `c`, Arrow Keys and Nearest-First `/` Search: Four Requests, One Cycle

**Date:** 2026-09-20
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

Four requests arrived together: open other programs from the `:` prompt (`:nvim ROADMAP.md`), a
`c` key that cancels an operation such as a cut whatever directory the cursor is in, arrow keys
for browsing, and a `/` search that finds `aerend` from `~` by looking through the directories
below. Each was put as three courses of action first; the choices are below.

#### `:` commands that need the terminal

`:` commands ran under `sh -c` with stdin closed and output piped, so a full-screen program could
not work. Options: **A**, suspend the interface and give the program the real terminal, as
ranger's `:shell` does; **B**, run it in a mini-shell pane; **C**, run every command on a pty and
promote it when it enters the alternate screen. **A** was chosen: it is what `nvim` expects, and it
is the handover *Open-with* will need. B is cheaper but boxes the editor into 80% x 70% of the
screen; C changes every `:` command for a heuristic. Which commands count is decided by a
configurable list of program names (`interactive_commands`) plus a `!` prefix for anything else,
because a fixed list is wrong for someone's editor and a prefix alone makes the common case
tedious. The name is compared by file name, and only the first word counts, so `ls nvim` stays
captured.

The app cannot suspend the terminal because `main` owns it, so `App` records a `Handover` and the
loop takes it at the top of the next turn. Leaving mirrors `TerminalGuard::drop` and returning
mirrors its setup, including popping and re-pushing the keyboard protocol flags, in that order.
The repaint is `Terminal::resize`, not `clear`, for the reason recorded under *Post-Shell Redraw*
above. `run` reached eight parameters, so its read-only inputs became a `Session` struct rather
than an `allow` on the lint.

One thing had to be added that was not asked for. With raw mode off, the terminal turns `Ctrl-C`
into `SIGINT` for the whole foreground process group, Minuteman included, so interrupting
`:!ping host` would have killed the browser. `run_foreground` installs a handler that does
nothing for the duration. A handler and not `SIG_IGN`, because an ignored signal stays ignored
across `exec` and the child could then not be interrupted at all, whereas a handler resets to the
default. It costs one `libc` dependency, already in the tree through `crossterm`.

#### `c` cancels everything

Options: **A**, clear the clipboard only; **B**, clear the clipboard, the marks and cancel a
running operation; **C**, clipboard first, marks on a second press. **B** was chosen: the request
was "cancel this operation entirely". The clipboard is state on `App`, not on a directory, so
"whatever directory I am in" needed nothing extra, and a cut does nothing on disk until it is
pasted, so cancelling one only forgets it. A running delete has no cancel hook, and the status
line says that rather than claiming success. The busy state used to swallow every key but `Esc`;
it now also accepts `c`.

Making "cancel" mean everything turned up an old bug. Each item of a batch paste gets a fresh
cancel flag, so a cancel that landed as one item finished was forgotten and the next item started.
`poll_bulk` now reads the finished item's flag before continuing. The status hint for `cancel`
sits last in the normal-mode list: hints are dropped from the tail on a narrow bar, and putting
it earlier pushed `quit` off a 100-column bar, which an existing test caught.

#### Arrow keys

Options: **A**, add the key names to the config parser and the default bindings; **B**, hard-wire
them as always-on aliases; **C**, A plus `PageUp`/`PageDown`/`Home`/`End`. **A** was chosen to keep
bindings config-driven. The cost is that a config naming `move_down = ["j"]` replaces the list and
loses the arrow, which is how `enter = ["l", "enter"]` already behaved and is now stated in
`config.example.toml`. C was left for later, since paging needs the viewport height.

#### Nearest-first `/` search

Options: **A**, a breadth-first walk on the blocking pool; **B**, a background index scored with a
fuzzy matcher; **C**, shelling out to `fd`/`find`. **A** was chosen. Breadth-first order makes the
nearest match win and makes the current directory's own entries come first, which is what the
old search did, and a walk stops at the first hit instead of paying for the whole tree. It uses
only `Vfs::list_dir`, checks a cancel flag between directories and never `stat`s or opens a path
itself, so it will work over SSH when the backend exists; C would have bypassed `Vfs`, and B pays
for a full walk of `~` before the first key. The limits are 16 levels and 200,000 entries, with
"no match" saying when a limit stopped the walk. Symlinks to directories are listed as
directories, so the depth limit is also what ends a loop.

Each keystroke starts a new job and drops the old one; a job owns its channel and sets its cancel
flag on drop, so a slow search for `aer` can neither keep walking nor answer late. The directory
the search started in is checked first from the list already on screen, so the common case has no
delay at all. `Esc` had to change from restoring an index to restoring a directory and an index,
because a hit can be anywhere. `Enter` while a walk is still running stops it and stays put,
because jumping later, unasked, seemed worse than making the user press `/` again. Ranking beyond
depth and stepping through further matches are on the roadmap.

#### Verification

`cargo test --workspace` passes (`tui` 186, `browser` 30, `shell_overlay` 15, `theming` 38) and
`cargo clippy --workspace --all-targets` is clean. A property test on an in-memory `Vfs` that only
implements listing checks that the hit matches, that nothing nearer matches, and that "not found"
means nothing does. Its first run failed on a fixture that let a file have children, which no
disk can; the search was right. On a PTY through `pyte` against the real binary: arrows browse;
`c` cleared a cut and two marks from another directory and left the files alone, and killed a
running `sleep`; `/` found the nearer of two entries, a deeper one, and reported a missing name,
with `Esc` and `Enter` as described; the real `nvim` opened, took `Esc`, saved with `:wq`, and the
browser came back repainted and responsive; `:!cat > file` received typed input; `Ctrl-C` ended
`:!sleep` and left Minuteman running; and with the terminal answering the kitty protocol query,
the flags were popped before the alternate screen was left and pushed after it was re-entered.
Two harness findings: `pyte` prints a DCS string as text, which `nvim` sends at startup, so the
harness strips those as it already strips APC; and `nvim` keeps its file name on the second-to-last
row, not the last. Not verified in a real kitty or with a physical keyboard.

---

### Right-Click Menu, Inspect Panel and Open With: A Conventional File Manager's Mouse

**Date:** 2026-09-21
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

The mouse already selected rows, opened directories on a double-click and scrolled, but there was
no way to act on a file with it: no menu, no way to open a file at all (double-click on a file did
nothing), and nowhere to see more than the status bar's one line about an entry. The request was a
right-click menu with copy, paste, cut, delete, open with and the other essentials, plus an
inspect view with detailed information about a file or folder. Three courses of action were put
first.

#### Options Considered

**A**, a flat one-level menu in `tui` that reuses the existing actions, with Inspect filling the
preview column and Open with typed at a prompt. **B**, a menu with a submenu and a modal Inspect
panel, as new modules inside `tui`, with Open with driven by `[[open_with]]` in `config.toml`.
**C**, B built as separate crates, with Open with discovered from XDG `.desktop` files, and drag
and drop, multi-select clicks, breadcrumb clicks and a scrollable preview added. **B** was chosen.
A does not give what a desktop file manager gives, and its Inspect has no room; C mixes four
unrelated features into one change, one of which (parsing `.desktop` files) is a new surface of
its own. C's extras are queued on the roadmap as separate items.

#### The menu is a pure model

`context_menu` decides what a menu lists for what was clicked, where it sits on a given screen,
and how a pointer or a key moves through it, and never touches a terminal, an `App` or a
filesystem. `overlay_view` draws from `ContextMenu::layout` and the mouse handler hit-tests
against the same function, so a click cannot land on a different row than the one drawn, which is
the property `browser_mouse::BrowserLayout` was made for. What a chosen item does is a
`MenuCommand`, carried out by `run_menu_command` in `main.rs` by calling the `App` or
`BrowserState` method the item's key already calls; there is no second implementation of Delete
or Paste to keep in step. The menu is modal: while it is open it takes every mouse event and key,
so the click that dismisses it can never also select the row under it.

Right-clicking selects first, as a left click would, so the menu is always about something lit.
Right-clicking inside a marked set keeps the marks; anywhere else replaces them with the one
entry, as in any file manager, and Cut, Copy and Delete say how many entries they will touch
(`Delete (3 marked)`) because the marks, not the clicked row, are what they act on. Right-clicking
the left column does what a left click there does (go up, select), then opens the menu on that
entry. A right-click away from an open menu moves it; one on the menu does nothing. `Paste into
folder` needed `App::begin_paste_into`, with `begin_paste` now calling it with the browsed
directory.

#### Inspect

`Inspection` is one `lstat`, so a symlink is inspected as a link and a broken one says so. A
folder's contents (files, subfolders, bytes) need a walk that can take seconds, so `InspectView`
runs it on the blocking pool as `search_job` does, shows `counting…`, stops at 500,000 entries
("at least"), does not follow symlinks (a loop cannot hang it) and sets its cancel flag when the
panel closes. Owner, group, link count and on-disk size come from `std::fs`, not `Vfs`, which has
no such fields; the panel is therefore local-only for those rows, and that is on the roadmap
rather than hidden. Times are UTC, formatted by a small `civil_from_days` (Hinnant's algorithm)
because the standard library has no time zone database and a wrong local time seemed worse than a
labelled UTC one; checked against known epochs and leap days, with a property that a later time
never sorts before an earlier one.

#### Open and Open with

There was no "open a file" at all, so this added it. Open runs `xdg-open` (or `open`). Open with
lists `[[open_with]]` entries, falling back to `$VISUAL`/`$EDITOR` so the submenu is never empty
by default. A command is run under `sh -c` with the path shell-quoted (`{}` places it, otherwise
it is appended); the quoting is the one part that decides what a file name can make the shell do,
so it has a property test that pipes random names through a real `sh` and expects the same bytes
back. A program in `interactive_commands` reuses the terminal handover the `:` prompt built; any
other is started detached (`shell_overlay::spawn_detached`: no terminal, own process group, reaped
by a thread) because a viewer must outlive the browser and must not leave Minuteman `BUSY`. A
detached program cannot print "not found", so the command's first word is looked up on `PATH`
first and the status line says so. Double-clicking a file now calls Open; the file to open is read
before the click is applied, because applying a double-click on a folder moves the cursor into it.
Copy path uses OSC 52 rather than a clipboard tool, so it works over `ssh` and inside `tmux`, and
says "sent", not "copied", since not every terminal honours it.

#### Trade-offs and what was left

Open with is a hand-written list, not discovery; an "Other..." prompt, MIME rules, local time,
marked-set totals in Inspect, multi-select clicks, breadcrumb clicks and drag and drop are on the
roadmap. While an operation runs, the menu still opens, but Rename, Delete and New refuse, since
the keys are all ignored then and those would start a prompt over it.

#### Verification

`cargo test --workspace` passes (`tui` 236, `browser` 30, `shell_overlay` 16, `theming` 39) and
`cargo clippy --workspace --all-targets -- -D warnings` is clean. Property tests: any file name
survives the shell quoting unchanged; a menu and its submenu stay on screen whatever the pointer
and screen size; every drawn row hit-tests to its own entry; and no sequence of moves, clicks and
keys leaves a missing or disabled row highlighted. On a PTY against the real binary with SGR
mouse sequences: the file, folder and empty-space menus opened; hovering "Open with" opened the
submenu; a click on its item ran a program on the file; Inspect showed a file and a folder
(`Directory`, `0 files, 0 folders, 0B`); Delete reached its `(y/N)` prompt and `n` left the file;
Cut then Paste into folder moved a file on disk; a double-click on a folder entered it without
opening anything, and on a file ran `xdg-open`. Two harness findings: the first mouse event after
start-up was lost every time (a throwaway motion event before the real ones avoids it; the cause
was not tracked down, and a left click was not tried as the first event to see whether it is
older than this change), and the harness reads text
out of a diff-drawn screen, so it can only ask whether a label appeared, not where. Not verified
in a real desktop session beyond `xdg-open` being invoked, and not with a physical mouse.

### Richer Status Line: Marked-Set Size and Git Status, Shelling Out to `git` Off the Render Thread

**Date:** 2026-09-21
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

The HUD's status line showed permissions, size, type, item count and position for the selection,
and the roadmap listed two gaps: what the marked entries add up to, and, where it applies, git
status. Both need work that can take seconds (walking a marked folder, running `git` in a large
repository), so the question was where that work runs and what it is allowed to cost. Three
courses of action were put first.

#### Options Considered

**A**, both pieces, with git read by running `git status --porcelain=v2 --branch -z` on the
blocking pool and marked sizes summed with the walk Inspect already has. **B**, both pieces, with
git read in-process through the `gix` crate. **C**, marked size only, with git split into a later
feature. **A** was chosen. B removes the dependency on a `git` binary but brings a large
dependency tree and slower builds for a decoration, and ties the code to local paths just as a
remote `Vfs` is queued. C leaves half of the roadmap item open. A adds no dependency, and both of
its risks are contained: a missing `git` means no segment, and a slow one is cut off by a timeout.

#### Where the two figures go

The marked total went into the header's existing `◆ N marked` pill (`◆ 3 marked │ 1.4 GiB`),
because that is where the count already is and the header has room; the divider is the glyph set's
own, so the ASCII set stays ASCII (the first version used a middle dot, and the existing
ASCII-only test failed on it). The git state went into the status bar: the repository on the right
beside the position, the selected entry's state on the left after its type. Both are added only if
they fit beside the segments that are always shown, so a narrow bar loses git before it loses a
file detail, and neither appears in the leader, resize, busy or prompt modes, which have their own
status.

#### Marked size

`marked_size::total_of` sums a set of paths: a file or link by its `lstat` length, a folder through
`inspect::tally_dir` (so it does not follow symlinks and shares the 500,000-entry limit, spent
across all the marked folders together). A marked folder already contains whatever is marked
inside it, so paths under one are skipped; sorting makes everything below a folder contiguous
after it, which is why remembering the latest folder is enough, and `dir.txt` sorting right after
`dir` while not being inside it is covered by a test. The tracker (`MarkedSize`) starts a fresh
walk whenever the marks change and cancels the one before it. It keeps showing the previous total
until the new one lands, because clearing it made the pill flicker on every mark. The sum is taken
when the marks change and is not refreshed: re-walking a marked folder twice a second would cost
far more than the figure is worth, and the roadmap says so.

#### Git status

`git_status` has a pure parser over the porcelain-v2 bytes (branch and upstream headers, ordinary,
renamed, unmerged and untracked records; anything unrecognised is skipped so a newer git cannot
break it), a `Repo` that maps every changed path, and every folder above one, to a state, and a
`GitStatus` that runs one job at a time. Four decisions are worth recording.

- `--no-optional-locks`. A plain `git status` refreshes a stale index, which takes the index lock
  and rewrites it; a refresh landing while the user runs `git commit` in a mini-shell would make
  their commit fail with "index.lock exists". A test makes the index stale on purpose, runs the
  status job, and checks the index's modification time is unchanged, then runs a plain
  `git status` as a control and checks that one does change it, so the assertion cannot pass
  vacuously.
- Repository detection is in-process. Looking for `.git` up the ancestors costs a few `stat`s, so
  browsing outside a repository never starts a process, and a `git init` made later is noticed on
  the next refresh with no special case.
- The state of a folder is the merge of what is inside it (a conflict outranks everything, an
  untracked file only shows when nothing tracked changed, different tracked changes read as
  staged-and-modified). Merge being commutative, associative and idempotent is what lets the
  ancestors be folded in any order; the five states are few enough that the test checks every
  combination rather than sampling.
- Timing. The next run starts three seconds after the last one finished, not three seconds after
  it started, so a slow repository gets a proportionally lazier refresh. A run is cancelled when
  the browsed directory leaves the repository (the segment goes at once, not after the next run)
  and killed after ten seconds. Output is read on its own thread, since a status listing larger
  than the pipe buffer would otherwise block git while the job waits for it to exit, and is capped
  at 32 MiB.

The `git_status` option in `config.toml` (default on) is the off switch, since this starts a
process in the background.

#### Trade-offs and what was left

The git segment can lag a commit by up to three seconds; refreshing when the live refresh sees a
change or a `:` command ends is on the roadmap. A repository whose `git status` takes more than ten
seconds never shows a segment. The per-file state shows for the selection only, not as a column.
There is no benchmark: neither piece is on a hot path (one job every few seconds, off the render
thread), and the workspace has no benchmark harness, so adding one would have meant a new
dependency for nothing to guard. There is no logging either, as the project has none and writing
to stderr would draw over the interface. The project also has no `scripts/check` yet, so
`cargo fmt --check`, `cargo clippy -- -D warnings` and `cargo test` were run by hand; `cargo fmt
--check` already reports differences in `app.rs`, `main.rs` and `shell_layout.rs` at the last
commit, which were left alone as unrelated.

#### Verification

`cargo test --workspace` passes (`tui` 268 including the new ones) and `cargo clippy --workspace
--all-targets -- -D warnings` is clean. Property tests: ordinary porcelain records round-trip with
paths containing spaces and counts that add up; arbitrary bytes never panic the parser; marked
totals equal the sum of the file sizes in any order, nesting or with repeats. Two tests found
mistakes of mine before the binary was run: the narrow-bar test assumed the file-state segment
would be dropped at 60 columns when it still fits (the assertion was wrong, the behaviour is
intended), and the ASCII-only test caught the middle dot. On a PTY against the real binary, read
through `pyte`, in a scratch repository with one staged, two modified (one inside a folder) and
one untracked file: the bar showed `⎇ main` and `+1 ~2 ?1`; each file, and the folder, showed its
own state; a clean file showed the branch and no state word; marking one file showed `1 marked │
2.0K` and a second `2.2K`; a marked folder showed `500B`; `c` removed the pill; `:cd` out of the
repository removed the segment at once and `:cd` back restored it; and with `git_status = false`
neither the segment nor a `git status` process appeared. Not tried in a real terminal emulator,
against a very large repository, or with a worktree or submodule (the `.git` file case is handled
by looking for any `.git` entry but was not exercised).

### Preview Extras, Stage 1: A Scrolling Preview, a Hex View and Archive Listings, Every Read Bounded

**Date:** 2026-09-21
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

The preview column showed a text file's start and nothing else: it could not scroll, and a binary
or an archive showed only its name. The roadmap's first stage of preview extras was a scrollable
preview, a hex view for binaries and a listing for `zip`, `tar` and `tar.gz`. Scrolling and the
hex view have one sensible design each; archive listing is where the choice was, so three courses
of action were put first for it. The two smaller questions raised with them (which keys scroll, and
whether text-named binary should fall back to hex) were answered by taking the proposed defaults
when the reply was "A. proceed".

#### Options Considered

**A**, list archives in-process with the `zip` and `tar` crates (and `flate2` for gzip). **B**,
run `bsdtar -tvf` off-thread. **C**, parse the tar headers and the zip central directory by hand.
**A** was chosen. B covers almost every format but shows nothing on a machine without `bsdtar`,
its output differs between versions, and it is a process per selected archive that is hard to test
deterministically. C has the fewest dependencies and the most code, all of it parsing untrusted
input I would then maintain (zip64, tar long names, pax headers). A adds two crates but keeps the
feature in-process, testable with generated archives and bounded by construction. B stays
available later for the formats A skips, as the external-previewer hook stage 3 already plans.
`zip` is built with no default features: listing reads only the directory, so no compression codec
is needed, which keeps the dependency small.

#### Scrolling

The position is a `Scroll { offset, viewport }`. A key or a wheel notch adds to the offset with no
upper limit, and the drawing code calls `fit`, which records the pane's height and clamps the
offset, because neither that height nor the content's length is known until it is drawn. Doing
the clamp at draw time rather than in the key handler is what keeps the handler free of layout
knowledge, and the clamped value is written back so the position never drifts past the end. A key
moves half the viewport (at least a row), the wheel three rows like the file columns. The offset
resets when the selected path changes and is kept when the same file is re-read after changing on
disk, so a log being appended to does not jump to the top. `J` and `K` are the defaults
(`preview_down` and `preview_up`); they were unbound. For text, the wrapped height comes from
`Paragraph::line_count`, which ratatui offers only behind `unstable-rendered-line-info`, and is
cached per width and per content because measuring walks the whole text. The scrollbar reuses the
list scrollbar, with the offset stretched over `0..total - 1` (`scrollbar_position`) since a
scrolled view stops at `total - viewport` and the thumb would otherwise never reach the bottom.

#### Hex view

Only the first 64 KiB of a file is read, so a huge binary costs what a small one does. Rows are
formatted from those bytes when drawn, for the visible rows only, because how many bytes fit in a
row (16, 8 or 4) depends on the pane's width, which changes on resize. `preview::load` decides what
a file is: an archive by name, then text if its name says so or, new here, if its first 64 KiB is
valid UTF-8 with no null byte, and otherwise bytes. Sniffing was needed because a hex view for
"everything else" would otherwise have shown `notes` or a `.service` file as hex. A text-named
file holding binary bytes now shows hex rather than `preview failed`; a text-named file over 1 MiB
still fails, as before. A file that is not a regular file is checked with `metadata` before it is
opened, because opening a named pipe for reading blocks until something writes to it; a test with
a real `mkfifo` guards that, and so does the PTY run.

#### Archives are untrusted input

Listing runs merely because the cursor passes over a file, so the limits matter more than the
parsing. At most 5,000 entries are kept. A `.tar.gz` is inflated for at most 256 MiB, through a
`Capped` reader that fails when the limit is hit and more data is coming, not `Read::take`, which
ends quietly: the tar reader takes a quiet end for the archive's real end, so a cut-off listing
would have passed for a whole one. A plain `.tar` is skipped through with `entries_with_seek`, so
listing costs its headers, tested on a 200 MiB sparse file. For a zip, the `zip` crate reads the
whole central directory into memory when it opens the file, so a hostile one could ask for a great
deal; the last 64 KiB is read first and the file is refused if its end-of-directory record claims
more than 8 MiB or uses zip64 (all-ones fields), and then it is shown as bytes. My first version of
that check looked only at the record nearest the end of the file; a test I wrote to put a small
fake there and a greedy real one before it failed, because a reader is free to skip a record that
turns out to be bogus and use an earlier one. It now checks every place that looks like a record,
and one claiming too much refuses the file. Names come from the archive's author, so they are
cleaned before drawing: control characters (an escape sequence in a name could retitle or repaint
the terminal) and the invisible characters that reorder text (a right-to-left override can make
`evil` + U+202E + `txt.exe` read as `evilexe.txt`) become `?`, and a name is cut at 1,024
characters. The PTY run put an OSC title sequence in a name and checked that none reached the
terminal.

#### Trade-offs and what was left

`xz`, `zstd`, `bzip2` and `7z` archives, and zip64 ones, show as bytes. A text over 1 MiB says
`preview failed` instead of showing its start. The scroll keys are not in the status bar's hints,
which are already crowded. A text scrolled to the end of a 1 MiB file makes ratatui wrap
everything above the visible rows each frame; an ignored benchmark test measures about 43 ms in a
release build, inside the 100 ms tick, so caching wrapped lines was not done. `TextPreview` keeps
its name although it now serves every non-image file; renaming it would have touched code with no
behaviour to gain, so its module doc says what it covers instead. There is no logging, as the
project has none, and `scripts/check` still does not exist, so the format check, clippy and the
tests were run by hand; `cargo fmt --check` still reports the differences in `app.rs`, `main.rs`
and `shell_layout.rs` that predate this work.

#### Verification

`cargo test --workspace` passes (`preview` 34, `tui` 287, `theming` 40, `browser` 30) and
`cargo clippy --workspace --all-targets -- -D warnings` is clean. Property tests: every byte of a
buffer appears once, in order, across the hex rows at any width; zip, tar and tar.gz of random
names and sizes list back exactly; the offset that gets drawn is always within the content; the
scrollbar thumb spans the bar and only moves down; a cleaned name never holds an unsafe character;
the zip guard follows its record's own fields; no pane size or scroll position panics for any kind
of content. Three of my own tests were wrong and were fixed rather than the code: the hex test
expected the last row to be as long as the others, the tar test expected a truncated data section
to be detectable (a seek past the end of a file is not an error), and one expected a text-looking
`bad.zip` to show bytes. On a PTY against the real binary, read through `pyte`, 21 checks passed:
`J`, `K` and the wheel scrolled a 200-line text and stopped with the last line as the last row,
without moving the file selection; a binary showed `00000000  00 01 02 03 04 05 06 07` at 8 bytes a
row in that pane width; a 200 KiB file said `first 64K of 200K shown` after scrolling to the end;
a zip and a tar.gz showed their summary and entries; an archive whose name held an OSC title
sequence showed `evil?]0;pwned?name.txt` and sent nothing to the terminal; a `.txt` of binary
showed hex; an extensionless text file showed text; and a named pipe showed only its name with
the browser still responsive. The harness lost its first keystroke after start-up, as the earlier
entries note, which cost one wasted run before a throwaway key was added. Not tried in a real
terminal emulator, with a physical wheel, or against an archive from an untrusted source beyond
the generated ones.

### Disk Usage View: A Modal `du`-Style Screen, One Folder Scanned at a Time

**Date:** 2026-09-21
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

Minuteman could tell you the size of a folder (Inspect) but not where the space in it went, and
`inspect::tally_dir` returns only a grand total: nothing about which child holds the bytes. The
request was a disk usage view. It is a new screen rather than a tweak, so three courses of action
were put first, and the four smaller questions raised with them were answered by taking the
proposed defaults ("A. defaults").

#### Options Considered

**A**, a full-screen modal view listing the current folder's children biggest first with a size,
share and bar, where `enter` opens a subfolder and `h` goes back. **B**, no new screen, a toggle
that fills the browser's size column with recursive folder sizes and sorts by them. **C**, A with
the scan cached as a tree so drilling down is instant, and marking and deleting from inside it
through the existing `d` flow. **A** was chosen. B is small but shows one level, has no bars and
does not answer "what is eating my disk". C is the most useful and close to two features: a cache
of a whole tree can use a lot of memory, and delete-from-view has to stay consistent with it. A
delivers the core, seeing where the space went and looking inside, and C's extras can build on it
later. The defaults taken: `u` opens it, the size is what is allocated on disk with the apparent
size a key away, hard links count once and the scan stays on its filesystem without following
symlinks, and there is no delete in this cycle.

#### What a size means

`std::fs` gives both numbers from one `lstat`: `blocks() * 512` is the space on disk (the unit is
512 bytes whatever the filesystem's block size) and `len()` is the apparent size. Both are kept in
a `Sizes` for every row, so `a` re-sorts without scanning again; a sparse file is the case that
shows why both matter (10,000,000 bytes long and about nothing on disk, checked in the PTY run).
A file with more than one link is remembered by `(device, inode)` and counts once, at the first
place the scan meets it, so `hl1` and `hl2` show one size and one `0B`. The scan compares each
folder's device with the starting folder's and lists another filesystem's mount point as such
without entering it, which is what keeps a scan of `/` out of `/proc` and network mounts, and it
reads entries with `DirEntry::metadata`, which does not follow a symlink, so a link is the size of
the link and a loop cannot hang it. A folder's own node (a few KiB) counts toward it, as in `du`.
A subfolder that cannot be read marks its parent `!` rather than silently undercounting.

#### One level at a time

Only the folder on screen is scanned. Opening a subfolder scans that one; the levels on the way
down stay on a stack, so `h` restores the level above from memory. That gives back instantly, and
memory holds the rows of a few folders, not a tree. The price is that opening a folder rescans it
instead of reading a cache, and hard links are deduplicated per scan, not across levels; both are
the trade C would not make and are listed as follow-ups. Going above the folder the view was
opened on replaces the bottom level with the parent and puts the cursor on the folder just left,
by remembering its path until that row arrives (`reselect`). The scan sends rows over a channel as
it goes: every file and link at once, then each subfolder when its total is known, so a large
first subfolder does not hide the small files. Dropping the view sets the job's cancel flag, and a
`seen` counter shared with the scan feeds the `scanning 1,204` in the totals line.

Two bounds keep it safe. A folder with a million files must not become a million rows, so at most
20,000 of a folder's biggest files get a row each (the list is sorted and folded when it doubles,
so the cost per file stays constant) and the rest become one `N smaller entries` row, with a test
that no byte is lost or invented in the folding and that every entry of the largest size is kept.
And a scan looks at no more than 10,000,000 entries, after which its sizes are labelled lower
bounds.

#### The cursor

The first version made the cursor follow the row it was on while rows streamed in and were
sorted, which is right once you have moved it and wrong before: the first small file to arrive
kept the cursor while bigger rows sorted in above it, so `enter` opened nothing and the first row
was not selected. Two tests caught it (one expected `enter` to open the biggest folder, one
expected the cursor at the top). It now stays on the first row, the biggest so far, until the user
moves it, with a `moved` flag on each level, and follows the chosen row from then on. A property
test keeps that promise across random arrivals; another checks the selected row is always on
screen and the view scrolls only when it must.

#### The screen

`disk_usage_view` holds the geometry as functions (`layout`, `row_at`, `scroll_top`,
`share_percent`, `gauge_cells`, `elide_left`), and draws only the rows on screen. The bar reuses
the glyph set's gauge, so the ASCII set stays ASCII (a test checks a whole screen), and the
scrollbar is drawn for the rows' region. What keys move by, and which row a click hits, depend on
things known only when drawing (the height, the first row shown), so the drawing code records them
in `Cell`s on the view; drawing takes `&self`, like the other overlays. The view is modal, like
Inspect: it takes every key and mouse event, `q`, `Q`, `Esc` and `u` close the view and not the
program, movement, `enter` and `h` follow the configured keys, and `a`, `r`, `PageUp`,
`PageDown`, `Home` and `End` are fixed, as the leader chord's keys already are.
`run_menu_command` had reached seven arguments and clippy's limit, so the two panels it can open
are passed together as a `Panels` value.

#### Trade-offs and what was left

No delete or mark from the view, no cached tree, no sort other than by size, and the key hints
in the footer are fixed text, so they are wrong if `enter` or `h` are rebound. The view reads
`std::fs` directly, so it is local-only, the same as Inspect's owner and on-disk rows; a `Vfs`
method for allocated size is what a remote backend would need. All of this is on the roadmap.
The first run of the throughput benchmark took 20 s over `/usr` and looked slow; a warm run took
2.85 s against `du`'s 2.9 to 3.0 s, so it was the disk, not the scan. As before there is no
logging, and `scripts/check` does not exist, so the format check, clippy and the tests were run
by hand; `cargo fmt --check` still reports the differences in `app.rs`, `main.rs` and
`shell_layout.rs` that predate this work, and the two import-order ones this work added to
`main.rs` were fixed.

#### Verification

`cargo test --workspace` passes (`tui` 321 including 34 new, `preview` 34, `theming` 40, `browser`
30) and `cargo clippy --workspace --all-targets -- -D warnings` is clean. Two of my own tests were
wrong and were fixed, not the code (the view tests shared a scratch directory and raced when run in
parallel, and one expected `10M` where 10,000,000 bytes is `9.5M` in binary units), and clippy
found three things in my code (an eight-argument function, a manual multiple-of check, an
assertion against a literal bool). On a PTY against the real binary, read through `pyte`, 21
checks passed: `u` opened the view and the scan finished; the totals line named the entries and
the measure; the biggest folder was first with a share and a bar; sizes ran biggest to smallest; a
hard-linked pair showed one size and one `0B`; a sparse 10 MB file took almost nothing on disk; a
symlink to the folder itself showed as a link and was not followed; `a` switched to the apparent
size and put the sparse file on top at `9.5M`, and `a` again went back; `enter` opened a folder
and listed its contents biggest first with the nested folder's total; `h` returned with the cursor
on the folder just left; `enter` on a file did nothing; a click selected the row under the pointer;
`r` rescanned; `q` closed the view and left the browser intact; a folder's right-click menu offered
Disk usage and choosing it opened the view; and `Esc` closed it. The harness lost its first
keystroke after start-up, as the earlier entries note, and a throwaway key was sent first. Not
tried in a real terminal emulator, on a network filesystem, or with a real mount point inside the
scanned folder (the other-filesystem rule was tested by pretending the root was on another
device).

---

### Configurable Panels and a Live Settings Popup: `[panels]` Config Plus `Space T`, Session-Only for Now (COA C)

**Date:** 2026-09-22
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

The request was to make Minuteman's appearance customizable "through code files, and an
interactive pop-up menu" summoned by a key chord, letting the number of panels, the theme and
other pieces be chosen — including, as an example, removing the command bar and replacing it with
something else. That last part is a plugin-shaped ask, and `ROADMAP.md` already commits to a
future WASM/Extism plugin host as the project's long-term answer to "add or remove any feature",
so the three courses of action were framed against that existing plan rather than a blank slate.

#### Options Considered

**A**, declarative-only: extend `appearance.toml`/`config.toml` with panel-layout fields and give
the popup nothing to do but edit and write that TOML back out. Fast and requires no new
architecture, but "remove or replace a feature" only ever means toggling among variants already
compiled in — it caps out below what was asked. **B**, pull the roadmap's planned WASM/Extism
plugin host forward into this cycle: a versioned host API with panel/layout/widget hooks, so the
popup drives a live plugin registry that can genuinely swap in a third-party, multi-language
command bar. This is the project's actual long-term destination, but it is a roadmap-scale project
of its own — the host API, sandboxing and versioning would all have to be settled in one cycle
rather than incrementally. **C**, a Rust-native `Panel`-shaped config compiled in: wrap the
already-fairly-modular built-in panels (the miller columns, the header, the status bar) behind a
config-driven registry (`[panels]` in `config.toml`), with the popup editing that registry live.
Not runtime-pluggable and not multi-language — a new panel still means new Rust code and a
recompile — but it is buildable now without contradicting or duplicating B, and its shape doubles
as a draft of what B's host API will eventually need to expose. **C** was chosen: B is the right
destination but too large to improvise inside one cycle, and A caps out below the ask.

#### The panels config and the column layout

A new `theming::panels` module adds `ColumnLayout` (`ThreePane`, the existing parent | current |
preview 20/40/40 split, or `TwoPane`, current | preview at 50/50 with the parent column removed)
and `PanelsConfig` (`columns`, `show_hud`, `show_command_bar`), read from a `[panels]` table the
same way every other `config.toml` table is: a `RawPanels` of `Option`s under `#[serde(default)]`,
resolved field-by-field so a partial table or a typo'd `columns` value falls back rather than
failing to start, exactly like `RawUi`'s `glyphs`. `BrowserLayout::split` (the one function both
`draw` and the mouse hit-test build their rectangles from, so the two can never disagree about
where a click lands) now takes the column layout and the HUD flag. Two-pane gives the parent
column a zero-width `Rect` at the same position rather than a `Percentage(20)` slice — `draw`
skips rendering it and `Rect::contains` can never resolve a click into it, so the column reads as
removed, not just left blank. Hiding the HUD reclaims its row's height (`Constraint::Length(0)`
instead of `1`) the same way. The status row is deliberately never reclaimed this way: a rename,
delete-confirm or `:` prompt renders there, so `show_command_bar` only suppresses the row's *idle*
chrome (the mode pill, the selected file's details, the key hints) — the row stays reserved, and a
prompt, a non-`Normal` mode or a transient message always forces it back, checked with one
`show_status_bar` condition at the single production call site in `draw`. This was a hard
constraint going into the cycle, not a style choice: silently dropping the confirmation UI for
delete or rename would be a real correctness bug, not a cosmetic one.

#### The settings popup: freeing up `space t`

The request said `Alt+T`, but `Alt+t` was already `SnapTop` in the shell-box `Alt` layer, so the
trigger became `space` then `t` instead — chosen deliberately over freeing up `Alt+t` because the
leader layer already had an unused slot for exactly this shape of command. `pending_leader`'s
handling was `if shells.is_some() { ... } ...` with no `else`: when leader is pressed with no
shell pane open, the block already does nothing at all with the next key — `t`, in particular,
only means "flip the split's orientation" when a shell pane exists to flip. That confirmed-empty
`else` arm is where the popup now opens, so nothing that worked before changes meaning. Like
`context_menu`, `tui::settings_popup` is pure state — a cursor over four rows (Columns, Theme,
HUD, Command bar) and a `key(KeyCode) -> Outcome` that never touches a `Frame` or a `Config` — so
`overlay_view::render_settings` is the only place it's drawn and `main::run` is the only place its
`Outcome::Cycle` is applied. Applying it means mutating session-local `live_panels`/`live_theme`
state and building a per-frame `effective_config` (the real `Config`, cloned, with `panels` and
`theme` overridden) inside `draw`, rather than threading two more parameters through every
`hud`/`overlay_view`/`style` function that already takes a `Config` — cheap next to the
`Vec<ListItem>`s `draw` already rebuilds every frame, and it means every existing renderer picks
up a live change with no signature changes elsewhere. `Theme::named` (the palette lookup by name,
already total and fallback-safe) went from private to `pub` so the popup's Theme row could reach
it directly. The popup does not seed its theme cycle from whatever `appearance.toml` currently
has — the resolved `Theme` type carries colors, not the name it came from — so cycling always
starts at `"neon"` regardless of the configured palette; the on-screen status message ("... —
this session only, not saved") is there partly to make that, and the lack of persistence, visible
rather than surprising.

#### Trade-offs and what was left

Nothing here is written back to `config.toml`/`appearance.toml`: there was no precedent anywhere
in this codebase for writing TOML back out, and doing it without destroying the user's comments —
both files are heavily, deliberately commented — is a real feature of its own, not a quick
addition. It's on the roadmap (Medium Priority) rather than built now. The popup toggles and
resizes what already exists; it does not let a panel be *replaced* with a different
implementation (a second command-bar variant, for instance) — that is explicitly B's job, not
C's, and building a throwaway "alternate command bar" just to prove the point would have been
scope creep against the actual ask. `scripts/check` did not exist before this cycle — the Verify
rule requires proposing one rather than inventing it, so `cargo fmt --all -- --check`, `cargo
clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` were proposed and
approved, then written to `scripts/check`. Running it immediately failed on `cargo fmt --all --
--check`: `app.rs` (8 spots), `shell_layout.rs` (2) and `main.rs` (7) already disagreed with
`rustfmt` on clean `main`, confirmed by stashing this cycle's diff and checking. This cycle's own
edits added 9 more (`config.rs`, `hud.rs`, `overlay_view.rs`, and 2 more in `main.rs`). Fixing the
new ones by running `rustfmt` directly on the touched files had a side effect worth recording:
passed a crate root (`main.rs`), `rustfmt` reformats the whole module tree it declares, not just
that file, so `app.rs` and `shell_layout.rs` — neither touched by this feature — were swept in and
silently fixed too. Asked directly, since that's a repo-wide change outside this feature's
surgical scope: keep it, as a separate leading `chore: rustfmt normalization` commit ahead of the
feature commit, so `scripts/check` is fully green going forward instead of red on unrelated,
pre-existing drift forever.

#### Verification

`cargo clippy --workspace --all-targets -- -D warnings` is clean and `cargo fmt --all -- --check`
now passes across the whole workspace. `cargo test --workspace` passes: `tui` 328 (2 pre-existing
ignored timing tests, unrelated to this cycle) and `theming` 48, including new property tests
(`proptest`, already a dev-dependency in `tui`/`browser`/`preview`, added to `theming` too) for
`[panels]` parsing never panicking and always resolving absent fields to their documented default
regardless of what subset of keys is present, the settings popup's cursor always staying in
bounds under an arbitrary sequence of moves, and `BrowserLayout::split`'s regions never
overlapping in either column layout with the HUD shown or hidden. A dedicated test pins
`ThreePane` plus `show_hud = true` to the exact rectangles the old hardcoded 20/40/40 split
produced, so this cycle could not silently change the default layout. Not verified in a real
terminal or PTY session — the popup's mouse dismissal path and its on-screen rendering were
checked by reading the code and the unit tests, not by running the compiled binary interactively.

---

### Adaptive Default Theme: an OSC 11 Background Query, Reusing `ratatui-image`'s Own Probe (COA A)

**Date:** 2026-09-22
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

The request was to make the default theme "adapt to the vibe of the overall user's desktop"
instead of always being the neon-cyberpunk palette, keep neon available as an explicit pick, and
add Catppuccin and Nord as two more built-in palettes.

#### Options Considered

**A**, an OSC 11 escape-sequence query: ask the terminal emulator itself for its background color
at startup, classify it dark/light, and pick a palette to match. Terminal-agnostic and reflects
whatever the user actually themed their terminal to — which, for a program that only ever renders
inside that terminal, is the more relevant "vibe" than the desktop's own setting. **B**, read the
OS/desktop environment's light/dark setting directly (`org.freedesktop.appearance` over D-Bus on
Linux, `AppleInterfaceStyle` on macOS, the registry on Windows): reflects the literal desktop
setting, but needs a new dependency and a different code path per platform, gives up entirely over
SSH to a headless host, and can disagree with a terminal deliberately themed differently from the
desktop (a common thing to do). **C**, a `$COLORFGBG`-only heuristic: no new dependency, but many
modern terminals (kitty, alacritty, wezterm) don't set that variable at all, so it degrades to a
guess far more often than A.

Suggested and chosen: **A**. The decisive fact, found while surveying `crates/tui/src/main.rs`'s
existing startup probes, is that `ratatui-image` (already a dependency, used for
`Picker::from_query_stdio()`'s graphics-protocol detection) already implements exactly this OSC 11
round trip internally, gated behind `QueryStdioOptions.terminal_background_color_osc` and surfaced
as `Capability::Background(r, g, b)` in `Picker::capabilities()`. No hand-rolled raw-stdio reading
(spawn a thread, race it against a timeout, parse a partial escape sequence) was needed — the
existing, already-tested probe in `ImagePreview::new` just had one option flipped on, and its
result read back out through a new `detected_background()` getter alongside the picker itself.

#### Deciding what "nothing configured" means

The one design question with real blast radius was: since `appearance.example.toml` has always
shipped with `name = "neon"` spelled out, how does auto-detection avoid silently overriding every
user who ever ran `minuteman init-appearance` and kept the example verbatim? Comparing the
*resolved* `Theme` against `Theme::default()` was rejected — `name = "neon"` resolves to the exact
same `Theme` value as an absent `name` (both fall through `Theme::named`'s catch-all), so that
comparison can't tell an explicit pin from silence. The fix was to compare the *merged, unresolved*
`RawTheme` (config.toml's `[theme]` overlaid with appearance.toml's) against `RawTheme::default()`
instead, exposed as a new `Config::theme_is_customized: bool`. `RawTheme` gained `PartialEq, Eq`
for this. The shipped example sets every field, so it is unambiguously "customized" and keeps
`neon` exactly as before; only a config with no `[theme]` table in either file at all — the
genuinely fresh-install case — resolves through `Theme::auto`. A user who overrides just one field
(no `name`) also counts as customized, deliberately: mixing an auto-detected base with field-level
overrides would need theme resolution deferred until after the terminal probe runs (today it
happens synchronously inside `Config::from_sources`, with no I/O), which was out of scope for what
was asked.

`Theme::auto(is_dark: Option<bool>)` is a pure function in the `theming` crate (dark → Catppuccin
Mocha, light → Catppuccin Latte, `None` — the terminal never answered — → the original neon
default); `Theme::is_dark(r, g, b)` classifies by ITU-R BT.601 luminance. Neither crate does
terminal I/O — that stays in `tui`, matching the existing split (`style.rs`'s truecolor detection,
`osc52.rs`'s clipboard escape codes) — so `main` is the only place that calls `Theme::auto` with a
live probe result.

#### Trade-offs and what was left

Catppuccin Latte (the light flavor) is reachable by name in `appearance.toml` but deliberately left
out of the settings popup's `THEME_NAMES` cycle, which sticks to dark-background palettes like
`neon`/`dracula`/`nord` already did — cycling into a light palette mid-session, on a terminal the
popup has no way to re-probe, seemed more likely to surprise than help. Auto-detection only ever
picks between two Catppuccin flavors, not a wider blend of the detected color into every field —
a literal "generate a palette from these exact RGB values" approach was considered and rejected as
open-ended color-design work with no clear stopping point, for a first cut of "adapts to the
desktop" this size.

#### Verification

`scripts/check` passes: `cargo fmt --all -- --check` clean, `cargo clippy --workspace
--all-targets -- -D warnings` clean, `cargo test --workspace` green (`tui` 328, `theming` 52,
including new tests for `catppuccin`/`catppuccin-latte`/`nord` being reachable by name,
`Theme::auto`'s three branches, `Theme::is_dark`'s luminance classification, and
`Config::theme_is_customized` being `false` only when `[theme]` is absent from both files — a
single overridden field, a bare `name`, or the shipped `appearance.example.toml` all correctly
count as customized). Not verified against a real terminal's actual OSC 11 reply — this sandbox has
no interactive TTY to test that round trip against a live terminal emulator, so the query path
itself rests on `ratatui-image`'s own test coverage and the same timeout/fallback shape its
existing graphics-probe call already relied on; only the pure classification/resolution logic
downstream of a detected color was exercised directly.

---

### Appearance Popup: Curated Colors, Real Text Entry, `ContextMenu`-Style Mouse (COA C)

**Date:** 2026-09-22
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

The request was a new popup, "similar to the Inspect one — usable both with keyboard or mouse,"
that lets the appearance be customized the way `appearance.toml` already allows, plus a way to
reset back to defaults. Three COAs were framed before coding, against what the codebase already
had: `appearance.toml` covers 19 theme colors, `border_type`, `separator`, `ui.glyphs`, ~20
per-element text styles and a font table — too much to expose as one flat list — and neither
existing popup (Inspect, Settings) is actually mouse-*driven*: both only ever treat a click as
"dismiss," never as "act on the row under it." The real mouse+keyboard precedent in this codebase
is `ContextMenu` (`hover_at`/`click_at`/`key` all driving the same state), not either popup.

#### Options Considered

**A**, grow the existing `SettingsPopup` with one row per field, still cycle-only like its Theme
row already is, with click-to-cycle added. Smallest diff, but a color field cycling through a
handful of presets isn't real customizing, and ~40 rows makes for a long scrolling list. **B**, a
full "Appearance" popup with a category submenu (Theme colors / Border & separator / Glyphs /
Styles / Font) reusing `ContextMenu`'s submenu pattern, each leaf field opening free-text entry
for arbitrary values — full parity with `appearance.toml`, but the largest scope: a new
two-level submenu layout, new text-entry-inside-a-popup interaction, and ~40 fields' worth of new
UI and tests, all in one cycle. **C**, the same free-text entry as B but only for a curated
subset — the highlight colors most people tweak first, plus the three fields that are naturally
cycle-shaped — with the rest deferred.

Chosen and suggested: **C**, on request, with B kept on the roadmap (see *Full appearance editor*
under Medium Priority) rather than dropped. The six colors are `accent_fg`, `border_focused_fg`,
`selection_bg`, `dir_fg`, `bar_bg` and `danger_fg` — the ones that visibly change the UI's
character at a glance, as opposed to per-file-kind colors (`source_fg`, `config_fg`, ...) or ones
that rarely differ from a neighbour (`title_fg`, `file_fg`, `status_fg`). `border_type`,
`separator` and `ui.glyphs` already only ever take one of a handful of fixed strings, so they got
the settings popup's existing cycle treatment rather than free text — there is nothing to "type"
for them that a cycle doesn't already cover.

#### Mouse model and geometry

`appearance_popup::hit(area, pos) -> Hit` (`Row(usize)` / `Inert` / `Outside`) mirrors
`context_menu::row_at` and `disk_usage_view::row_at`'s "answer from the same geometry the renderer
draws with" shape, but sized off `overlay_view::panel_area` (the Inspect/Settings popups' own
layout helper) rather than `ContextMenu::layout`'s anchored-at-the-pointer one, since this popup
is centered like those two, not opened at a click point. `click_row(row_index)` then runs the
row's action exactly as `Enter` would after moving the cursor there — a deliberate simplification
against `ContextMenu`'s continuous hover-highlight-follows-the-mouse behavior, whose real purpose
there is driving submenus; with no submenus here, tracking hover separately from the keyboard
cursor would be machinery this popup has no use for.

#### Live edits, layering and "the defaults"

Committed color/border/separator overrides live in a `theming::RawTheme` (`live_appearance` in
`main::run`) rather than a second full `Theme`, reusing the existing "some fields set, most
`None`" shape `[theme]` overlays already use. New `Theme::overlay_raw(&self, &RawTheme) -> Theme`
layers it onto *whichever* theme is currently in effect — `config.theme`, or a palette the
settings popup's Theme row picked — rather than only onto a named base palette, which
`RawTheme`'s existing `From` impl needed (base palette, then field overrides) but couldn't reuse
directly. Rewriting that `From` impl as `Theme::named(...).overlay_raw(&raw)` removed a second,
drifting copy of the same 19-field list. A color row's in-progress (uncommitted) edit previews
live on top of everything else, via a small pure `appearance_popup::preview` — the same one-field
substitution `overlay_raw` does per-field, just for a value that hasn't been committed to
`RawTheme` yet.

"Reset to defaults" was read as "clear what *this popup* changed this session," not "force the
literal built-in neon palette" — it clears `live_appearance` and the popup's own `live_glyphs`
pick, falling back to whichever theme/glyphs were already in effect (the adaptive auto-detected
one, an explicit config pin, or a settings-popup palette switch, whichever applies). Forcing neon
specifically would silently undo an unrelated choice (the settings popup's Theme row, or the
adaptive default from the previous cycle) that this popup never touched and has no business
overriding.

#### Trade-offs and what was left

Glyphs is the one row that touches `theming::Ui` rather than `Theme`/`RawTheme`; it gets its own
`live_glyphs: Option<GlyphSet>` in `main::run`, parsed from the cycled string via the existing
`GlyphSet::parse`. `Appearance…` is only offered on blank space's right-click menu, not on a file
or folder's — it isn't about a specific entry, matching how the menu already scopes Inspect and
Disk usage to what makes sense for the target. The remaining ~13 theme colors, every `[style]`
element's flags and the font table stay config-file-only until the deferred full editor (COA B)
is built.

#### Verification

`scripts/check` passes: `cargo fmt --all -- --check` clean, `cargo clippy --workspace
--all-targets -- -D warnings` clean, `cargo test --workspace` green (`tui` 345, up from 328;
`theming` 53, up from 52 for the new `overlay_raw` test). New coverage: cursor movement and
wraparound, `h`/`l` cycling a cycle row but doing nothing on a color row, the full
edit-type-backspace-commit-cancel sequence, the Reset row being an action rather than a color or
cycle, `click_row` reaching the same outcomes as the keyboard path and abandoning an in-progress
edit on another row, `next_in`'s wraparound and unrecognised-value fallback, `AppearanceView`
reading the right theme field per row, `preview`/`commit` touching only the field they're asked
to, `hit`'s row/inert/outside geometry, and two property tests (the cursor always stays in bounds;
`hit` never names a row that doesn't exist). Also updated: `context_menu`'s blank-menu label-list
test and its disk-usage test (no longer assuming Disk usage is the last entry, now that
Appearance… is). Not verified against a real terminal or PTY session — no interactive TTY in this
sandbox, so the popup's on-screen rendering, its live-preview caret, and the actual mouse
click-to-row mapping at real screen coordinates were checked by reading the code and the unit
tests (which exercise the same `hit`/`panel_area` geometry `overlay_view::render_appearance` and
`main`'s mouse handler use), not by running the compiled binary interactively.

---

### Persisting the Settings and Appearance Popups: a Third, Program-Owned `local.toml` (COA B)

**Date:** 2026-09-22
**Author:** deltaog-117
**Status:** Confirmed

#### Context / Background

Asked directly, after finishing the appearance popup: "does it save the changes I make using
it?" It didn't — like the settings popup, it was session-only. The request was to persist both
popups' changes, and, separately, to make Catppuccin and Nord (already built-in palettes)
pickable *from the appearance popup itself* rather than only from the settings popup's Theme
row, which the previous cycle had left untouched.

#### Options Considered

**A**, an in-place edit: parse `config.toml`/`appearance.toml` with `toml_edit` (a lossless,
comment-preserving TOML document type) and surgically set just the changed keys, writing the
document back out. Matches the literal ask most closely — "my edit is now in my file" — and is
what `ROADMAP.md` had already flagged as the eventual target. But this program has never written
to a user's dotfiles before; pairing that first write with a new dependency and the general
complexity of a lossless-edit library felt like the wrong place to spend the "first time we ever
touch the user's files" budget. **B**, a third file, `local.toml`, that only the popups ever read
or write — `config.toml`/`appearance.toml` stay exactly as hand-authored, always. **C**, append a
generated `[theme]`/`[panels]`/`[ui]` block to the end of the existing files, stripping a
previous generated block by a marker comment before re-appending. Rejected outright once actually
worked through: TOML forbids a duplicate `[theme]` table in one file, so this needs the same
string-surgery risk as A (stripping the user's *own* conflicting values, not just the generated
block) for none of A's benefit.

Chosen and suggested: **B**, on request, with A kept on the roadmap rather than dropped — B is a
reasonable permanent answer on its own, not just a stepping stone, since "nothing but Minuteman
touches this file" is a stronger guarantee than "an edit library was careful."

#### Keeping `local.toml` from masking a later hand-edit

The one real design risk with B: if `local.toml` ends up carrying a full snapshot of the
resolved theme (everything `config.theme` had, not just what a popup changed), it would silently
win over the *literal same fields* if the user later hand-edited `appearance.toml` — reintroducing,
one layer removed, the exact problem B exists to avoid. The fix is the same discipline
`RawTheme`/`RawUi`/`RawPanels` already use for `config.toml`/`appearance.toml`'s own layering:
`local.toml`'s in-session accumulator (`local_theme`/`local_ui`/`local_panels` in `main::run`) is
seeded once, at startup, from whatever `local.toml` already held — not from the fully resolved
`config.theme` — and every popup commit sets exactly one field on it, never the whole structure.
`RawPanels` needed the same treatment `RawTheme` already had: a new `overlay` (for layering
`config.toml` under `local.toml`) and `PanelsConfig::overlay_raw` (for applying the sparse
override onto a resolved `PanelsConfig`, mirroring `Theme::overlay_raw` from last cycle).
Confirmed empirically, not assumed: a scratch `toml::to_string` call showed the crate already
skips a `None` field entirely with no `skip_serializing_if` needed, which is what makes an
`Option`-heavy `Raw*` struct serialize sparsely for free. `Config` now exposes `local.toml`'s
three tables both unmerged (`local_theme`/`local_ui`/`local_panels`, for seeding `main`'s
accumulators) and merged into `theme`/`ui`/`panels` as before (for everything that just wants the
resolved values) — the same "raw next to resolved" shape `theme_is_customized` set a precedent
for last cycle.

`theme_is_customized` (from the adaptive-default cycle) now also layers in `local.toml`: a theme
picked and saved from the popup correctly counts as "customized" on the next launch, so it isn't
silently swapped back out by the OSC 11 auto-detect the moment the popup that set it is closed.

#### Moving the Theme row

Cycling a base palette (including the newly-requested Catppuccin/Nord access) from *inside* the
appearance popup, rather than leaving it on the settings popup, was the more coherent split once
both popups existed side by side: the appearance popup already owns "what does the theme look
like," and a Theme pick is the base the popup's six field-level color overrides layer onto
anyway — splitting "pick the palette" and "tweak it" across two separate popups only invited them
to disagree about what's currently active. `RawTheme.name` already existed as a field (used by
`config.toml`/`appearance.toml`'s own parsing) but had gone unused by the popup's `commit`
function until now; routing the Theme row through it turned out to need no special-casing beyond
what `Glyphs` already needed for `RawUi`. The one new piece was `appearance_popup::theme_name`:
since a resolved `Theme`'s fields can't always be reverse-mapped to "which named palette is this"
(several palettes could coincidentally share a field's value, though none currently do), it
prefers an explicit `local_theme.name` and falls back to finding a named palette whose *entire*
resolved output matches exactly — which incidentally fixes a pre-existing display quirk from the
settings-popup version of this row, which always started labeled "neon" regardless of what
`config.theme` actually was (an `appearance.toml` pin, or the previous cycle's adaptive default).

#### Trade-offs and what was left

A save reports a write failure (e.g. a read-only filesystem) in the status line rather than
treating it as fatal — the edit is already live for the rest of the session either way, saving is
strictly in addition to that. `local.toml` gets no `local.example.toml` and no `minuteman
init-local` — unlike the other two files, nothing about it is meant to be hand-authored, so an
example file would only invite editing it directly, which works but isn't the intended path (use
the popups; edit `config.toml`/`appearance.toml` for anything meant to be a permanent baseline).

#### Verification

`scripts/check` passes: `cargo fmt --all -- --check` clean, `cargo clippy --workspace
--all-targets -- -D warnings` clean, `cargo test --workspace` green (`tui` 346, `theming` 57).
New coverage: `local.toml` winning over both hand-edited files while still being exposed unmerged
on `Config`, `RawLocal` round-tripping through its own `Deserialize` and serializing only the
fields actually set, `PanelsConfig::overlay_raw`/`RawPanels::overlay` (mirroring `Theme`'s own
tests), the appearance popup's Theme row cycling through `THEME_NAMES` and `RawTheme.name` via
the now-uniform `commit`, and `appearance_popup::theme_name`'s three-way fallback (explicit pick,
resolved-value match, first name). All of the appearance popup's row-index-dependent tests were
re-derived for the new eleven-row layout (Theme now at index 0). Not verified against a real
`~/.config/minuteman/` round trip in an actual second launch — this sandbox has no interactive
session to run the compiled binary in twice and confirm a saved pick is still there; the file
I/O in `Config::save_local`/`Config::load` itself is exercised only by `from_sources`'s pure
logic on in-memory strings, the same boundary this project's tests have always drawn around
filesystem access.

---

### Plugin System: Out-of-Process JSON-RPC over stdio, Sandboxing Deferred (COA B)

**Context.** The roadmap's Low Priority section had long carried a WASM/Extism plugin host as
the answer to "let people extend Minuteman in any language without recompiling it." Asked for
three COAs to get there, the user picked out-of-process JSON-RPC over stdio (Neovim/LSP-style)
over the WASM host and over an embedded Lua-only tier, specifically because it is the only one of
the three that needs *no compile step at all* for the plugin itself — a bare Python, shell or
Node script runs as-is. Sandboxing was discussed explicitly: WASM's appeal is that a plugin gets
no ambient filesystem/process access by default, only what the host API hands it; the chosen
transport has none of that (a plugin runs with Minuteman's own OS permissions), which is an
accepted trade for now and the reason the WASM entry stays on the roadmap as a later, sandboxed
tier to sit *alongside* this one rather than be replaced by it — relevant once a community plugin
registry (Long-Term Vision) means running code nobody local wrote.

**Protocol.** New `plugins` crate, depending only on `shared`/`file_ops` (for the operations) and
`tokio`/`serde`/`crossterm` — not on `theming` or `browser`, so deleting it only breaks `tui`'s
wiring, never theirs. One JSON object per line, deliberately close to JSON-RPC 2.0 rather than a
bespoke shape, and line-delimited rather than LSP's `Content-Length` framing: a plugin author in
any language needs nothing more than "read a line, parse JSON, print a line." Host → plugin is
always an `event` notification (`init`, once at spawn; `key`, when the plugin's configured key is
pressed) carrying the API version, current directory and selection. Plugin → host is either a
`log` notification (fire-and-forget, surfaces as the status bar message) or a request — `read_dir`,
`copy`, `mv`, `delete`, `create_dir`, `create_file`, `touch`, `rename` — reaching the exact same
`file_ops` orchestration the built-in keys use, not a bolted-on read-only subset. Requests and
their params are parsed manually (`RawIncoming` → `PluginRequest::parse` per method) rather than
via a single `#[serde(tag, content)]` enum, so a malformed or unknown line from an untrusted
external process always fails as data, with the original request `id` preserved so the host can
still reply with an error rather than leaving the plugin hanging.

**Concurrency.** Mirrors the existing `LiveRefresh`/`MarkedSize`/`GitStatus` shape exactly: a
`tokio::runtime::Handle` passed in at construction, `unbounded_channel`s for cross-task
communication, and a `poll()` the render loop calls once a tick — no new concurrency pattern
introduced. Per plugin: a `write_loop` task drains an unbounded queue onto the child's stdin (so
firing an event from the render loop never blocks), a `read_loop` task parses stdout line by
line and runs any `file_ops` call on `spawn_blocking` before replying, and a `reap` task awaits
the child so `kill_on_drop` has something alive to act on for the process's whole lifetime and an
abnormal exit gets one log line (a clean exit stays silent). `Command::spawn` needs the calling
thread's tokio reactor bound via `handle.enter()` even though `PluginManager::spawn` itself is
synchronous — the one non-obvious wiring detail, caught by the integration test panicking with
"no reactor running" before the fix.

**Config and dispatch.** `theming::config` gained `[[plugin]]` (`name`, `command`, `args`,
`on_key`), mirroring the existing `[[open_with]]` precedent exactly; `on_key` resolves through
the same `keymap::parse_key` every other binding uses (now `pub(crate)` instead of private) so a
plugin's key string means the same thing everywhere, and an unrecognised string just means no
binding rather than a startup failure. `tui::App` gained a `plugins: PluginManager` field spawned
the same way `git`/`marked_size` already are (a `with_plugins` builder, a bad spawn logged rather
than fatal), and a genuinely unbound key (`config.keys.resolve` returns `None`) now routes to
`App::dispatch_plugin_key` rather than doing nothing — additive only, the built-in keymap and
`Action` enum are untouched.

**Trade-offs and what was left.** No sandboxing (see Context). No plugin → host keybind
registration RPC — a plugin's key comes only from its own `[[plugin]]` config entry, one key per
plugin; a plugin binding multiple keys or a `:plugin` manual-trigger command is future scope, not
built speculatively. No supervisor/restart on crash. The fixture plugin used for the integration
test hand-writes JSON text rather than reusing `plugins::protocol`'s Rust types, specifically so
the test exercises the wire format itself — the same thing a plugin in any other language would
have to get right — not two ends of the same struct agreeing with itself.

**Verification.** `scripts/check` passes: `cargo fmt --all -- --check` clean, `cargo clippy
--workspace --all-targets -- -D warnings` clean, `cargo test --workspace` green (`plugins` 15 unit
+ 3 integration, `theming` 59, `tui` 346). The integration suite spawns the real fixture binary as
a genuinely separate OS process through `PluginManager`, fires a `key` event exactly the way `tui`
will, and asserts the fixture's `read_dir`/`create_file` requests reached the real filesystem —
not mocked at any layer. Property tests (`proptest`) cover `WireEntry`'s conversion from
`shared::DirEntryInfo` for arbitrary names/sizes/timestamps. Not verified: an actual Python, Lua
or shell plugin script running interactively inside the built `mman` binary — this sandbox has no
interactive session to drive that by hand, so the "any language" claim rests on the wire-format
unit tests (raw JSON text, not the crate's own types) plus the process-boundary integration test,
not a literal non-Rust script exercised end to end.

---

### Appearance Popup: Color Picker and Saved Custom Themes (COA A)

**Date:** 2026-09-23
**Author:** deltaog-117
**Status:** Confirmed

**Context.** Asked directly to enhance the appearance popup three ways: an actual color picker
(not just typed hex/names), the ability to save a created theme under a name, and — since editing
a color now implicitly makes "a different theme" from whatever it started as — a way to say
whether a save should update the theme it started from or create a new one. Given three COAs: (A)
extend the existing popup and `local.toml`'s single theme override with an HSV picker mode plus a
list of named, fully-specified theme snapshots; (B) split into a separate Theme Manager popup
(list/rename/delete/duplicate) and a standalone full-screen picker overlay; (C) a minimal version
— a swatch-grid picker instead of full HSV, an explicit "Save as…" that always creates a new
theme, no automatic divergence detection. Chosen and suggested: **A** — the only option that
delivers a real picker *and* the automatic update-vs-new behavior without doubling the popup
surface area (B) or quietly dropping part of what was asked for (C).

**Color math.** New `theming::color` module: pure `Hsv { h, s, v }` ↔ `#rrggbb` conversions, no
`ratatui`/`crossterm` dependency — matching the rest of `theming`, which only ever deals in color
*strings*, never a resolved `Color` (that stays `tui::style::parse_color`'s job; `theming::color`
has its own small hex parser rather than reaching across the crate boundary for `tui`'s private
one). Property-tested: any RGB byte triple round-trips through HSV within ±1 per channel (float
rounding), and `Hsv::clamped` always leaves a value `to_hex` can convert regardless of how far a
slider nudge pushed `h`/`s`/`v` out of range — `h` wraps (it's an angle), `s`/`v` clamp.

**Picker UI.** A color row's in-progress edit (`appearance_popup`'s private `Edit` enum) now has
two modes, `Text`/`Picker`, toggled by `Tab` mid-edit — converting the value across the toggle
(parsing the typed hex, or formatting the HSV back to hex) so neither mode loses what the other
set. In picker mode, `Up`/`Down` select which of H/S/V a subsequent `Left`/`Right` nudges (a fixed
5-unit step for all three, for a consistent key feel despite hue's wider range), and `Enter`
commits the resulting hex exactly like a confirmed text edit would. `overlay_view::render_appearance`
grew a two-cell colored swatch next to every color row (`style::color` on whatever's live — the
in-progress edit if there is one, the committed value otherwise), and the edited row shows either
the typed buffer with its caret or a bracketed `[H 210°] S 80% V 100%  #1ac8ff` readout naming the
selected channel.

**Saving.** A new `Row::SaveTheme` action row, next to `Reset`. `theming::RawLocal` gained
`custom_themes: Vec<CustomTheme>` (`{ name, theme: RawTheme }`, each a *full* color snapshot via
new `RawTheme::from_theme` — every field `Some`, not an overlay — since a saved theme has to look
identical regardless of what `config.toml`/`appearance.toml` do later) and
`active_custom_theme: Option<String>` (which saved entry, if any, the live look is currently
tracked against). The popup itself still never touches `Config` or compares themes — new
`main::classify_save` does that on `Outcome::WantSaveTheme` and hands the popup a `SaveChoice`
(`New` / `UpdateOrNew(name)` / `Unchanged(name)`, the last one short-circuited before the popup
ever sees it, since there's nothing to save) to drive its own two-step flow: an `u`/`n` choice when
there's something to offer updating, then a name prompt either way. `Reset` clears
`active_custom_theme` only — the saved themes list itself is untouched, since resetting the *live*
look to defaults doesn't un-save anything.

**Deferred.** Reselecting, renaming or deleting a saved custom theme from the popup — the Theme
row's cycle still only knows the five fixed built-in palettes (`RowKind::Cycle` takes a
`&'static [&'static str]`, which a dynamic, session-loaded list of saved names doesn't fit without
either leaking strings or a small redesign of that row's cycling). Left for a follow-up rather
than folded in here, to keep this cycle's scope to what was actually asked for.

**Verification.** `scripts/check` passes: `cargo fmt --all -- --check` clean, `cargo clippy
--workspace --all-targets -- -D warnings` clean, `cargo test --workspace` green (`theming` 71,
`tui` 359). Also verified against the real compiled binary via a scripted PTY session
(reconstructed through `pyte`, with the kitty-graphics APC startup query stripped from the raw
stream before feeding it — the same stripping this project's PTY sessions have needed before):
opening the popup showed the new swatch next to every color row; editing Accent, pressing `Tab`,
then nudging hue and saturation with the arrow keys visibly changed both the on-screen readout and
the swatch's actual rendered color, and `Enter` committed the exact resulting hex as plain text;
"Save theme" with nothing active yet asked for a name, and after saving, that same row's own value
flipped live from "new theme" to "updates 'verify-theme'" while the status line reported the save;
`local.toml` afterward held both the pre-existing field-level `[theme]` override (unchanged
mechanism) and a new `[[custom_themes]]` entry whose full color snapshot matched the edited theme
exactly. One harness-only gotcha found and worked around, not a bug in `mman`: the startup
DA1/kitty-keyboard/OSC-11 capability probes can still be mid-flight even after the first real
frame has rendered, and a keystroke sent into that window can be swallowed — the same category of
finding the Image Preview Concurrency and Command/Search Bar Mechanism entries already documented,
just a new instance of it. The verification script now retries its first keystroke until the
popup is actually visible on screen rather than assuming a single send always lands.

---

### Built-in Trash: `d`/`D`, `:trash`, OS-Integrated (COA B)

**Date:** 2026-09-23
**Author:** deltaog-117
**Status:** Confirmed

**Context.** Asked to add a built-in trash bin, reachable via `:trash` and a keybinding. Three
COAs were raised for how the trash mechanism itself should work: (A) a hand-rolled trash directory
under `~/.local/share/minuteman/trash`, moved into via the existing `Vfs`/`file_ops::mv`, with a
`.trashinfo`-style sidecar recording the original path; (B) delegate to the real OS/desktop trash
via the `trash` crate, landing in the same place Nautilus/Dolphin/Explorer/Finder do; (C) a full
subsystem with its own restore-by-id index and a browsable `:trash` listing popup, matching the
roadmap's older "trash + undo history" item in one pass. Chosen: **B**, at the user's explicit
direction — the goal was integrating with the same trash other file managers and the OS already
provide, not a private minuteman-only one.

**SSH vfs.** `file_ops`'s other operations are all `Vfs`-generic so they work over any backend
(including a future SSH vfs), but the desktop trash has no such generic form — there's no "SSH
trash". Asked directly what should happen there: agreed a non-local `Vfs` should fall back to a
permanent delete rather than either teaching `Vfs` a trash primitive it can't honor everywhere, or
inventing a project-private remote trash convention. Moot in practice today — every real call site
in `tui::app` still hands a concrete `LocalVfs` — so `file_ops::trash` takes a plain `&Path`, no
`Vfs` parameter, with a doc comment stating the fallback a future caller needs.

**Keybinding.** The original ask was `space t` for the trash keybinding, but that chord was
already bound twice over (flips a shell pane's split orientation when one is open; opens the
settings popup when none is) — surfaced during design, and the user picked a different scheme
instead: bare `d` now sends the marked-or-selected entries to the desktop trash (`Enter` or `y`
confirms it, since it's reversible), and `D` (Shift+d) keeps the previous behavior — a permanent,
trash-bypassing delete, confirming with `y` only — the same convention a desktop file manager's
Delete vs. Shift+Delete uses. New `theming::Action::DeletePermanently` (default key `"D"`) sits
alongside the existing `Action::Delete`, whose own meaning changed from "permanently delete" to
"trash".

**Command-line naming collision.** The `trash` crate on crates.io shares its name with the empty
`crates/trash` workspace member already scaffolded for this feature — `cargo add trash` (and
`cargo add trash --rename os_trash`) both refuse outright with "cannot add trash as a dependency to
itself", since the CLI checks the argument against workspace member names before considering a
rename. A manually written manifest entry (`os_trash = { package = "trash", version = "5.2.9" }`)
has no such restriction and resolves from the registry correctly — `cargo add`'s refusal is a
CLI-only guard, not a real limitation of Cargo's dependency resolution.

**Wiring.** New `trash::send` (in that scaffold crate) wraps `os_trash::delete`, converting its
error to plain text rather than depending on `os_trash::Error`'s own trait shape. `file_ops::trash`
exposes it alongside `copy`/`mv`/`delete`/`rename`, gaining `file_ops` a dependency on the tiny
`trash` crate and `FileOpsError` a new `Trash` variant — the natural place for a fourth kind of
file operation, even though this one bypasses `Vfs`. In `tui::app`, `Prompt::ConfirmTrash` and
`BulkKind::Trash` mirror `ConfirmDelete`/`BulkKind::Delete` exactly (same batching via
`marked_or_selected`, same spawn-on-the-blocking-pool shape), so `poll_bulk`'s completion handling
— reload, prune marks, reload-and-prune on partial failure — needed only its `is_delete` check
widened to include `BulkKind::Trash`, not new logic. `:trash` at the `:` prompt calls the same
`App::begin_trash` the `d` key does; any trailing text after "trash" defers to the shell instead
(`:trash --empty`, `:trash foo.txt`), the same "bare form only" convention `mkdir`/`touch` already
use for a flag they don't recognize, so a real `trash` CLI on the user's `$PATH` stays reachable.

**Verification.** `scripts/check` passes: `cargo fmt --all -- --check` clean, `cargo clippy
--workspace --all-targets -- -D warnings` clean, `cargo test --workspace` green (new: 2 in `trash`,
1 in `file_ops`, plus `theming`/`tui` keymap and command-parsing tests for `D` and `:trash`). Also
verified against the real compiled binary via a scripted PTY session (answering the Device Status
Report and Device Attributes startup probes by hand — this project's established fix for a
synthetic PTY that never answers them on its own): pressing `d` then `Enter` on a real file in a
scratch directory removed it from that directory and produced a matching entry in the actual
freedesktop trash — `$topdir/.Trash-<uid>/files/` plus a correct `.trashinfo` sidecar recording the
original path and deletion time, since the scratch directory lived on a different filesystem than
`$HOME` (confirming the crate follows the spec's per-mountpoint fallback, not just the common
`~/.local/share/Trash` case).

---

### Syntax Highlighting in the Text Preview: a Curated `TokenKind` Set, Tokenized Off the Render Thread

**Date:** 2026-09-25
**Author:** deltaog-117
**Status:** Confirmed

**Context.** The roadmap's "preview extras, stage 2" asked for coloring source and config files
in the text preview with `syntect`, using the theme's palette where it can. A prior pass (see the
`feat(preview)` commit just before this entry) had already added `preview::highlight` — a
tokenizer that parses a file with `syntect`'s bundled Sublime-syntax grammars and classifies each
scope into `TokenKind::{Keyword, String, Comment, Number, Function, Type, Plain}` — and six new
`syntax_*_fg` fields on every bundled theme, but nothing in `tui` ever called it: the preview pane
still rendered every text file in one flat color. This cycle wired that tokenizer into the actual
render path.

**Curated kinds, not one color per scope.** `syntect`'s grammars produce dozens of distinct scope
names (`storage.type.rust`, `keyword.operator.arithmetic`, `entity.name.function.member`, …).
Mapping each to its own theme field would mean a theme needing tens of new colors, most of which
would go unused by most languages. `classify_scope` instead matches scope *prefixes* into the same
seven kinds every language shares, so one small palette (already sized to fit the appearance
popup's existing color-editing UI) covers Rust, Python, TOML, and everything else `syntect`
bundles a grammar for. An unrecognized extension, or a scope prefix this project doesn't classify,
falls back to `Plain` — the same color the preview already used before this feature existed, so a
file with no known syntax looks exactly as it did before.

**Off the render thread, alongside the read.** `text_preview::TextPreview` already reads a
selected file's bytes on `tokio::spawn_blocking` so a slow disk can't stall input — the same
treatment `image_preview` gives image decoding. Tokenizing a whole file is comparable work (a
full pass over every line), so it runs inside that same blocking closure rather than on every
render tick: `ReadOutcome` grew a `highlighted: Option<Vec<HighlightedLine>>` field, computed from
`preview::highlight::highlight` right next to the `preview::load` call, and carried into
`TextPreview` alongside `content` with the same generation-based staleness check the read itself
already used. The two fields are set together in every code path (a fresh read, a reload, a
failed/unsupported result clearing both) — which let `preview_view::draw_text` treat "the content
is `Loaded::Text`" and "highlighted tokens exist" as one invariant rather than two independently
nullable fields, resolved with a documented `.expect()` at the one call site instead of a second,
never-really-reachable fallback branch.

**Clippy's argument-count lint caught a real smell.** `draw_text` initially grew to eight
parameters (frame, area, inner, the raw text, the highlighted tokens, the row cache, the scroll
state, and the config) to support a defensive `None`-highlighted fallback that reconstructed a
plain paragraph from the raw text. `clippy::too_many_arguments` (default threshold 7) flagged it.
Rather than silence the lint, dropping the now-provably-dead fallback (and the `text: &str`
parameter it existed for) fixed the count *and* removed the redundancy — the raw text was only
ever needed to reconstruct what `highlighted` already carries.

**Verification.** `scripts/check` passes: `cargo fmt --all -- --check` clean, `cargo clippy
--workspace --all-targets -- -D warnings` clean, `cargo test --workspace` green (two new tests in
`preview_view`: a recognized language colors a keyword differently from plain text, and an
unrecognized extension still renders — both by reading the actual foreground color ratatui wrote
into the test-backend buffer's cells, not just the text). Also verified against the real compiled
binary: opened this project's own `crates/preview/src/highlight.rs` in a scripted tmux session
(`COLORTERM=truecolor`, captured with `tmux capture-pane -e` for the raw escape codes) under the
auto-detected Catppuccin Mocha theme, and confirmed six-for-six that `let`/`mut`/`for`/`if`/
`continue`/`else` rendered in `38;2;203;166;247` (the theme's own `#cba6f7`), function calls in
`38;2;137;180;250` (`#89b4fa`), `Vec`/`Ok` in `38;2;249;226;175` (`#f9e2af`), comments in
`38;2;108;112;134` (`#6c7086`), and a string literal and a number in `38;2;166;227;161` (`#a6e3a1`)
and `38;2;250;179;135` (`#fab387`) respectively — an exact byte-for-byte match against the theme's
published hex values, not merely that the preview rendered without panicking.

---

### External Previewers for PDF/Video: a New Hook Layer Reusing Both Existing Pipelines (COA A)

**Date:** 2026-09-25
**Author:** deltaog-117
**Status:** Confirmed

**Context.** The roadmap's "preview extras, stage 3" asked for a config-driven hook that runs a
user-chosen command (`pdftoppm`, `ffmpegthumbnailer`, ...) and feeds its image or text into the
existing preview pipelines, falling back quietly to the file name when the tool is missing or
times out. Three COAs were raised for where the hook mechanism itself should live: (A) a new
`tui::preview_hook` layer that runs the command and hands its image/text result into
`ImagePreview`/`TextPreview`'s existing pipelines, keeping `preview` free of config and process-
spawning; (B) push the hook config and its execution down into the `preview` crate itself, which
already owns "what to preview"; (C) support image-producing hooks only this cycle, folded
directly into `ImagePreview`, deferring text hooks. Chosen at the user's direction: **A** — it is
the only one that honors the roadmap's own "image *or* text" wording without giving `preview` a
new dependency on `theming` and a kind of work (spawning arbitrary user commands) it has never
needed, and `git_status.rs` already gave a proven, tested template for the exact spawn/poll/
timeout/kill mechanics a hook needs, worth mirroring directly rather than inventing a new shape.

**Config shape.** A new `[[preview_hook]]` table (`theming::PreviewHook`) mirrors `[[open_with]]`:
`extensions` (matched case-insensitively), `command` (run under `sh -c`, with `{in}`/`{out}`
substituted the same way `open::command_line` substitutes `{}`), `kind` (`HookKind::Image` or
`Text`, a plain serde enum — a typo'd value falls back to the whole config file's defaults, the
same tradeoff `[[open_with]]`/`[[plugin]]` already accept for a malformed required field), and an
optional `timeout_ms`. `{in}`/`{out}` reuse `open::shell_quote` rather than inventing a second
quoting scheme.

**Failure routing.** The subtlest part: a hook's own failure needs to look different from a real
image file's decode failure. `preview::is_image(path)` staying false for a PDF/video means the
old `is_selected_image = preview::is_image(&path)` gate in `main.rs` would never let a hook-image
file into the image-rendering branch at all — so it changed from a static, path-based check to a
dynamic one, `previews.image.status() != Empty`, mirroring the idiom `is_selected_file` already
used for the text pipeline. `DecodeOutcome::Failed`/`ReadOutcome`'s `Loaded::Failed` both gained a
`via_hook` flag so a hook's failure maps to `Empty` (falls through to the ordinary filename
placeholder, exactly like an unsupported file already does) while a real image's decode failure
still maps to `Failed` ("preview failed", unchanged). Confirmed this doesn't change any existing
real-image behavior: `Previews::update` always runs before the same tick's `draw`, so a real
image's status is never still `Empty` by the time it's read for rendering.

**Reused rather than duplicated.** `preview::load_image` (already used for real image files)
decodes a hook's `Image`-kind output too, rather than `tui` depending on `image::ImageReader`
directly — it needed one change first: `.with_guessed_format()`, since a hook's scratch file has
no reliable extension to trust. `tui` still needed `image` as a direct dependency purely to name
the `DynamicImage` type in `HookOutcome::Image`; it carries no format-decoder features, since
decoding itself stays inside `preview`.

**Verification.** `scripts/check` passes: `cargo fmt --all -- --check` clean, `cargo clippy
--workspace --all-targets -- -D warnings` clean, `cargo test --workspace` green (new:
`preview_hook`'s own 5 tests covering a successful text hook, a successful image hook, a missing
program, a failing exit, a timeout that's killed near-immediately rather than waited out, and an
oversized-output rejection; 2 in `theming::config` for the new table; 2 in `text_preview` for the
hook-covered-file and hook-failure-falls-back-to-`Empty` paths). Also verified against the real
compiled binary via a scripted tmux session: generated a real one-page PDF (a solid red 100×100
image via ImageMagick's PDF delegate), pointed a `[[preview_hook]]` at the real `pdftoppm` binary
on this machine, selected it in the browser, and confirmed the preview pane showed genuine
halfblock image output with `38;2;254;0;0` in the captured escape codes — the source image's own
red, not a placeholder. A second file with an extension hooked to a nonexistent program showed
nothing but its own file name, exactly the "fall back quietly" behavior the roadmap asked for,
confirmed on the real binary rather than assumed from the code. (The first attempt at the
`pdftoppm` example command double-appended `.png`, since `-singlefile` already appends it to
whatever prefix it's given — caught by reproducing the exact substituted shell command by hand
before trusting the compiled binary's result, which is what `config.example.toml`'s shipped
example now reflects.)

---

### UI Overhaul, Phase C (Start): an Animation Tick and Animated Shell-Pane Focus Transitions (COA A)

**Date:** 2026-09-26
**Author:** deltaog-117
**Status:** Confirmed

**Context.** The roadmap's "UI overhaul, phase C — cinematic layer" bundled six unrelated effects
(a boot splash, animated focus transitions, gradient borders/titles, a pulsing selection, a
typewriter preview reveal, an optional system/git HUD) behind one line: "needs an animation tick
on top of the existing 100ms poll." Three ways to slice one cycle's worth of that were raised and
compared before coding:

| COA | Approach | Advantages | Disadvantages | Difficulty |
|---|---|---|---|---|
| A | The animation-tick core plus two payloads that exercise it: a pulsing selection highlight and an animated focus-border transition | Small blast radius, proves the tick mechanism end-to-end on real widgets, both payloads are cheap pure-color computation with no new state machines | Doesn't touch the splash, preview reveal, or gradient titles this cycle | M |
| B | The boot splash only | Fully self-contained and one-shot; no persistent tick needed | Fires once per session; doesn't lay the tick infrastructure the rest of the feature needs later | S |
| C | Everything in the roadmap bullet in one cycle | Matches the bullet completely | Touches nearly every render path in one PR; contradicts this project's own precedent of deferring large multi-piece UI work instead of bounding each cycle (see the appearance-editor deferral); hard to verify, high regression risk | H |

Chosen at the user's direction: **A** — the only option that builds the reusable tick mechanism
the rest of the cinematic layer will need, while staying small enough to verify properly in one
cycle.

**Built, then cut before committing: the pulsing selection.** COA A's second payload — the
selected row's background breathing gently brighter and back on a ~1.4s cosine cycle, via a
`style::pulse` on top of a shared `anim::pulse_phase()` clock — was implemented, unit-and-property
tested, and confirmed live on the real compiled binary (its raw background escape code visibly
moving between two captures 0.6s apart). Before anything was committed, the user tried it and
called it distracting, so it was removed in full: `style::pulse`, `anim::pulse_phase`/
`started_at`/`PULSE_PERIOD`/`PULSE_STRENGTH`/`TICK`, their tests, the `selection_style` call site
in `main.rs`, and the poll-timeout branch that shortened the idle 100ms tick to 33ms whenever a row
was selected all went back out together, leaving only what the focus-border transition needs. What
shipped is the animated focus transition alone; the pulsing selection is still an open roadmap item
(worth trying again with a subtler effect, if at all, but not by default). Recorded here per this
diary's own rule to write an entry when reversing a decision — the lesson is that a real, verified,
compiled-and-tested effect can still be the wrong call on taste alone, and that isn't a defect in
the verification, just a separate axis it doesn't cover.

**The tick.** A new `tui::anim` module holds nothing but pure wall-clock math — a `Duration` in, an
`f64` out, no `ratatui` or `theming` types at all — so `transition_t(elapsed)` is tested directly
rather than through a render: a linear `0.0..=1.0` ramp over 180ms, since a focus transition only
ever plays once per change rather than looping (unlike the pulse it replaced any need for, above).
A shell pane's own poll was already 16ms whenever any shell is open — faster than the 180ms
transition needs to look smooth — so the focus-border fade needed no poll-timeout change of its
own at all; with the pulse gone, the idle poll outside a shell session stays exactly the flat 100ms
it always was.

**Color blending, and its honest limit.** `style::blend_rgb(from, to, t)` linearly interpolates two
resolved `ratatui::Color`s, but only returns `Some` when both are `Color::Rgb` — the case a
truecolor terminal resolves a hex theme color to. A basic ANSI color name (`"white"`, `"cyan"`,
what the bundled `classic` and `dracula` themes use for `border_fg`/`border_focused_fg`) or a hex
color quantized down to a 256-color index on a non-truecolor terminal has no smooth path between
two swatches — interpolating between two arbitrary palette indices would be a guess dressed up as
smoothness, so it returns `None` instead and `shell_layout`'s border blend falls back to whichever
endpoint the pane is actually settled at. That fallback reproduces exactly what the code already
did before this cycle — a theme built entirely from named border colors simply doesn't animate,
rather than freezing on a half-lit or randomly-quantized shade.

**Tracking a focus change without changing `render`'s signature.** `shell_layout::ShellPanes`
already had a plain `focused: usize` field, mutated from half a dozen call sites (`move_focus`,
`split`, `close`, `resize_focused`, ...). Rather than touch every one of them, `render(&self, ...)`
itself now compares `focused` against a `last_seen_focused: Cell<usize>` on every call: a
mismatch means focus moved since the last frame, so it stamps `previous_focused` (the pane that
lost focus) and `focus_changed_at` (an `Instant`) before computing `anim::transition_t` for this
frame. `Cell`, not a plain field, because `render` only ever borrows `&self` — `ShellPanes` is
single-threaded UI state, so the interior mutability costs nothing but is exactly what lets the
bookkeeping live inside the one function that actually needs to observe it, instead of leaking
into every mutator. A new `FocusAnim { focused, previous, t }` is threaded down through
`render_tree` to `popup_shell::render` (which gained a `border: Color` parameter, resolved by the
caller) so each leaf can compute its own border color: the newly-focused pane blends from
`border_fg` toward `border_focused_fg`, the pane that just lost focus blends the other way, and
every other pane (in a tree of more than two panes) simply stays at the plain unfocused color —
never both blending and losing track of who's mid-transition. `style::themed_block` itself is now
a thin wrapper over a new `style::themed_block_with_border`, so every other caller (the modal
popups, the always-focused current-column pane) is unaffected.

**Verification.** `scripts/check` passes: `cargo fmt --all -- --check` clean, `cargo clippy
--workspace --all-targets -- -D warnings` clean, `cargo test --workspace` green (new: `anim`'s own
unit and property tests for the transition math, `style`'s for `blend_rgb`'s endpoints and its
refusal to blend named/indexed colors, and `shell_layout`'s for `FocusAnim`'s settled-state
dispatch and for a real `ShellPanes::split` + `render` sequence actually starting and later
settling a transition). Also verified against the real compiled binary via a scripted tmux session
on a truecolor terminal: opened a shell (`s`), split it (`Alt+n`), and moved focus back (`Alt+z`)
while capturing a frame roughly 50ms later — both panes' border escape codes sat at the *same*
blend fraction (≈0.267, computed independently from each pane's own resolved
`border_fg`/`border_focused_fg`) between their respective endpoints, confirming the two panes
crossfade in lockstep rather than one snapping while the other lags or drifts. (Driving the split
via the documented `Space`-leader chord needs a bare "Alt tap," which depends on the kitty keyboard
protocol's modifier-press events — not available by construction over this scripted tmux session —
so `Alt+n`/`Alt+z` were used instead, a real, independently-documented keybinding that exercises
the identical `ShellPanes::split`/`render` code path.)

---

### UI Overhaul, Phase C (Remainder): Boot Splash, Gradient Borders/Titles, a Typewriter Preview Reveal, and a System HUD — Pulsing Selection Dropped for Good (COA A)

**Date:** 2026-09-27
**Author:** deltaog-117
**Status:** Confirmed

**Context.** Last cycle's "UI overhaul, phase C (start)" built the animation tick and shipped one
payload for it (animated shell-pane focus-border transitions), deliberately deferring the rest of
the "cinematic layer" bullet: a boot splash, gradient borders/titles, a typewriter reveal on the
preview, a pulsing selection, and an optional system/git HUD. The pulsing selection had already
been built, verified, and cut before that commit on taste alone (see the entry above). This cycle
the user said to drop it for good and ship the other four in one pass.

**The one real fork: how the system/git HUD gets its data.** Everything else was a rendering
question with an obvious answer already established by this project's own precedent (see below).
The system segment needed session uptime, load average and memory — and whether that costs a new
dependency was worth measuring, not guessing, before writing any code:

| COA | Approach | Advantages | Disadvantages | Difficulty |
|---|---|---|---|---|
| A | No new dependency: `/proc/loadavg` + `/proc/meminfo`, uptime tracked in-process | Zero new deps, matches every past "shell out or read directly" precedent (`git_status`, `disk_usage`) | Linux-only; degrades to nothing (not an error) elsewhere | S |
| B | The `sysinfo` crate, cross-platform | One clean API, works everywhere, richer data (real CPU%) | New dependency; a second concurrency idiom this project has never needed | M |
| C | Shell out to `uptime`/`free` the way `git_status` shells to `git` | No new dependency | A second background poller alongside git's, and fragile locale-dependent output parsing | M |

Chosen, at the user's direction: **A**, after actually measuring **B**'s cost rather than
estimating it — the release `mman` binary was built once at baseline (9.33 MiB), then rebuilt with
`sysinfo = "0.33"` declared *and actually called* (`System::new_all()` + `refresh_all()`, so the
linker couldn't dead-strip it away), landing at 9.60 MiB: **+276 KiB, ~2.9%**, and eight new
transitive crates (`sysinfo`, `rayon`, `rayon-core`, three `crossbeam-*`) — `rayon`'s data-parallel
thread pool exists only to enumerate processes in parallel, work this project's own `tokio`
blocking-pool idiom already covers everything else with. Not alarming on its own, but a real,
avoidable cost for a segment the roadmap itself calls optional, so **A** won on the same grounds
`git_status` and `disk_usage` already established: read the data directly, no new crate, and treat
a platform where the source doesn't exist as "shows nothing" rather than an error. If Minuteman
ever needs to run somewhere `/proc` doesn't exist, **B** is the one to revisit then — the module
doc for `tui::system_hud` says so directly, so a future cycle doesn't have to rediscover this.

**Gradient borders/titles.** New `tui::gradient` walks a `Rect`'s perimeter in one continuous
order (top edge left-to-right, right edge down, bottom edge right-to-left, left edge up) and
recolors every cell it visits with `style::blend_rgb(from, to, fraction(i, total))` — deliberately
*every* cell, title text included, rather than trying to tell a border glyph from a title glyph:
the effect reads as one ribbon of color sweeping the whole frame, not a flat border with an
oddly-untouched title sitting inside it. It falls back to the existing flat look wherever
`blend_rgb` can't blend the two colors (the same "only two resolved `Color::Rgb` values" rule the
focus-fade transition already lives by), so a theme built from named ANSI colors is unaffected.
Scoped to just the browser's current/focused pane — not the parent/preview panes, not shell
panes (which already animate their border a different way, the focus-fade from last cycle; layering
a second, competing animation on the same border was rejected on sight), and not any modal panel —
to keep this cycle's blast radius to the one place a gradient border reads as "the active thing is
lit up" rather than visual noise.

**Typewriter reveal.** Scoped deliberately to *the very first, unscrolled view of a freshly
selected file* rather than trying to reconcile a wrapped-row viewport position with a source-line
index: `Paragraph`'s wrapping is opaque from outside (there's no API answering "which source line
is wrapped-row 12"), so capping by wrapped row while scrolled would need re-deriving that mapping
by hand for no real benefit — a file that's already been scrolled has already been seen. `total`
(for the scrollbar and `scroll.fit`) is always measured from every source line, unconditionally;
only the *rendered* `Paragraph` is ever a truncated rebuild from `highlighted[..revealed]`, and
only while `revealed < highlighted.len()` — so the common steady state (nothing to reveal) costs
nothing beyond what the pane already did every frame. `anim::REVEAL_LINES_PER_SEC` (90) was picked
so even a tall 70-something-row pane finishes well under a second; `reload()` (the same file
changing on disk while still selected) never restarts the reveal, the same "don't jump back to the
top" precedent `Scroll::reset` already carved out for that path.

**Boot splash.** A fixed 46×7 box, centered and clamped to the frame, with the title/subtitle
fading in from `border_fg` to `accent_fg` over 300ms (again via `blend_rgb`, again falling back
gracefully) and gone after 900ms on its own or the instant any key or mouse event arrives —
checked ahead of every other handler in `run`'s event loop, the same first-claim-on-input
precedent the disk usage/Inspect/menu modals already set. A `Resize` event dismisses it too but
still reaches the ordinary resize handling below, so maximizing the terminal right at startup
doesn't leave a stale splash-sized rect on screen.

**Verification.** `scripts/check` passes: `cargo fmt --all -- --check` clean, `cargo clippy
--workspace --all-targets -- -D warnings` clean, `cargo test --workspace` green (new: `gradient`'s
perimeter/fraction/paint tests, `anim`'s `revealed_lines` tests, `system_hud`'s `/proc` parser
tests — property-tested against arbitrary bytes for "never panics" the same way `git_status`'s
parser is — and `boot_splash`'s timing/area tests). Also verified against the real compiled
release binary via four scripted PTY sessions reconstructed through `pyte` (answering the startup
Device Status Report / Device Attributes / cell-size probes per this project's established fix,
one throwaway `HOME` with a scratch `config.toml` per run): the boot splash showed real
"MINUTEMAN" / "a Ranger-inspired terminal file manager" / "press any key" text over the browser on
the very first frame and vanished the instant a key was sent; the header showed a real
`up now  load 1.45 1.48 1.13  mem 5.0G/15G` segment with `system_hud = true`, sourced from the
actual `/proc` on the machine running the test; a 40-line file's preview showed only `line00` at
30ms after selection and the rest by a second later — genuinely progressive, not a placeholder; a
single frame captured with the default theme carried 139 distinct truecolor `38;2;r;g;b` triples
along the focused pane's border/title, against 9 with `gradient_borders = false`; and with
`boot_splash`/`gradient_borders`/`typewriter_preview`/`system_hud` all set to `false`, the very
first frame showed the full 40-line file at once, no uptime segment, and the same flat 9-color
palette as the gradient-off run — confirming all four switches actually gate their effect rather
than merely existing in `config.toml`.

---

### Caps Lock Header Pill: Static, Not Blinking

**Date:** 2026-09-27
**Status:** Confirmed

#### Context / Background

`main.rs` already reads the keyboard protocol's `KeyEventState::CAPS_LOCK` flag to fix up a
letter's case (`with_caps_lock_applied`, see the earlier "Caps Lock Under the Keyboard Protocol"
entry above), but that flag was never kept anywhere — it was read and discarded once per
keystroke. Asked to "add an icon for when caps lock is activated, loop it," the plain reading of
"loop it" is a blinking or pulsing icon. This project's own diary record says otherwise: a pulsing
selection highlight was built, verified, and cut in full before committing, on taste alone, and is
recorded as "worth trying again with a subtler effect, if at all, but not by default." Rather than
guess against that precedent, the user was asked directly what "loop it" meant here; the answer
was that the icon should be added continuously — kept in sync every time through the main loop —
not that it should blink, "so I can distinguish what commands I can run."

#### Options Considered

**Option A: A static header pill, always kept in sync (chosen)**
- `App` gains a `caps_lock: bool` field and `set_caps_lock`/`caps_lock` methods; `main`'s event
  loop calls `set_caps_lock(key.state.contains(KeyEventState::CAPS_LOCK))` as the very first thing
  it does with every `Event::Key`, ahead of the `Alt`-layer and modal-view branches, so a modal
  view or a prompt owning the keyboard never leaves the pill stale. `HeaderView` gains a
  `caps_lock: bool` field; `render_header` shows a danger-colored "caps lock" pill, using a new
  `Glyphs::caps_lock` glyph (`⇪` for the unicode/nerd sets, empty for ascii, matching how
  yanked/cut/marked already handle a glyph-less set), exactly while it's true.
- No new poll-timeout tier, no animation clock, no new dependency; the pill costs nothing beyond
  what the header already recomputes every frame.

**Option B: A blinking/pulsing pill**
- Matches the literal words "loop it" most directly, but reintroduces exactly the kind of
  continuous animation this project's own diary already tried and rejected as distracting for a
  different indicator (the pulsing selection). Would also need a poll-timeout change (the idle
  tick is 100ms; a convincing blink needs faster) for an indicator whose entire job is to be
  noticed the instant it changes, not to visually compete for attention while it's on.

Chosen at the user's direction, after asking directly rather than guessing: **A** — "loop it" meant
wiring the check into the running event loop so the pill can never drift out of sync with what a
letter key actually does next, not a repeating visual effect.

#### Consequences

`Glyphs` gained one more field (`caps_lock`), threaded through all three glyph sets and the
`glyphs sample` output the same way every other pill's symbol already is — no special-casing.
`App::caps_lock`/`set_caps_lock` are plain synchronous state, no `Instant`/tick machinery, so they
carry no risk of drifting from real time the way an animated effect would. The pill sits first in
the header's pill list, ahead of the progress/clipboard/marks pills, since Caps Lock changes what
every subsequent letter keystroke does and is the one piece of header state worth seeing first.

**Verification.** `scripts/check` passes: `cargo fmt --all -- --check` clean, `cargo clippy
--workspace --all-targets -- -D warnings` clean, `cargo test --workspace` green (408 passed,
including new tests: `glyphs`'s ascii-purity and pill tests extended to cover `caps_lock`,
`hud::tests::the_caps_lock_pill_shows_only_while_it_is_on`, and
`app::tests::caps_lock_starts_off_and_tracks_the_last_key_event`).

---

### Bookmarks / Marks: a Local Chord, Not a New Prompt or File

**Date:** 2026-09-28
**Status:** Confirmed

#### Context / Background

The next High Priority roadmap item was Ranger-style directory bookmarks: `` ` ``/`m` jumps to a
saved directory, `m` saves the current one under a register letter. Before writing anything, a
codebase pass turned up a naming collision the roadmap wording didn't anticipate: `m` was already
the default `Action::Cut` binding (`crates/theming/src/keymap.rs`), and this project's own
`Select` action already owns the word "mark" for per-file multi-select. Ranger's own default key
for saving a bookmark could not be reused as-is.

#### Options Considered

**Option A: a `main.rs`-local pending chord, `local.toml` persistence (chosen)**
- Two new `Action` variants, `BookmarkJump` (default `` ` ``, free) and `BookmarkSet` (default
  `B` — "Bookmark", since `m` was taken), resolved through the same `KeyMap` every other binding
  uses. `run`'s event loop gets a `pending_bookmark: Option<BookmarkChord>` local, set from the
  final action-dispatch match and consumed one keystroke later — the exact shape `shell_chord`
  already uses for the leader's resize/move chords, except one-shot: whichever key completes it
  (a letter/digit register, `Esc`, or anything else) ends it immediately, there is nothing to
  repeat. Registers live in a new `RawLocal::bookmarks: BTreeMap<char, PathBuf>`, mirroring exactly
  how `custom_themes` already rides along in `local.toml` unmerged with `config.toml`/
  `appearance.toml` — no new file, no new format, and `Config::local_bookmarks` follows the same
  `local_*` mirroring every other `local.toml` table already gets.
- A miss (jumping to a register nothing was ever saved under) reports `no bookmark at 'x'` in the
  status line rather than erroring; the status bar's mode pill shows `BOOKMARK`/`SET BOOKMARK`
  while a chord is pending, with its own `hud::hints` row (`a-z 0-9 register`, `esc cancel`) — the
  same treatment every other pending chord in this codebase already gets.

**Option B: a new `Prompt::BookmarkJump`/`BookmarkSet` variant**
- Would reuse `Prompt`'s existing status-bar label/render plumbing for free, but `Prompt` is built
  for blocking, often-destructive flows (`is_destructive()`, full text input) — stretching it over
  a quick one-key register tap is a semantic mismatch with the rest of that enum, for no real gain
  over `shell_chord`'s already-proven shape.

**Option C: a dedicated `bookmarks.toml` plus a browsable picker overlay**
- Richer (not capped at 26 letters, browsable rather than register-only), but a new file format and
  a new overlay rendering surface is well beyond what the roadmap item — explicitly "Ranger-style
  `` ` ``/`m` register" — asked for.

Chosen: **A**. It is the smallest extension of patterns already proven in this exact codebase
(`shell_chord` for the pending-key gesture, `custom_themes` for the `local.toml` persistence
shape), and matches the roadmap's own wording — a register, not a picker.

#### Consequences

`m` stays `Cut`; a dedicated test (`bookmark_set_does_not_shadow_cut`) pins the two apart so a
future rebind of either can't silently reintroduce the collision. `persist_local` and
`apply_appearance_outcome` both grew a `local_bookmarks` parameter threaded alongside their
existing `local_custom_themes`/`local_active_custom_theme` ones, since every save of `local.toml`
writes the whole file back out regardless of which table actually changed (same reasoning as
`local_panels` already being read-only in `apply_appearance_outcome` — see that function's own
doc). `BTreeMap` rather than `HashMap` for `bookmarks`, so `local.toml`'s `[bookmarks]` table comes
out in the same sorted order on every save instead of shuffling for no reason — the same choice
`custom_themes` would have made if a `Vec`'s insertion order weren't already sufficient there.

**Verification.** `scripts/check` passes: `cargo fmt --all -- --check` clean, `cargo clippy
--workspace --all-targets -- -D warnings` clean, `cargo test --workspace` green — `theming` grew
from 76 to 79 passed with three new tests (`keymap::tests::bookmark_set_does_not_shadow_cut` and
`config::tests`' `bookmarks_are_read_from_local_toml_unmerged` /
`a_bookmark_saved_from_the_running_session_round_trips_through_local_toml`); `tui`'s own 408 is
unchanged, since the new dispatch code is exercised by the manual smoke test below rather than a
unit test of `run`'s event loop. Also smoke-tested
against the actual compiled binary in an isolated `tmux` session (a scratch directory tree, a
throwaway `XDG_CONFIG_HOME` so it never touched a real `local.toml`): `B` then `a` saved a bookmark
and showed `bookmark 'a' set`; entering a subdirectory and then `` ` `` then `a` jumped straight
back with `jumped to bookmark 'a'`; `` ` `` then `z` (never saved) showed `no bookmark at 'z'`
without changing directory or crashing; the mode pill read `SET BOOKMARK`/`BOOKMARK` while each
chord was pending; `q` quit cleanly. `local.toml` came out exactly as expected: `[bookmarks]`
followed by `a = "/tmp/bm-test"`.

---

### Full Appearance Editor: Categories and a Two-Axis Color Picker (COA A)

**Date:** 2026-09-28
**Status:** Confirmed

#### Context / Background

The appearance popup covered a Theme pick, six highlight colors, border style, separator and
glyphs; the rest of `Theme`'s ~22 color fields, every `[style]` element's modifier flags and the
font table were still config-file-only. The color picker was also a channel-select-then-nudge
widget (arrow keys over `Hsv`), not the click-into-a-square dialog a desktop color chooser gives.
Both gaps had to close for "a full editor with a VS Code-style picker", so the options below
bundle structure and picker fidelity.

#### Options Considered

**Option A: category submenu plus a true 2D picker (chosen)**
- Five categories (Theme & colors, Border & separator, Glyphs, Text styles, Font), each a flat row
  list with a "‹ Back" row. Every color row can toggle into a saturation/value gradient square plus
  a hue strip, painted in truecolor cells and set by click or drag. Difficulty: high. Closest to
  what was asked and to the design the roadmap had already scoped and deferred; the cost is the
  most new code (a render/hit-test module and the two-level view state).

**Option B: the same 2D picker, one long scrollable list**
- Same picker fidelity with no submenu state, but about 45 rows in a single scroll are harder to
  scan than grouped categories. Difficulty: medium-high.

**Option C: category submenu, picker as three clickable H/S/V bars**
- Cheapest, since it reuses the existing `Hsv`/nudge logic, but it is a slider dialog rather than a
  true square, so it undershoots the request. Difficulty: small-medium.

Chosen: **A**, for the reason the roadmap note had already given: the 2D square is the part of
"like VS Code" that matters, and categories keep the field count navigable.

#### Consequences

`View` tracks the level and `AppearancePopup::rows()` builds the current list fresh rather than
returning a `'static` slice, since which rows exist depends on where the cursor drilled into.
`picker_areas` and the `hsv_from_*_click` functions are pure geometry, and the same rectangles the
renderer paints are what a click is hit-tested against, so paint and hit-test cannot disagree.
Style and font edits go through `RawStyles::set_by_index`, indexed by `STYLE_ELEMENTS`, so the
popup addresses an element by a `Row::StyleElement(usize)` alone instead of a 22-armed match. The
popup's mouse handling in `main.rs` keeps a `dragging_picker` flag, set on a mouse-down inside
the square or strip and cleared on release. `RawLocal` lost its `Eq` derive because `RawFont`
holds an `f64`. The font is never previewed live, since the terminal owns it, so it only feeds a
later `minuteman init-terminal`.

Known gap: the picker always paints `Color::Rgb` cells, with no fallback for terminals without
truecolor, so on a 256-color terminal it will be approximated by the terminal or look banded.

**Verification.** `scripts/check` passes: `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets -- -D warnings` and `cargo test --workspace` are all clean. `tui` runs 410 tests
and `theming` 85. Two things were fixed to get there: rustfmt's import ordering under the 2024
edition, and a `field_reassign_with_default` lint in a `commit` test, which now builds its
expected `RawTheme` with struct-update syntax. The picker was not smoke-tested against the
compiled binary in a real terminal this cycle.

---

### Saved Custom Themes: a "Saved themes" Category, Not a Dynamic Theme Row (COA A)

**Date:** 2026-09-28
**Status:** Confirmed

#### Context / Background

"Save theme" could create and update named themes in `local.toml`, but nothing could load one
back: the Theme row only cycled the five built-in palettes, so a saved theme was write-only, and
there was no way to rename or delete one. A saved theme is a full color snapshot
(`RawTheme::from_theme`), so applying one is a matter of replacing the live overrides with it.

#### Options Considered

**Option A: a "Saved themes" category (chosen)**
- One row per saved theme; `Enter` applies, `r` renames, `d` deletes after a confirmation. It
  reuses the category and flat-row-list machinery the previous cycle built, and leaves
  `RowKind::Cycle` alone. Difficulty: medium. The cost is one more root category, so applying a
  theme is two steps from the Theme row.

**Option B: make the Theme row cycle built-ins plus saved themes**
- Closest to the old roadmap wording, but `RowKind::Cycle` holds a `&'static [&'static str]`, so
  it needs a new kind or an owned list threaded through `AppearanceView`, and rename and delete
  still need a separate manage view. Difficulty: high, for the least new capability.

**Option C: a picker opened from the "Save theme" row**
- Smallest change to the root layout, but it adds a third mode next to `SaveState`'s two and hides
  load and delete behind a row called "Save". Difficulty: medium.

Chosen: **A**, as the smallest extension of what was already built, and the only option that gave
rename and delete a proper home.

#### Consequences

`Row` is `Copy` and its labels are `'static`, so the dynamic list is a `Row::SavedTheme(usize)`
index, the same shape as `Row::StyleElement(usize)`, and `overlay_view` asks
`AppearancePopup::saved_name` for the label instead of `Row::label`. The popup never sees
`local.toml` (the same reason `WantSaveTheme` is resolved by the caller), so it holds a copy of the
names that `main` refreshes through `sync_saved` once per loop iteration and again after every
outcome; `sync_saved` also drops a rename or delete in flight whose target vanished and pulls the
cursor back onto a row that exists. Rename and delete are a small `ManageState` modal, like
`SaveState`, so the row list itself never changes shape mid-edit. Applying a theme replaces
`local_theme` wholesale, which is exact because a snapshot sets every field; it leaves `local_ui`,
`local_style` and `local_font` alone, since a saved theme is colors, border style and separator
only. Deleting the active theme clears `active_custom_theme` but leaves the live look, so "Save
theme" then offers a fresh name rather than an update. The appearance popup also got its own,
wider label column (`APPEARANCE_LABEL_COLUMN`, 22): the shared 13-cell one cut off category labels
and any theme name past 12 characters, and the shared constant is left alone for the other popups.
The root layout gained a row, so two existing tests that index into it by position moved by one.

**Verification.** `scripts/check` passes: format, clippy with `-D warnings` and the whole
workspace's tests are clean. `tui` went from 410 to 419 tests, nine new ones covering the category's
rows, apply, rename (including refusals and the unchanged name), the delete confirmation, the
`r`/`d` no-ops off a saved theme, and `sync_saved`'s cursor and in-flight handling; `theming` is
unchanged at 85. Smoke-tested against the compiled binary in an isolated `tmux` session with a
throwaway `XDG_CONFIG_HOME` seeded with two saved themes: the list showed both with the active one
marked; `Enter` applied each and `local.toml`'s `[theme]` and `active_custom_theme` followed;
renaming the active theme to `dusk` kept it active and rewrote its `name`; `d` then `y` removed it,
cleared `active_custom_theme`, and left the live accent color as it was. The `y`/`n` prompt and
the rename field both rendered in the hint line. Not exercised by hand: a mouse click on a saved
theme, and the context-menu route to the popup, which use the same code paths as `Enter` and `a`.

### Archive Compress/Extract: In-Process Crates and a `SafePath` Newtype (COA A)

**Date:** 2026-09-28
**Status:** Confirmed

#### Context / Background

The browser could list an archive's contents in the preview but not create or unpack one. The
`preview` crate already builds `zip`, `tar` and `flate2`, so the question was how much more to take
on: the archive formats are large, and unpacking untrusted input onto the disk is a classic source
of vulnerabilities, chiefly writing outside the destination (zip-slip) and zip bombs.

#### Options Considered

**Option A: native Rust crates only (`zip`, `tar`, `flate2`), other codecs behind features**
- No runtime dependency and the same behavior everywhere, at the price of writing the path-safety
  code ourselves and having no `rar` or `7z`. Difficulty: medium.

**Option B: shell out to `tar`, `unzip`, `zip`, `7z`, `unrar`**
- Least code and every format, but it depends on what is installed, passes user-chosen names to a
  program's argument parser, and makes progress and cancel awkward. Difficulty: small.

**Option C: native for the common formats, an external tool for the rest**
- Best coverage, but two code paths to test and detect. Difficulty: medium to high.

Chosen: **A**. Zip and tar cover nearly all real use and their crates were already in the build;
the rare formats can wait for a feature flag or a fallback later.

#### Consequences

`file_ops::archive` works on local paths with `std::fs`, not `Vfs` (like `trash`), because an
archive is a stream. Every entry name becomes a `SafePath` before anything is created, so an unsafe
path is unrepresentable past that point (the only constructor rejects it); a proptest checks that
whatever an archive names an entry, joining the result never leaves the destination. The other
rules are enforced where the write happens: a symlinked directory on the way to an entry is
refused, a file is opened with `create_new` (which does not follow a link), and only symlinks whose
target descends are created, since a lexical check of `..` targets is unsound once links chain
(`a -> d/b/..` where `d/b -> ..`). Overwrite uses `remove_file`, which unlinks a symlink itself and
refuses a directory, so an archive cannot redirect a write or delete a tree. Limits count bytes
written rather than sizes the archive declares, and the `.tar.gz` inflate is capped by a reader
that errors instead of ending quietly (the same trap the preview's listing has), because a tar
reader takes a quiet end for a complete archive. Compression writes to `.<name>.<pid>.partial`
and renames on success, so a cancel or error leaves no half archive. The TUI extracts into
`<stem>/` rather than the browsed directory, and reports an existing destination as an error
instead of prompting, since overwriting could clobber a whole tree. The zip crate reads a central
directory whole on open; the preview guards that and extraction does not yet (recorded in the
roadmap). Zip entries carry no modification time, since converting one needs a `time` or `chrono`
feature.

**Verification.** `scripts/check` passes: format, clippy with `-D warnings` and the whole
workspace's tests are clean. `file_ops` went from 19 to 41 tests and `tui` from 419 to 424. The 22
new archive tests cover a round trip of every format (files, nested and empty directories, a
symlink), climbing and absolute names in both tar and zip (including a backslash separator), a
control character in a name, escaping and chained symlinks and a hard link, writing through a
symlinked directory, overwrite over a symlink, clamped modes, the byte limit on a gzip bomb (and
the partial file removed) and exactly at it, the entry limit, all three conflict policies,
overwrite never deleting a directory, damaged and unsupported archives, cancel in both
directions, refusing an output inside a source, and two proptests (a `SafePath` never escapes;
`tar.gz` round-trips random files). The five in `tui` cover parsing and `:compress` then
`:extract` through the app, the two error messages and the existing-archive refusal. Not done: a
run against the compiled binary in a terminal, and archives from other tools (only ones made by
this module and hand-built hostile tars were extracted).

---

### Compress Form: a Modal Popup with a Validated `Request` (COA A)

**Date:** 2026-09-28
**Status:** Confirmed

#### Context / Background

`:compress <name>` could pack files but the only way to pick a format, a level or a name was to know
the command. The right-click menu should offer it, with the choices a person expects: name, format,
level, whether to make one archive per item, and whether to delete the originals. A password was
asked for too, and is deferred to its own cycle (see Consequences).

#### Options Considered

**Option A: a "Compress…" item that opens a form popup**
- Every choice in one place with inline validation, and the same shape serves a later "Extract…".
  The cost is a new popup with its own keyboard and mouse handling. Difficulty: high.

**Option B: a format submenu, each choice asking for a name in the prompt bar**
- Smallest change, reusing the prompt bar and `:compress`, but no level, no options and no
  password. Difficulty: small.

**Option C: a chain of prompts in the bottom bar**
- No new widget, but no way back to fix an earlier answer, and masking a password in the shared
  prompt is awkward. Difficulty: medium.

Chosen: **A**, the only one that covers the choices asked for; B remains a fine first slice if the
form proves too heavy.

#### Consequences

The popup follows `settings_popup`: `compress_popup` is pure state (no `Frame`, no filesystem),
`overlay_view` draws it and `main` is the only place that acts on it. Its output is a `Request`
whose `Layout` is either `One { file_name }` or `PerItem`, so a job with both a typed name and one
archive per item cannot be expressed, and a submitted file name is checked to be one plain path
component (a proptest types arbitrary characters and asserts that). The form never starts the job:
`App::start_compress` does, and returns an error string that the form shows in place, because only
the app can see that a name is taken or that an operation is running. Existing names are refused
rather than overwritten, the same rule `:compress` follows. On the Name row letters are typed, so
`j` and `k` are text there and only the arrows and `Tab` move focus; on the other rows `j`/`k`
also move, matching the other popups. Typing a known extension in the name switches the format
instead of producing `x.tar.gz.zip`. With one archive per item each is named `<item>.<ext>`
(`a.txt.zip`), which cannot collide the way `a.txt` and `a.pdf` both becoming `a.zip` would.
"Delete originals" trashes a job's sources only after that job's archive is complete, and a
failure stops the batch, so an original never loses its only copy; the trash is the desktop one, so
it is undoable outside the app. The form's rows are hit-tested by the same function that lays them
out (`compress_area`/`compress_inner`), so a click and the drawing cannot disagree. Level maps to
deflate levels 1, 6 and 9 for zip and gzip and is ignored, and greyed, for a plain tar.

The password is deferred because it is a change of a different kind: zip AES needs the `zip`
crate's `aes-crypto` feature (about seven more small crates), a masked field held in a
zeroize-on-drop buffer, and, to be usable, a password prompt on extraction. It is on the roadmap
with the plan, including that zip encryption does not hide file names.

**Verification.** `scripts/check` passes: format, clippy with `-D warnings` and the whole
workspace's tests are clean. `file_ops` went from 41 to 42 tests (levels: a higher level is no
larger and still round-trips, for zip and tar.gz) and `tui` from 424 to 449. The new ones cover the
form's keys, focus wrap, format and level cycling, name validation and the error clearing, the
per-item and delete-originals options, a proptest that a submitted name is always one safe
component, the menu entry and its marked count, drawing (every row, the extension after the name),
click hit-testing, and, through the app, one archive of two items, one per item, a taken name
refused without starting, nothing selected, and a second job refused while one runs. Two existing
tests changed because the file menu grew a row: the expected entry order gained "Compress…", and
the screen-corner menu test uses a 17-row screen instead of 14 so the whole, now taller, menu
still fits. Smoke-tested against the compiled binary in an isolated `tmux` session with a
throwaway `XDG_CONFIG_HOME`, sending SGR mouse sequences: a right-click showed "Compress…", a
left-click on it drew the form, editing the name and cycling the format to tar.gz then `Enter`
produced a `backup.tar.gz` that `tar` listed correctly, and the header said "compress complete".
Not exercised by hand: delete originals (it would fill the real trash; the option and the trash
call are only covered as far as the form's `Request`), the taken-name error in the terminal (unit
tested through the app), a click on a form row, and a folder or a marked set as the source.

### Extract Form: the Compress Form's Shape, a `Destination` Instead of a Name (COA C)

**Date:** 2026-09-28
**Status:** Confirmed

#### Context / Background

`:extract` unpacked archives but the right-click menu had no way to reach it. The user asked for
extraction "similar to how compression is done", which ruled out a bare menu item.

#### Options Considered

**Option A: a single "Extract" item running `:extract`**
- Tiny and safe, but no choice of destination or of what happens to files already there.
  Difficulty: small.

**Option B: an "Extract here / to…" submenu**
- Covers the common cases, but "to…" still needs a path prompt and there is no conflict choice.
  Difficulty: medium.

**Option C: a form like the compress form**
- Destination, conflict policy and delete-archive in one place with inline errors, consistent with
  compression. The cost is a second popup with its own keys, drawing and hit-testing. Difficulty:
  high.

Chosen: **C**, because it was what the user asked for.

#### Consequences

`extract_popup` mirrors `compress_popup`: pure state, drawn by `overlay_view`, acted on only by
`main`. Its `Request` carries a `Destination`, which is `Here`, `Folder(name)` or `PerArchive`, so a
typed name together with several archives cannot be expressed; the Folder row is greyed for several
archives. The default folder is the archive's stem, as `:extract` uses, so a tarball of loose files
does not litter the browsed directory; emptying the field is the deliberate way to extract here,
and it skips the name check that would otherwise reject an empty name. A typed name reuses
`compress_popup::check_name` (now `pub(crate)`), so a submitted folder is one plain path component;
a proptest types arbitrary characters and asserts that. The conflict choice maps onto the existing
`ConflictPolicy` (stop, skip, replace). With "stop", `App::start_extract` refuses a destination
folder that already exists before writing anything, reported inside the form, because otherwise
extraction would write the non-clashing files and only then fail on the first clash; "here" is
exempt since the browsed directory always exists. "Delete archives" trashes an archive only after
its extraction succeeds and a failure stops the batch, so an archive never loses its contents.
`spawn_extract` now takes a list of `ExtractJob { archive, dest }` plus a policy, and `:extract`
builds the same per-archive jobs it always did with "stop" and no deletion. The menu item is only
listed when the clicked entry is a supported archive (`Context::archive`), and the form takes the
archives among the marked entries, as `:extract` does.

**Verification.** `scripts/check` passes: format, clippy with `-D warnings` and the workspace's
tests are clean. `tui` went from 449 to 467 tests. The new ones cover the form's keys, focus wrap,
policy cycling, the delete toggle, the default folder, empty meaning here, several archives, name
validation, a proptest on submitted folders, drawing and click hit-testing, and, through the app,
the form opening only for archives, extracting into a named folder, extracting here and trashing
the archive, and "stop" refusing a folder that exists while "replace" fills it. Not exercised by
hand in the terminal: the right-click flow itself (the compiled binary was not driven through
`tmux` this time) and the trash call for "delete archives", which would fill the real trash.

---

### Mouse Stage 2: Marks by Click, a Range Key the Terminal Delivers, Breadcrumb Jumps (COA B)

**Date:** 2026-09-29
**Status:** Confirmed

#### Context / Background

The roadmap's next item was multi-select and breadcrumb clicks: `Ctrl`-click toggles a mark,
`Shift`-click marks a range, a click on a segment of the header's path jumps there, and a
middle-click opens the entry. The catch is `Shift`: most terminals keep `Shift`+mouse for their
own text selection and never report it to the application, so a range click bound to `Shift`
alone would silently do nothing in kitty, alacritty and others. This could not be tested here
against a real terminal.

#### Options Considered

**Option A: the roadmap as written, `Shift` for ranges**
- Matches desktop file managers, one cycle. Range select is dead wherever the terminal keeps
  `Shift`. Difficulty: medium.

**Option B: the same, with the range modifier configurable and `Alt` accepted by default**
- Works in every terminal, and `Alt`-click is free over the browser (the shell box claims `Alt`
  only over itself). One option beyond the roadmap. Difficulty: medium.

**Option C: two cycles, navigation first and modifiers later**
- Small first step, but the modifier plumbing is still needed and the item takes two loops.
  Difficulty: small, then medium.

Chosen: **B**, because it keeps A's scope and removes the failure that could not be checked.

#### Consequences

`mouse_range_modifier` in `config.toml` is `"either"` (the default: `Shift` or `Alt`), `"shift"` or
`"alt"`; an unrecognised value falls back to the default like every other field. `Select::of`
turns a click's modifiers into `Plain`, `Toggle` (`Ctrl`) or `Range`, with the range winning when
both are held since it is the more specific request. `BrowserState` gained a range anchor, kept as
a path rather than an index so a refresh cannot move it, and `toggle_mark_at`/`mark_range_to`. A
range keeps earlier marks and spans anchor to click in either direction; without a usable anchor
the cursor before the click stands in for it. A plain click sets the anchor, as in a desktop file
manager; the keyboard's `v` does not, to keep this change to the mouse path.

A modified click resets the `ClickTracker`. Without that, two quick `Ctrl`-clicks on a folder
paired into a double-click and opened it. The middle-click is a new `Click::Open` that never goes
through the tracker at all.

The header needed the most care. `hud::breadcrumb` returned only strings, and how wide the
breadcrumb is depends on how much room the right-hand pills leave it, which depends on live app
state. `breadcrumb` is now a thin wrapper over `crumbs`, which keeps each segment's directory (the
`…` that stands for dropped segments has none, so it is not clickable). The pills moved into
`header_pills`, and `crumb_rects` lays the segments out with the same widths and separators
`render_header` draws with; `main` builds the header's data through one `header_view` for both, so
a click cannot land on a different segment than the one drawn. A click on the last segment (where
the user already is) is ignored rather than resetting the cursor to the top.

**Verification.** `scripts/check` passes. New tests: a proptest that a range marks exactly the
span between anchor and click, plus earlier marks, in either direction; toggling twice restores
the marks; a stale anchor is ignored; the modifier table for all three settings; the tracker
reset; the middle-click opening a folder in place; crumb targets, and a proptest that every
clickable rectangle sits over the text drawn for it at any width. Driven against the real binary
in a PTY read through `pyte`: header clicks on `sub`, `proj` and `~` jumped there, a click on the
last segment did nothing; `Ctrl`-click marked, `Alt`-click and `Shift`-click marked the span,
`Ctrl`-click unmarked, two quick `Ctrl`-clicks left the marks unchanged without opening anything,
and a middle-click entered a folder. Two harness findings: a debug build needs about three seconds
before its first frame, and the first click after start is lost unless the harness answers every
query of `ratatui-image`'s probe (device attributes, window size, background colour and device
status), not just the first — the same gap already recorded in the Image Preview Concurrency
entry. Not tried in a real terminal, so whether `Shift`-click arrives there is still unchecked.

---

### Mouse Stage 3: Drag and Drop Through the Paste Path, With Feedback and Every Surface (COA C)

**Date:** 2026-09-29
**Status:** Confirmed

#### Context / Background

The roadmap asked for dragging a row or the marked set onto a folder to move it (copy with `Ctrl`),
reusing the `file_ops` paste and conflict flow, with drag events routed to the browser, a
highlighted target, and a rule for dropping on the parent column, blank space and the shell box.

#### Options Considered

**Option A: release-only drop**
- Smallest: press, drag, release over a folder. No feedback until release, so the user cannot
  tell whether the drop will land or whether it copies. Difficulty: small to medium.

**Option B: a state machine with visible feedback**
- A pure module, a label at the pointer, the target row lit, `Esc` to cancel. Leaves the shell
  and the edges out. Difficulty: medium.

**Option C: B plus every surface**
- Adds the left column (folder rows and "up a level"), the shell box and auto-scroll at the list's
  edges. Most of the scope is separate small pieces. Difficulty: high.

Chosen: **C**, on the user's pick.

#### Consequences

`browser_drag` is pure, like `browser_mouse`: `Drag` (`Idle`, `Pressed`, `Active`), `resolve` (what a
drop at a spot does), `landing` (what actually travels), `edge_scroll` and `ghost_text`. A press
becomes a drag after two cells of travel, so a click that wobbles by a cell stays a click; what the
pressed row carries is only worked out then. The label and the highlight in `draw` and the release
in `main` all call one `drop_target`, so what is lit is the drop that happens.

Decisions worth keeping. A drag only starts from a plain press: `Ctrl`- and range-clicks mark, so
`Ctrl` for copy is read at the drop, not the press. A drag carries the marked set only when the
pressed row is in it; an unmarked row carries itself and leaves the marks alone. A drop is refused
when any dragged folder contains the target, rather than carrying on without that one, and
entries already directly in the target are dropped from the batch (copying them would only raise a
conflict with themselves). Blank space in the left column means the folder above the browsed one;
that folder has no row, so its column's frame lights instead. The drop is the ordinary background
paste through the new `App::begin_drop`, which keeps the yank/cut clipboard out of it. That exposed
a small trap in `poll_bulk`: a finished move always emptied the clipboard, which would have
cleared one the user filled separately, so it now empties it only when it holds the cut that was
just pasted; a completed move also prunes marks on the files that moved. Dropping on a shell
pane writes the shell-quoted paths and a trailing space into the pane under the pointer and
focuses it, never a newline, so nothing runs; a path that is not valid text is skipped and
reported rather than sent mangled. Auto-scroll aims the selection at the row just outside the
view, because the list keeps the selection on screen and nothing else moves the view; it runs on
the loop's tick (50 ms while dragging, a row per 80 ms) because a pointer held still sends no
events.

**Verification.** `scripts/check` passes. New tests: the state machine (threshold, click release,
`Ctrl` at the drop, empty press, cancel), the target rules for both columns, blank space, the shell
box and a folder on itself, two proptests (a folder target is never a source or inside one and
nothing already in it travels; the scroll target is one row outside the view), the label text and
the label staying on screen, and three `App` tests (a dropped move leaves the clipboard alone, a
dropped copy keeps the original and a moved mark is forgotten, a conflict asks before
overwriting). Driven against the real binary in a PTY read through `pyte`: every gesture in the
ROADMAP entry, including a real shell receiving the quoted path and staying focused, and holding at
the top border scrolling 80 files back to a folder that began out of view. Harness findings: a
drag of fewer than two cells is correctly a click, which made three of my own first tests fail; and
the label drawn beside the pointer covers the row being aimed at when hovering the top border.
Not tried in a real terminal or with a real mouse.

---

### Open With: Associations Read In-Process (COA A)

**Date:** 2026-09-29
**Status:** Confirmed

#### Context / Background

`enter` on a file did nothing and the "Open with" list was only what was hand-written in
`config.toml`. The roadmap asked for choosing a program by extension or MIME type from config,
listing the programs actually installed, `enter` opening a file, and an "Other…" entry.

#### Options Considered

**Option A: in-process, no new crates**
- Parse the shared MIME database, `mimeapps.list`, `mimeinfo.cache` and the `.desktop` files,
  expand `Exec` ourselves. No process to build a menu, testable with fixture files, and
  `Terminal=true` programs can use the existing terminal handover. The cost is the specification's
  detail. Difficulty: high.

**Option B: delegate to `xdg-mime` and `gio`**
- Least code, the tools know every quirk. But a spawned process on every menu open, a dependency
  on `xdg-utils`/`gio`, platform differences, poor hermetic testing, and `Terminal=true` programs
  escape the handover. Difficulty: small to medium.

**Option C: in-process discovery, launch through `gio`**
- Real listing with less of the spec, but two launch paths and `gio` still required for the hard
  cases. Difficulty: medium.

Chosen: **A**, because it is the only one that lists what is installed, works in a terminal-only
session and stays testable without the user's desktop.

#### Consequences

Three modules. `mime_type` picks the highest-weight matching glob and breaks a tie by the longest
pattern (so `*.tar.gz` beats `*.gz`), honours case-sensitive globs, follows `subclasses` and
`aliases` (an app registered under `text/x-markdown` opens `text/markdown`), treats every `text/*`
as also plain text, and has a small built-in table for a machine without the database. A name that
matches nothing is sniffed: the first 512 bytes, text if valid UTF-8 without a NUL, allowing the cut
to fall inside a character. A directory is `inode/directory` and is never sniffed, or it would have
been offered every text editor. `desktop_entry` reads only the first group and turns `Exec` into
the command form `[[open_with]]` already uses; the file's place is `{}`, and a literal `{}` in an
`Exec` word is broken up with quotes so the launcher never mistakes it for the file. `associations`
merges lists in the specification's order, per type from the most specific to its parents: the
user's default, their added associations, then what is installed; a removed association is never
offered, and a program that is not installed is skipped (`TryExec`, else the first word of
`Exec`). `NoDisplay` programs stay out of the menu but can be the default.

A decision worth keeping: a rule's `match` accepts one string or a list, because a config parse
error costs the user the whole file, and `match = "md"` is the mistake anyone makes first. Another:
`enter`, `l` and `→` are all one action in the default keymap, so `l` on a file now opens it, as in
Ranger; the ROADMAP notes it as the thing a person used to browsing with `l` may find eager. The
menu's "Other…" is added in `main` past the end of the list rather than in `context_menu`, which
keeps that module's "empty list is shown but cannot be opened" behaviour and its test unchanged.
The associations load on first use, held in a `OnceCell` on `App`, so a session that never opens
a file never reads a megabyte of MIME data.

Writing the tests found a real bug. A desktop entry for a program under a path with a space or a
parenthesis produces a quoted first word, and `launch` looked for a program literally named
`'/opt/My App/bin/app'`, reporting "command not found". The old `program_of` split on whitespace and
never unquoted. `open::program_word` reads the first word as the shell does, with a property test
that any quoted path reads back as itself. Another fix from the same tests: the specification says
only double quotes group in `Exec`, so a test that used single quotes was wrong, not the parser.

**Verification.** `scripts/check` passes. New tests cover the glob precedence and the built-in table,
lineage with cycles, text sniffing, the parser and its locale choice, `Exec` splitting and every
field code, the command line launched under a real `sh` for hostile file names (a property test),
the association order, removal, parents, aliases, hidden and missing programs, desktop-specific
and user-over-system precedence, dashed IDs and an empty database, rule matching by extension and
MIME pattern, and through `App`: a rule beating the desktop's default, the default running with
the exact path, `Terminal=true` asking for the terminal, the fallback naming `xdg-open`, the menu's
order, and "Other…" running or cancelling. Driven against the real binary in a PTY read through
`pyte`, on a scratch desktop with its own globs, applications and cache: every behaviour in the
ROADMAP entry, including a real `Terminal=true` program taking the screen and returning to the
browser. Not tried against a full real desktop's application set.

---

### VFS Hardening: Complete the Trait, Move the Readers (COA A)

**Date:** 2026-09-30
**Status:** Confirmed

#### Context / Background

The browser and the core file operations already went through `Vfs`, but the readers did not:
preview, Inspect, disk usage, the marked-size walk and a few checks read `std::fs` directly, and the
trait had no way to read a file, describe one, or follow a link. SSH/SFTP and browsing inside an
archive both need exactly those, and Inspect's own docs said it could not work elsewhere because
"`Vfs` has no such fields".

#### Options Considered

**Option A: complete the trait and move the readers.** Add what is missing, migrate the bypassing
code, and test against a second backend.

**Option B: A, plus archives through `Vfs`.** What archive browsing needs, but the most invasive:
zip wants a seekable stream, and the hardened extraction code would be rewired.

**Option C: guardrails only.** A lint gate and the two primitives, migrating only preview and
inspect. Smallest, but leaves the abstraction incomplete and untested.

#### Decision

Option A. The design choices worth keeping:

*Optional metadata.* `Metadata` makes every field but kind and length optional instead of
inventing values. Inspect then leaves a row out, and disk usage measures by length when nothing is
allocated, rather than showing a made-up owner or a zero.

*`local_path` as the honest escape hatch.* Some things are local by nature: handing a file to
another program, reading an archive by seeking, a preview hook. Rather than pretend they are
portable, `Vfs::local_path` answers "is this file on this machine's disk, and where", and each such
feature skips the file when it is `None`. That makes the remaining direct uses of the disk a short,
explained list instead of scattered exceptions.

*`scan_dir` beside `list_dir`.* `list_dir` sorts and follows links, for display. A size scan wants
the opposite: each entry described as itself, unsorted, in one call (one round trip on a remote).
Keeping them separate kept the local scan exactly as cheap as before.

*A second backend is the proof.* `MemVfs` is in memory with real symlink resolution, a `sparse()`
mode, and the same error behaviour as the disk, and `conformance::check_backend` runs twelve
scenarios against both. I checked the suite has teeth by breaking three behaviours in `MemVfs`
(rename overwriting, scans following links, `symlink_metadata` following) and confirming each made
it fail. Each migrated feature also has a test on memory, including `App`'s background jobs, which
took the filesystem as an `Arc<dyn Vfs>` where they used to build a `LocalVfs` inside their thread.

*The gate.* `scripts/vfs-gate` greps non-test code for direct disk use and fails on any file not in
its allow-list, and on a listed file that no longer needs its entry, so the list cannot rot. It
skips comment lines and `#[cfg(test)]` items; a first version flagged a test-only `use` and a probe
of mine appended a violation after the test module, where it is (correctly) not looked at; both
were fixed in the probe or the script, not by loosening the gate.

Left out on purpose: archives still use `std::fs` and are on the allow-list with that reason; the
trait cannot yet write bytes, make a symlink or change permissions; nothing copies between two
backends. All are in the roadmap.

**Verification.** `scripts/check` passes, now including the gate. New tests: the conformance suite
on the local disk, on memory and on a sparse memory backend, with mutations confirmed caught;
`Send + Sync`, `local_path` and the local disk's metadata; preview text, bytes, images, symlinks
and the size cap on memory, an archive off the disk shown as bytes, and a hook skipped off the disk;
Inspect on memory (a file, links, a tally, the view), with the sparse backend leaving rows out and
another machine's uid staying a number; the marked-size total, disk usage rows and the view on
memory, and a sparse backend measured by length; and through `App`: delete, copy, move and undo on
memory, the compress form's checks, Inspect and disk usage. Not tried in a real terminal, and there
is no second real backend yet to try it against.

---

### Archives Remainder: UX and Robustness (COA A)

**Date:** 2026-09-30
**Status:** Confirmed

#### Context / Background

What was left of the archive work was a set of small pieces: no key for the forms, no folder to
save into, no warning as you type a taken name, `:compress` and `:extract` erroring where a paste
asks, no password for `:compress`, zip entries without times, and a zip's whole central directory
read into memory with no bound.

#### Options Considered

**Option A: these pieces, no new dependencies.** Finishes what users hit; each is small and
testable alone.

**Option B: A plus `xz`, `zstd`, `bzip2`, `7z`, `rar`.** Opens more archives, but the format matrix
(form rows, preview, tests per codec) and C-backed dependencies are a cycle of their own.

**Option C: A plus browsing an archive as a directory.** The biggest win, but it needs the VFS
hardening item first.

#### Decision

Option A. Three choices worth recording.

The live warning needed to know what is on disk, and the forms are deliberately pure (no `Frame`,
no filesystem). Rather than give them a filesystem handle, each form exposes `build()`, which
returns the request it would submit or why it cannot, and changes nothing; `App` runs the same
`compress_problem` / `extract_problem` check on that request after every key, and again when Enter
is pressed. One function answers both "warn now" and "refuse on submit", so they cannot disagree.

`:compress` and `:extract` reuse the conflict prompt instead of growing a new one. A fresh request
carries `Abort`, which here means "not asked yet"; the prompt's answer becomes the request's policy
and the request is continued, and an abort answer simply ends it. Extract's "overwrite" and "skip"
apply to the files inside the existing folder, which is what the extract form's Existing files row
already meant, so the prompt says "replace files / skip existing". Compress's skip cancels, since
there is only one archive, and originals are never trashed for a skipped archive. `-p` goes through
a two-step masked prompt after any conflict question, so a password is never typed on the command
line where it would show in the status line or history.

The central-directory bound is the preview's check made suitable for extraction: extraction must
still open zip64 archives, so a zip64 record is followed to the real counts instead of being turned
away. Every record in the last 64 KiB is checked, since a reader may skip a bogus one, which means a
hand-made comment holding a look-alike record can make an honest archive be refused; that cost is
accepted and logged. Modified times are written as UTC because the standard library has no time
zone database, and extraction does not restore them yet.

Three existing tests asserted the old behaviour (an existing destination was an error) and were
updated to the new one rather than weakened: each now checks the prompt opens and what each answer
does.

**Verification.** `scripts/check` passes. New tests cover the keys in the default map, the Save in
row (relative, absolute, typing `j` and `/`), `build` leaving no error behind, the live warning for
a taken name, a missing folder and a taken extract folder until skip or replace is chosen, saving
into another folder and refusing one that is not there, overwrite, skip and abort for `:compress`
and `:extract` (skip keeping the file already there), `-p` asking twice and making an encrypted zip,
a mismatch starting over, a tar name and Esc, the command parser for `-p`, a known instant becoming
the expected zip date, dates outside 1980-2107 left off, a zipped file keeping its time, a property
test that every day from 1970 to 2150 round-trips, and a zip claiming a huge directory, too many
entries, a hidden bogus record before an honest one, and a zip64 claim all being refused before
anything is written. Not tried in a real terminal.

---

### Encrypted Archives: Zip AES-256 (COA A)

**Date:** 2026-09-30
**Status:** Confirmed

#### Context / Background

The compress form had no password field, and an encrypted zip could not be opened at all. The
roadmap's plan was zip AES-256, a masked field on the form and a prompt on extract.

#### Options Considered

**Option A: zip AES-256 through the `zip` crate's `aes-crypto` feature.** Fits the existing forms
and format. Opens in 7-Zip, Keka and WinZip, not in Windows Explorer or old `unzip`. File names
stay visible. Adds a handful of small crates.

**Option B: 7z with AES-256 and header encryption.** Hides names and is a standard format, but
pulls in an LZMA codec and a far larger dependency, and needs its own read, list and preview paths.

**Option C: a standalone `age` encrypt/decrypt action.** Hides names and covers any file, but is a
separate feature beside the archive forms and its output opens only with `age` or this app.

#### Decision

Option A. The password is its own type, `archive::Password`, not a `String`: it is wiped when
dropped, prints as `Password(..)` so a stray `{:?}` of a request or a prompt cannot leak it, and
is capped at 256 characters in a buffer reserved up front, because a growing `String` leaves the
old, unwiped allocation behind each time it reallocates. The compress functions refuse an empty
password and a password with a tar format, so neither reaches the writer even if a caller forgets
to check.

The form does the user-facing checks: the confirmation must match, and a password with a tar
format, whether picked on the Format row or implied by a typed `.tar.gz` name, is an error in the
form. An archive made with a typo in its password is lost data, so a mismatch is caught before the
job starts rather than after.

Extraction asks in a prompt of its own and not in the extract form. The form is shown before the
archive is known to be encrypted, and `:extract` has no form at all; both go through one
`request_extract` that looks at the archives (`needs_password`) and opens the prompt only when one
is encrypted. The password is checked against every encrypted archive of the batch before anything
is written (`check_password` opens the first encrypted entry, where AES verifies the password
up front), so a wrong guess asks again instead of failing halfway through a tree.

Not solved, and logged in the roadmap: `:compress` cannot take a password, names stay visible by
the nature of zip, and a per-entry password assembled by another tool is reported at the entry
that fails.

**Verification.** `scripts/check` passes. New tests: a round trip with a folder and a nested file,
a password, no password and a wrong one telling `PasswordRequired` from `WrongPassword`, plain
archives needing none and ignoring one, tar and an empty password refused with nothing written, a
symlink surviving encryption, the password's redaction and cap, and a property test that any
printable password seals and opens arbitrary bytes and a different one never does. For the forms:
the dots, the mismatch and tar errors, a confirmed password reaching the request; through `App`:
an encrypted compress, the prompt opening before anything is written, a right password extracting,
a wrong one asking again with nothing written, `Esc` cancelling, an empty one not submitted, and a
plain archive never prompting. One assumption I checked and got wrong: deflate bit-packs short
text, so searching the archive's bytes for the plaintext proves nothing either way; the tests use
`needs_password` and the refusal to open. Not tried in a real terminal or with another tool's
reading of the archive.

---

### Undo History: Inverse-Operation Journal (COA A)

**Date:** 2026-09-30
**Status:** Confirmed

#### Context / Background

The roadmap asked for a stack of recent file operations that can be stepped back through. Delete to
the trash had shipped; this is the other half.

#### Options Considered

**Option A: inverse-operation journal.** Each finished operation pushes a record that knows its
inverse. Small and fast, but each operation site must record correctly and an overwrite or a
permanent delete cannot be put back.

**Option B: A, plus an app-owned holding area** for permanently deleted and overwritten data so
those become undoable. Covers everything, but permanent delete stops being permanent until purged
and needs a size cap.

**Option C: a history popup with undo for only the reversible entries.** Smallest, but a second UI
surface and partial undo.

#### Decision

Option A, with the keys `z` and `Z`. The journal lives in `file_ops::history`, not in the `tui`
crate: it is the inverse of what `file_ops` does, it needs only `Vfs` and the trash, and it can be
tested with no terminal. `tui` owns one `History`, records at the points where an operation
finishes, and commits a batch when the next operation starts or an undo is asked for, so the
batching needs no state at the ends of a long paste.

Three choices were made for safety rather than completeness. A created file is removed only while
it is still empty, so an undo can never throw away what was typed into it since. An undone copy
goes to the trash, not to `remove_file`, for the same reason. A step that fails leaves itself and
everything before it on the stack, and what did succeed becomes redoable, so the stacks always
describe the disk.

Trash needed one real addition. `trash::delete` returns nothing, and restoring needs the item. The
`trash` crate now takes a snapshot of the trash's ids before a batch and, after it, claims the new
item for each path by its original location (trying the parent with symlinks resolved too, since
the trash records it that way). Nothing found means not undoable, reported on the status line.

Undo runs on the render thread. A cross-device move undone is a copy, so a large one stalls the
interface until it is done; recorded in the roadmap as a follow-up rather than building a second
background pipeline in this cycle.

**Verification.** `scripts/check` passes. New tests: each change kind both ways (including a real
trash and restore), a written-to file refusing removal, a batch as one step, a failure partway, redo
being forgotten by a new entry, the capacity, and a property test that any sequence of creates and
moves round-trips through undo-all and redo-all; at the `App` level, `:mkdir` of two names, a
rename through the prompt, a dropped batch of moves, a trashed batch, and a permanent delete
reporting nothing to undo. Not tried in a real terminal.

---

### Default Key Remap: Marks-Only Operation Keys

**Date:** 2026-09-30
**Status:** Confirmed

#### Context / Background

The default keys were changed on request: `d` cuts, `x` sends to the trash, `X` deletes
permanently, `s` stops opening a shell, and `y` should act on marked entries only.

#### Decision

The key defaults moved in `RawKeyMap`; `shell` now defaults to an empty list, since `Alt+n`
already opens a shell. `y`, `d`, `x` and `X` are guarded by `App::require_marks`, which refuses with a status message
when nothing is marked. The `yank`, `cut` and delete methods keep their cursor fallback because
the right-click menu and several tests rely on it; only the keys changed.

**Verification.** `scripts/check` passes; the keymap tests pin the new defaults. Not tried in a
real terminal.

---

### Archives Through Vfs: Write Primitives and Seekable Streams

**Date:** 2026-10-01
**Status:** Confirmed

#### Context / Background

The VFS hardening cycle moved the readers onto `Vfs` but left archives on `std::fs`, because zip and
tar want seekable streams and the extraction code was hardened against writing through links. The
roadmap named this the largest piece left: remote extraction and browsing inside an archive both
need it.

#### Decision

The trait gained four methods rather than a general "write bytes" call, because each one carries a
guarantee archive code relies on. `create_write` is create-new and never follows a link, so an
entry cannot be written through one; its writer is seekable because a zip returns to patch an
entry's header. `create_symlink` stores the target as written. `set_mode` lets a backend with no
permissions answer `Unsupported`, which extraction treats as success. `replace` is what lets a
finished archive appear under its real name at once. Archive creation, extraction, the password
checks and the preview listing all take a `&dyn Vfs` now. Each new method has a conformance
scenario, so a future backend is held to the same guarantees as the local disk and memory.

**Gate.** Both archive files left `scripts/vfs-gate`'s allow-list, which had only been passing
because zip's own `entry.is_dir()` matched the gate's pattern. Those few lines now carry a
`// vfs-gate: archive entry` marker the gate skips, rather than a looser pattern that could miss a
real disk call. Copying between two different backends is still missing.

**Verification.** `scripts/check` passes. Not tried in a real terminal.

---

### Remote Browsing: The System `ssh`, Routed by Path Prefix (COA C)

**Date:** 2026-10-01
**Status:** Confirmed

#### Context / Background

The two `Vfs` cycles before this one existed to make a remote backend possible: every reader and
writer, archives included, now goes through the trait. `vfs_ssh` was an empty crate.

#### Options Considered

**Option A: `ssh2` (libssh2 bindings), synchronous.** Fits the synchronous trait, but pulls in a C
library and OpenSSL, serialises calls on one session, and leaves `~/.ssh/config`, `ProxyJump` and
host-key prompts to be rebuilt by hand.

**Option B: `russh` and `russh-sftp`, pure Rust.** No C, but the largest dependency tree, an
async-to-sync bridge under every call, and known-hosts checking to write ourselves, where a mistake
is a security hole.

**Option C: the system `ssh -s sftp`, speaking SFTP over its pipes.** OpenSSH keeps the hard parts.
The cost is needing the `ssh` program and having no password prompt.

#### Decision

Option C, at the user's direction after the three were compared. The design choices worth keeping:

*A remote place is an ordinary path.* `ssh://[user@]host[:port]/path` is carried as the `PathBuf`
the rest of the program already passes around, so the browser, marks, clipboard and file
operations needed no new types. `Remote::locate` is the only parser. Its pieces are newtypes built
by validators (`Host`, `User`, a non-zero port), a host or user cannot begin with `-`, and the
connection is always requested as an `ssh://` URI, so nothing a user types can become an `ssh`
option. Stepping up from a machine's root lands on the bare scheme, which names nothing and is
reported as not found.

*One `Vfs` routes.* `RoutedVfs` replaces `LocalVfs` as the application's filesystem. A path goes to
the disk or to the `Link` for its machine, and the `Link` returns the full `ssh://` paths itself, so
no caller translates. A copy between two places that are not on one machine is streamed through
`open_read` and `create_write`, and a move there reports `CrossesDevices`, which `file_ops::mv`
already answers by copying and deleting. That is why yank and paste work across them with no new
code in `file_ops`.

*The asynchronous client stays behind the synchronous trait.* The SFTP client lives on a small
runtime of its own, and a caller waits on a standard channel, so it works from the render thread,
a plain thread or inside another runtime. File streams are blocking `Read`/`Write`/`Seek`; a seek
from the end is turned into an absolute one because the client cannot do it, and a writer counts
its own length because the server will not describe a handle opened for writing.

*Connection model* (shared state across threads, so written down). A `Link` is in one of two
states, `Idle` (no session) or `Live(session)`. A call in `Idle` connects under the lock, so callers
arriving meanwhile wait for that one attempt instead of each making their own. A call that fails
with a transport error moves `Live` back to `Idle`, but only if the session it used is still the
current one, so a late failure cannot discard a newer session. A read is then repeated once, since
asking twice cannot do harm; anything that changes something is not, because the first attempt may
have happened. Invariants: at most one session per machine; a session is dropped once and by the
call that saw it fail; a server's refusal never drops the session. There is no model checker run on
this, only the tests below.

*Safe by default.* An unknown host key is refused (`StrictHostKeyChecking=yes`), the reason is shown
in the status line, and the user connects once with `ssh` to trust it. `BatchMode` means no password
prompt, only keys and the agent. The control socket is kept in the per-user runtime folder rather
than under `$HOME`, because a Unix socket path is limited to about 100 bytes and a long home folder
overflowed it, which a trial run showed. The control process closes ten seconds after the
application stops using it, because a trial showed that an application killed mid-run left one
running for good.

*Local-only actions are refused, not redirected.* A shell pane, a `:` command and opening with a
local program would act on this machine's files while the screen shows another's, so each says why
it will not run. The trash is refused too. The trash decision above recorded that a non-local
backend should fall back to a permanent delete; that was reconsidered, because a key that promises
a recoverable delete should not delete for good, so `x` refuses and `X` stays the permanent one.

**Two test-suite changes.** Both are on the `Seed` the backend supplies, not on an assertion. SFTP
v3 stores times in whole seconds, so the scenario that checks `touch` moved the time waited 20 ms
and could not pass; `time_resolution` lets a backend say how long to wait. A link's text is in the
store's own namespace, so a link seeded to point at `ssh://host/x/file` reads back `/x/file`;
`link_text` says what a backend reads back.

**Found while testing.** `BrowserState::leave` and `enter` set the current folder before listing
it, so stepping up from a remote root (whose parent is the bare scheme) left the browser pointing
at it. Both now keep their place on failure; local unreadable folders had the same flaw. `:cd`
used `is_dir`, which turned "cannot connect" into "not a directory"; it uses `metadata` now.

**Verification.** `scripts/check` passes. The remote backend passes the conformance suite against a
real `sshd` on a loopback port (started by the test, skipped where `sshd` is missing), along with
copying a tree both ways, a move between the disk and a machine, a refused unknown host, a trusted
one under strict checking, the home folder, browsing to and stopping at a machine's root, and a
writer that is only dropped. The real binary was driven in a terminal against that `sshd` with
strict host keys: a remote `:cd`, a preview, and a paste to the local disk worked. Not tried: a
real network, a password or passphrase login, `ProxyJump`, an agent, Windows, or a server other
than OpenSSH.

---

### Browsing Archives: A Read-Only `Vfs` Addressed by a `!` Suffix (COA A)

**Date:** 2026-10-02
**Status:** Confirmed

#### Context / Background

Archives were already read and written through `Vfs`, and the remote backend had just shown that a
second backend works with every feature for free. Opening an archive as a folder was the remaining
payoff of that work.

#### Options Considered

**Option A: a `Vfs` that wraps another and serves archive paths.** Every feature that goes through
the trait works inside an archive unchanged, and it nests over a remote archive. The cost is an
index of the archive and a new crate.

**Option B: extract to a temporary folder and browse that.** Small, but it extracts everything up
front, goes stale, needs a place on disk to put it, and undoes the work that stopped archives from
touching the disk.

**Option C: the same wrapper, zip only.** Least risk, but a feature where some archives open and
others do not.

#### Decision

Option A, all three formats, read-only.

*The address.* An archive's root is its path plus `!`, and a file in it `/x/a.zip!/dir/f`. The path
is split on its text, never rebuilt from components, because a component walk collapses the `//` of
`ssh://host/...` and would change which machine the archive is on. Only a `!` that follows an
archive extension and is followed by `/` or the end counts, so a folder named `notes!` is untouched.
The spelling (`mount_root`, `archive_of_root`) lives in `shared`, because the browser needs it to
put the cursor back on the archive when stepping out and may not depend on the backend.

*The wrapper sits above the router.* `ArchiveVfs` wraps `RoutedVfs`, not the other way round, so
the archive file is read through whatever backend holds it. Paths that are not in an archive are
passed through untouched.

*An index, bounded, never truncated.* Listing a folder must not touch data, so each archive is read
once into a tree (a zip from its central directory, a plain tar by seeking over its data, a
`.tar.gz` by inflating it once) and kept for eight archives, reused while the file's size and time
match. Every limit is an error rather than a short listing, because a folder that looks complete
and is not misleads more than one that says it is too big. The zip central-directory guard and the
capped reader are copies of the ones in `preview`; feature crates do not depend on each other, and
a shared home for them is not worth a crate yet.

*Opening a file.* A plain tar's entry is read in place through a window onto the file. A zip or
gzip entry is unpacked into memory, capped at 256 MiB, because the caller needs `Seek` and a
compressed stream cannot. A temporary file would have been the alternative and is exactly what the
vfs-gate forbids.

*Read-only, and local-only things refused.* Every way of changing an archive answers
`Unsupported`, the same answer a backend gives for a field it lacks. A move out of an archive is
refused rather than reported as crossing a device, which would have copied the file and then failed
to delete the original. A shell, a `:` command, the trash and opening with a program are refused
inside an archive for the reason they are refused on a remote machine: the folder is not on this
disk.

*Links.* A tar link is followed only inside the archive: an absolute target, a `..` that climbs out
and a loop longer than 16 hops all lead nowhere. A zip link is shown as the file holding its text,
since reading it to find the target would mean inflating data while indexing.

**Found while testing.** The tar writer refuses a `..` in a name, which is the very thing a hostile
archive carries, so the test that checks such entries are dropped writes the name into the header's
bytes directly. A first clippy pass also pointed out that the format-by-extension check was a
table in disguise.

**Verification.** `scripts/check` passes. Property tests cover walking random trees in all three
formats, a window onto a buffer, and the path parser; unit tests cover links, hostile names,
damaged archives, read-only refusals, copying out, and an archive that changes on disk. A 50,000
entry zip indexes cold in about 90 ms. The real binary was driven in a PTY through `pyte`: entering
a zip and a tar.gz, a preview, stepping out with the cursor on the archive, copying a file and a
folder out to disk and checking the bytes, and every write, a shell and `:ls` refused with the
archive unchanged. Not tried: a real kitty, an archive on a real remote machine, or an archive
bigger than a few hundred bytes in the terminal.

---

### Filmstrip View: A `V` View Cycle and a Cache of Finished Thumbnails (COA B)

**Date:** 2026-10-02
**Status:** Confirmed

#### Context / Background

The browser had one way to show a folder, a list of names with one preview beside it. The request
was a second view, switched by `V`, that shows images already enlarged while moving through them,
and a question whether it would make the binary much larger.

#### Options Considered

**Option A: a grid of thumbnails only.** Nearest to what a gallery is, but it never shows a picture
large, which was the point of the request.

**Option B: a view cycle, with a filmstrip first and the grid after.** The filmstrip is the large
picture with a strip of neighbours; the grid reuses its cache. More states to design than A or C.

**Option C: a bigger preview only.** Smallest to build and nearly free, but it is a larger preview
and not a different view of the folder.

#### Decision

Option B, in two stages. This one ships the cycle and the filmstrip; the grid is queued under
Medium Priority.

#### Consequences / Implementation Notes

*The big picture is the existing preview.* The filmstrip does not draw its own big view. `draw`
gives the preview pane the filmstrip's `big` rectangle and the two list columns the zero-width
`Rect` the two-pane layout already uses for a removed column, so text, hex, archive listings and
folders work there with no code of their own. The one difference is that an image is scaled up:
`ratatui-image`'s default `Fit` only shrinks, which left a 60×40 picture at its native few cells in
the same window.

*Thumbnails are finished before the render loop sees them.* `ImagePreview` uses `ThreadProtocol`
because it shows one picture whose area changes. A strip needs a dozen of one fixed size, so each
is decoded and encoded for its cell size on the blocking pool with `Picker::new_protocol` and
arrives as a `Protocol` that the stateless `Image` widget draws. That is simpler than a dozen
`ThreadProtocol` channels and leaves nothing for the render thread to do. The cache is keyed by
path alone because the size is the same for every slot, and it is emptied when the size changes;
a result that arrives for another size is dropped.

*Bounded.* 64 entries, oldest requested evicted first, and the whole cache cleared on leaving the
view, so a folder of photos is not held in memory behind a list. Requests go nearest the cursor
first and only for the window on screen.

*Geometry is pure.* `FilmstripLayout` follows `browser_mouse`: the rectangles drawn and the ones a
click is tested against come from one function. The strip always has an odd number of slots so the
cursor can sit in the middle, and the window slides to the ends of the folder rather than leaving
empty slots there.

*Mouse.* The list view's mouse code is gated to the list view and the filmstrip has its own small
branch (click a thumbnail, wheel over the strip or the big preview), rather than teaching the
list's hit-testing about a layout it does not have. Right-click, drag and drop and range clicks do
nothing in the filmstrip for now.

*Size.* Built from the committed `HEAD` and from the working tree with the feature, the release
binary went from 13,511,080 to 13,596,648 bytes, 85,568 more (+0.63%), all of it the crate's own
code. The first comparison, against a binary built two weeks earlier, showed 3 MB and was wrong
for this purpose: it included every commit since.

**Found while testing.** The first PTY run showed no pictures at all, in the filmstrip and in the
plain list, so it was the harness: the test shell inherited the real kitty's environment variables
and the app chose the Kitty protocol, whose escapes the harness strips for `pyte`. A second run
with those variables removed still showed nothing, because the startup probe's last query is a
device-status request and the harness had answered only the first of its five queries; the app
waited out the rest and never learned the cell size. Answering all of them (primary attributes,
cell size, background color, status) made the pictures appear. Worth keeping next to the earlier
notes on this probe.

**Verification.** `scripts/check` passes. Property tests cover the layout for any terminal size,
the window and the click mapping; unit tests cover the cache. The real binary was driven in a PTY
through `pyte`, reading per-cell colors: the enlarged image, four colored thumbnails, the
highlighted name, `j j`, a click on a thumbnail, the wheel, a text file with its placeholder, `V`
back to the list, and a 60×9 terminal. Not tried: a real kitty, photographs rather than 60×40
test images, or a remote folder.

---

### Grid and Details Views: Tiles That Answer the Mouse Like Rows (COA A)

**Date:** 2026-10-02
**Status:** Confirmed

#### Context / Background

The filmstrip shipped and was reported as bugged, with a messy lower part, and the wish behind it
was a view closer to a traditional graphical file manager. Reproducing it at 80×24, 100×30 and
200×50 showed what was wrong: a strip frame up to nine rows tall holding pictures one row high, with
the names far below, no outline on the selected tile, and small images left at their native size.

#### Options Considered

**Option A: a view cycle of columns, grid and details, with the filmstrip removed.** The cleanest
set. It was offered with the filmstrip going away.

**Option B: the same, with a toggleable preview side pane on the grid.** Keeps enlarged images but
adds layout states in exactly the area that had just come out messy.

**Option C: keep the filmstrip as is and add the new views after it.** Nothing thrown away, but
the old strip stays as it was.

#### Decision

Option A's views with the filmstrip kept, which was the answer given, and a flat table rather than
columns for details. The filmstrip's strip was rebuilt from the same framed tile as the grid, which
removes what looked messy there instead of leaving it beside two new views.

#### Consequences / Implementation Notes

*A tile is a row, to the mouse.* The list's click handling is about 200 lines of selecting,
marking, double-click opening, right-click menus and drag presses, all keyed on a `Hit`. The grid
reports `Hit::CurrentRow(index)` for a tile and `Hit::Blank` for the space between, so every one of
those paths works on tiles with no copy. Two things are deliberately different: the wheel moves a
row of tiles and not three entries, and a press never becomes a drag, because `drop_target` only
knows rows.

*Details is the list pane, widened.* `browser_layout` returns the usual split everywhere except
details, where the parent and preview get zero-width rectangles and the current pane takes the whole
body minus one title row. Because drawing, hit-testing, edge scroll and drop targets all ask that one
function, the list code needed no change to work full width; only the row builder grew two columns
(`hud::Columns` gained `modified` and `mode`). The title row sits outside the list's frame so the
rows' hit-test offsets stay exact.

*Keys.* In the grid, whatever is bound to move down and up goes a whole row, and the left and right
arrows go one tile. Those two arrows are `Leave` and `Enter` everywhere else, so they are fixed here
rather than bindable, while `h`, `l` and `enter` keep going up and in. Binding sideways movement to
`h`/`l` as well would have cost the vim-style way up a folder.

*Scrolling.* `GridLayout::follow` moves the first row as little as it can, the same rule the disk
usage view uses, and is clamped so the last screen never shows half a body of blanks.

*The shared tile.* `tile_size` fixes a tile from its picture height (three columns per row, plus a
frame and a name row) and one painter, `tile_view::render_tile`, draws it in both the grid and the
strip. Thumbnails are now made with `Resize::Scale` so a small image fills its tile; the cost is
that a tiny icon is enlarged and soft, which is what a graphical file manager does too.

*Size.* 17,240 bytes more than the committed filmstrip (13,596,648 to 13,613,888, +0.13%).

**Found while testing.** A property test failed on a 0×0 tile because two empty rectangles compare
unequal when one sits at the origin and the other does not; that was a wrong assertion, not a
drawing fault, and it now says "empty, or inside". The double-click check also failed once, for
a plain reason: it read row 1 of the screen for the header, which is row 0. And `themed_block` always
wrapped its title in blanks, so an unnamed tile frame showed `╭  ───`; the title is now added only
when there is one.

**Verification.** `scripts/check` passes. Property tests cover the grid geometry, scrolling,
cursor movement and click mapping, the tile's parts, and the details plan and header. The real
binary was driven in a PTY through `pyte` with the terminal probe answered in full: the grid
(tiles across and down, a filled picture, both arrow directions, `j`/`k`, scrolling, the wheel, a
click, a double-click into a folder and `h` back, an empty folder), details (title row, date, mode,
size, full width, `j`, a click), the filmstrip's framed tiles, and back to the columns. Not tried: a
real kitty, photographs rather than test images, or a remote folder.

### Trash Browser, and Marks as the Only Selection (COA B)

**Date:** 2026-10-04 · **Author:** deltaog-117

**Context.** Two requests in one cycle: a way to see and manage the desktop trash from inside the
app, and a rule that choosing entries belongs to `v` alone, with `y` (and the other operations)
responsible only for what they do.

| COA | Approach | Verdict |
|---|---|---|
| A | Browse the trash as a virtual folder in the normal browser | Rejected: the `Vfs` is built for real paths, and every key would need a special meaning there |
| B | A popup view like the disk usage view, opened with `T` | Chosen |
| C | Shell out to `trash-cli` | Rejected: an external tool most people do not have |

**The marks rule.** The keys already refused to act with nothing marked ("mark with v first"), so
the first step was to reproduce what could still behave like selection. Two things did: the
right-click menu and the `:trash`, `:compress` and `:extract` commands still fell back to the cursor
row, and a yank left its marks in place, so the next `x` would trash what the `y` had just taken.
The request was answered with a free-text clarification that read as wanting both fixed, with the
stress on selecting being `v` alone and `y` doing only the yanking, so both were done: every
operation reads the marks alone, and each spends the marks it takes (yank and cut as they fill the
clipboard, a confirmed trash or delete, a compress or extract once it starts). Declining a question
keeps the marks. Two things were left on purpose and are named in the roadmap: a plugin key still
gets the marks or else the cursor row (that is what the plugin API says it gets), and dragging an
unmarked row still drags it (a pointer gesture, not an operation on a selection). If either is
meant to change too, it is a small edit.

**The trash crate.** It already listed the trash for undo. `TrashEntry` is plain data (id, name,
original folder, when, size) so a view can sort and test it; restoring and purging look the entries up
by id in one listing per batch and return one result per entry, so a restore onto a place that is
taken fails for that entry alone and the rest go through. On a platform whose trash cannot be read
(macOS) every function says so.

**The view.** State, keys and drawing are in `trash_view`, with the trash behind a `TrashBackend`
trait so the tests run against a fake that can simulate a clash and an unreadable trash. Reading and
acting run on the blocking pool, and the keys are ignored while an action runs. The destructive steps
ask `(y/N)` and any key but `y` declines, including `Enter` and `Esc`. The view takes `v` for marks
for the same reason everything else does, and has no mark-all key: `E` is the one operation that
takes everything, and it asks first.

**A mistake worth recording.** The first run of the real binary showed 109 items in the trash view
although it was given an empty home. The crate also lists the `.Trash-<uid>` folder of any mount, and
`/tmp` is its own mount, where earlier unit tests had trashed temporary files; the run's `E` then
deleted those. It was test leftovers only, and the real home trash was checked afterwards and was
untouched (54 items before and after), but it showed that an isolated `HOME` is not enough to
isolate the trash. One of the new app tests had also been sending a file to the real trash each
time it ran; it now uses the permanent delete for the confirming half.

**Preview (same day).** The trash browser then got a preview of the selected item. The browser's
preview code (an image or the text pipeline, a folder's children, or the file's name) was moved
unchanged into `draw_preview_pane`, so the trash browser reuses it instead of copying it, and the
main loop feeds `Previews::update` the trash cursor's item while the view is open and the
browser's selection otherwise. What the pipelines need is a readable path, so the `trash` crate
reports where each item's contents sit (`stored_path`: `<trash>/files/<name>` for the record at
`<trash>/info/<name>.trashinfo`), which also keeps the file's extension, so type detection works.
The view splits into a framed list and a preview pane at 100 columns; below that it keeps the
single frame. The pane is a separate request from the first version, so the earlier verification
below does not cover it; see the end of this entry.

**Verification.** `scripts/check` passes. Tests cover the marks-only rule for every operation, the
marks being spent by each, declining keeping them, the trash crate against the real trash with
uniquely named files (listing, restore, purge, a clash, a missing entry, a folder's entry count),
and the view against the fake (listing, a read failure, moving, `v` as the only way to mark, nothing
acting on the cursor row, restore, a partial restore, the confirmation, `E`, the keys while working,
the column plan, the scroll, a click, and the drawing at several sizes). The real binary was driven in
a terminal emulator with its own home and trash: `y`, `d` and `x` refused with nothing marked, `v`
then `y` copied and cleared the mark, an immediate `x` had nothing to act on, `v` then `x` then `y`
trashed only the marked file; and in the trash view `r` and `x` refused with nothing marked, `v` then
`r` put the file back, `Enter` declined a delete, and `x` then `y` deleted it for good. Not tried: a
very large trash, the trash on a second disk beyond what `/tmp` showed, or Windows.

**Verification of the preview.** Tests cover the layout (no pane below 100 columns, a pane beside
the list above it, the two never overlapping and filling the screen for any size), the preview
subject (the stored path, whether it is a folder, none when the trash does not say where the
contents are, following the cursor), the stored path worked out from the record's path (including
names with spaces, another disk's `.Trash-<uid>` and layouts that are not freedesktop), and the
list drawn in its own frame beside the pane. The real binary was driven in a terminal emulator with
its own home and trash, four items trashed through the app (a text file, a PNG, a 1 KiB binary and
a folder of two files): the pane showed the text's lines, a drawn picture (as coloured half blocks,
since that terminal answered no graphics query), the binary's hex dump and the folder's two names,
and `J` scrolled the text by half a page. Not tried: a real kitty with its graphics protocol, a
very large text, or a trashed archive.

---

### Instant Start: Splash Off by Default, Keys Pass Through

**Date:** 2026-10-04 · **Author:** deltaog-117

**Context.** "It opens a little slowly, less than Ranger, but still some time." Nothing was measured,
so the first step was to measure.

**Measurement.** A terminal emulator driven through a pseudo-terminal, answering the startup
queries as kitty does (font size, graphics support, colors, the status request), timed from the
process start. The first frame is drawn after about 14 ms. The 900 ms boot splash was the delay, and
because its first key press only dismissed it, a key typed at launch was lost as well. A first
measurement showed 2.3 s because my fake terminal left one query (the status request that ends the
graphics probe) unanswered, so the library waited out its 2 s timeout; that is a fault of the
measurement, but it also shows what a terminal that ignores the queries costs.

**Choice.** The user chose to turn the splash off by default and draw the browser at once. The
browser was already drawn underneath the splash, since it is an overlay, so "at once" is what the
default now gives; with the splash switched on, a key press goes on to be handled after it
dismisses the splash, and a click only dismisses it because it would land on a row hidden behind
the box.

**Not done.** Cutting the 2 s waits for a terminal that never answers (the keyboard query and the
graphics probe), offered as COA C and not chosen. It is in the roadmap as the next place to look.

**Verification.** `scripts/check` passes, with the default and the shipped example both read as off.
The real binary was driven through the same emulator: with no config the first key press changed the
view at once, with `boot_splash = true` the splash was on screen at 0.3 s with the file list drawn
around it, a key press was both dismissing it and handled (the view changed), and a click dismissed
it without moving the cursor. Not tried: inside tmux, or a real kitty.

---

### Fuzzy Search: A Scorer of Our Own, Nearest Level First (COA A)

**Date:** 2026-10-04 · **Author:** deltaog-117

**Context.** `/` found the first entry whose name contained the text. With letters allowed to be
apart, "first" stops being an answer: `ae` matches nearly everything, so matches have to be ranked.

| COA | Approach | Verdict |
|---|---|---|
| A | Own scorer; finish the nearest level that has a match and answer with its best, as a short list | Chosen |
| B | Own scorer, ranked over the whole walk with a depth penalty (like `fzf`) | Rejected: always scans up to the 200,000-entry cap, so answers are slower and move while typing |
| C | A matcher crate (`nucleo-matcher`) with A's walk | Rejected: a new dependency that is not available offline, for scoring that is about a hundred lines |

**Scoring.** A name's score is the best placement of the typed letters over it, found by dynamic
programming: each letter earns a base, a bonus for sitting next to the previous letter, a bonus at
the start of the name or of a word (after a separator, or a capital after a lowercase letter) and a
small one for matching case; skipped characters and a late first letter cost a little. A name that
holds the whole text as one piece gets a bonus larger than anything the placement can earn, so a
real substring always outranks a scattered match however well that one is placed; that is what keeps
`aer` finding `aerend` before `a-e-r`. Among equal scores the shorter name wins, then the listing's
order. The row of the table is filled with a running best of the gap-discounted earlier scores
instead of looking back at every earlier column, which makes a name cost letters times characters.
My first version of that recurrence had a slip (`previous[j - 2] - GAP * 0`, which dropped the
gap cost of the nearest skipped letter); it was found on re-reading the derivation, before any test
ran, and the ordering tests then passed with the corrected line.

**Speed.** Most names in a big folder do not match, so the check that the letters appear in order
runs first and does not allocate, with an ASCII fast path for lowercasing; that took a name that
fails from about 0.7 µs to 0.1 µs in a release build. A full walk of 200,000 entries that all match
takes about 130 ms. A debug build is ten times slower, which only the tests see.

**Walk.** The queue is breadth first, so levels arrive in order; when the first directory of the
next level is reached and the last level had a hit, the answer is that level's best 20. The entry
limit stops a walk partway through a level that already has hits with those hits, rather than
saying nothing found. The first level is the folder on screen, so `rank_entries` ranks the loaded
listing and the answer shows at once with no walk.

**Stepping.** The app keeps the kept matches and a position; `Down`/`Ctrl-N` and `Up`/`Ctrl-P` move
through them, wrapping, and a match in another folder is reached the way a search hit already was.
`n` cannot step after `Enter` because it is Create. The status note reads `2/14`, with a `+` when
more were found than kept, and says nothing for a single match.

**Verification.** `scripts/check` passes. Tests cover the scorer (order, adjacency, word starts,
case, whole piece beating scattered, multibyte names), the walk (the best of a level winning over
the first met, the kept limit, the entry limit keeping what was found, nearest depth by property
test, matches best first by property test), and the prompt (finding by subsequence here and below,
the best in a folder winning over an earlier one, stepping and wrapping with arrows and with
`Ctrl`, nothing to step on a single match, stepping across folders with `Esc` still going home,
retyping after stepping). The real binary was driven in a terminal emulator: `/aernd` selected
`aerend.txt`, `/aerend` chose it over a scattered name listed first, `/report` stepped 1/3, 2/3,
3/3 and wrapped with the arrows and with `Ctrl-N`/`Ctrl-P`, `Enter` kept the cursor, and `/qtrbdg`
found `deep/er/quarterly-budget.md`, with `Esc` returning to where it began. Not tried: a remote
folder, a very large tree, or names in scripts without case.

---

### Sorting, Local Time, Grid Icons, Page Keys and Prompt Editing

**Date:** 2026-10-04 · **Author:** deltaog-117

**Context.** Five small gaps that each showed up in daily use: the details table could not be
sorted, times were in UTC, folder tiles in the grid were text only, there were no page keys, and a
typo in the middle of a name in a prompt meant deleting everything after it.

| Choice | Options | Verdict |
|---|---|---|
| Local time crate | `chrono` (already built for the trash crate) / `jiff` / a direct `localtime_r` | `chrono` (A): nothing new to compile, cross-platform, handles DST and `TZ`; `jiff` adds a tree nothing else uses; `localtime_r` needs `unsafe` and is Unix only |
| Prompt editing | a `LineEdit` type shared by the prompts / a cursor field in each prompt / only Ctrl-W and Ctrl-U at the end | `LineEdit` (A): one tested editor instead of the logic five times, or less than was asked |
| Where to sort | when a listing is stored / when it is drawn / in the `Vfs` | when stored: the list the cursor, `/` search and the mouse index is the list on screen, which is why hidden files are already filtered there; sorting at draw time would make every index disagree |

**Sorting.** `browser::sort` holds `SortKey` and `Sort` (a key and a flag). Each key has a natural
direction: name and permissions run from the start, size and time from the biggest and newest,
because that is what is looked for. `Sort::descending` is that direction flipped by the flag, so the
arrow in the header and the comparison use one definition. Folders always come first; a folder's
size means nothing, so folders order by name under the size key; equal keys fall back to the name so
nothing shuffles between refreshes. Both listings are sorted, and `set_sort` re-orders them in place
without reading anything and keeps the cursor on the same entry by path. The header's title is the
click target: `hud::header_key_at` mirrors the layout of `details_header` so the drawing and the
hit test cannot disagree. The keys had to be `,` and `<`: `o` and `s`, the obvious picks, are named
in an existing test as keys deliberately left unbound, and that test is a decision, not a fault.

**Local time.** `inspect::format_in` takes the zone, so the tests use fixed offsets and are the
same on every machine; `format_time` passes the machine's. The hand-written civil-date code was
removed with the UTC formatting. Inspect now writes the offset so a time is unambiguous.

**Icons.** The placeholder rows are a pure function, so the icon and its text are centred as a pair
and a single row gives the icon the space. Tested against a rendered tile for both glyph sets.

**Page keys.** `jump_target` and `page_size` are pure, in `view_mode`. A page is what the view shows:
the list's rows, the grid's tiles, the strip's thumbnails. They are fixed keys, like the disk usage
view's.

**Prompt editing.** `LineEdit` is text and a byte cursor kept on a character boundary, with a
property test that no key sequence can break that. It dereferences to `str`, so the existing reads
of a prompt's buffer kept working. The prompt handler gained `handle_prompt_event`, which takes the
whole key event because shortcuts need the modifiers; `handle_prompt_key` remains for the tests.
`Ctrl`+`Alt` together is how some layouts type AltGr characters, so it still inserts text. The
cursor is drawn inside the line by `status_line_with`, replacing the glyph that used to be appended
to the end. The password prompts were left alone.

**Found while testing.** The first key press of a session dismisses the splash and never reaches
the app, which made several rounds of test output confusing before it was understood. A terminal driver
that reads the byte stream instead of rendering a screen also misleads, because the interface only
redraws changed cells; the final checks use a terminal emulator (`pyte`).

**Verification.** `scripts/check` passes, with tests for every sort order and direction, the
cursor surviving a re-sort, the order surviving a reload, a move and a background listing, the
header arrows and the click hit test, local time in several zones, the icon rows and a rendered
tile, the jump targets and page size, and the line editor (including a property test). The real
binary was driven in a terminal emulator: `,` cycled name, size, modified, mode and back with the
right messages, `<` reversed, clicking Size, Size again and Modified changed the header arrow,
the time column read `07:13` under `TZ=Asia/Tokyo` for a file shown as `22:13` in UTC, `PageDown`
moved 26 rows in a 30-row terminal and `End`/`Home` landed on the last and first entries, and a
rename typo was fixed with five Left presses and two Backspaces (`reprot.txt` became `report.txt`
on disk). Not tried: the grid icons in a real Nerd Font terminal, the filmstrip's page key, or
`Alt`-based shortcuts in a terminal that sends Alt differently.

---

### Background Folder Moves and a Remembered View (COA A)

**Date:** 2026-10-04 · **Author:** deltaog-117

**Context.** Every folder change listed the folder on the render thread and waited, including the
SSH connect (up to ten seconds) and the indexing of a large archive, so the interface froze. The
view chosen with `V` was also lost on every launch.

| COA | Approach | Verdict |
|---|---|---|
| A | Make the move itself asynchronous; the old folder stays on screen until the new one arrives | Chosen |
| B | Also show the target at once as an empty "loading" listing | Rejected: cursor, marks and preview would point at a folder with no entries |
| C | Run the whole browser on a worker thread | Rejected: rewrites every read of the listing |

**Design.** A move is split in three: `plan_*` (instant, from the current state), `MovePlan::load`
(all the filesystem work, touching no browser state, so it can run on the blocking pool) and
`apply_move` (one swap). The synchronous `enter`, `leave` and `goto` now run that same path, which
removed a second copy of the logic and fixed two old faults on the way: a failed step up returned
its error out of the event loop and quit the program, and `goto` set the new folder before listing
it, so an unreadable one left the browser stranded there. A result is dropped if the browser is no
longer in the folder it was planned from. `App::begin_move` waits up to 40 ms for the result before
going busy. Without that grace every move would be asynchronous, the app would be busy for a tick
or two after each one, and keys pressed in that gap (`l` then `j`) would be dropped; with it a
local folder is already open when the call returns, and only a slow one takes the background route.
While one runs, `is_busy` holds other keys back, as it already did for copies and commands, and
`Esc` drops the job. The thread cannot be interrupted, so it finishes its read and the result is
discarded.

The view is saved in `local.toml` on every `V`, and `view` in `config.toml` gives the first one.
`persist_local` has nine call sites that rebuild the whole file from live state and none of them
knows about the view, so `save_local` keeps the stored `view` when the caller leaves it empty
instead of threading a tenth argument through all of them.

**Trade-offs.** Typing during a slow load is ignored rather than queued, so a held-down key cannot
pile up moves; the cost is that keys pressed meanwhile are lost. The live refresh, previews and the
search walk were already off the render thread and are unchanged.

**Verification.** `scripts/check` passes. Tests cover planning and applying separately, a stale
move being dropped, a failed load leaving the browser alone, the cursor landing on the folder left
(and on an archive's own row), a second move being ignored while one runs, `Esc` dropping one, a
local folder being open before `begin_move` returns, and the view's round trip through
`local.toml` and its precedence over `config.toml`. The real binary was driven in a terminal
(with the capability queries answered): `V` wrote `view = "details"` to `local.toml`, and a second
launch started in details. Not tried: a real slow connection (the slow path is covered by tests that
disable the grace period, not by a remote machine), or the filmstrip and grid as starting views.

---

### Default Theme Source: The Terminal's Own Palette (COA B)

**Date:** 2026-10-04 · **Author:** deltaog-117

**Context.** The default theme should fit the user's wallpaper-driven setup, and it has to work for
anyone, not only people on one compositor or one theming tool.

| COA | Approach | Verdict |
|---|---|---|
| A | A static palette built from the current wallpaper colors | Rejected: goes stale when the wallpaper changes |
| B | Build the theme from the terminal's live colors (OSC 4/10/11) | Chosen |
| C | A user palette file written by a hook | Rejected: manual wiring per user |

Reading pywal's cache or a compositor's border file was also considered and dropped, since each only
works for people who use that exact tool. Every theming tool ends up writing into the terminal, so
asking the terminal is the one source they all share.

**Design.** `theming::palette` holds the pure mapping (`Theme::from_palette`): no I/O, so it is
tested on plain numbers. Backgrounds stay `reset` so a translucent terminal shows through; the
selection row alone is a solid tint between background and foreground. The accent is the most
colorful of ANSI green, yellow, blue, magenta and cyan (red means danger). Each text color is moved
toward the foreground until it reaches a contrast floor against the background, because wallpaper
palettes often contain a slot barely above the background. `tui::terminal_palette` does the asking:
it sends the queries followed by a device-attributes request and stops reading when that reply
arrives, because every terminal answers it, so a terminal that ignores OSC costs nothing extra; a
300 ms cap covers the rest. A palette missing the background or any ANSI slot is rejected whole, and
the older behavior (background only, then neon) applies.

**Trade-offs.** The probe is Unix only (it uses `poll` and `read` on stdin), adding a direct `libc`
dependency to `tui`; other platforms keep the old behavior. It runs after the graphics probe so the
two never read stdin at once. Multiplexers that do not forward OSC 4 fall back as above.

**Follow-up (same day).** The automatic default turns off as soon as any `[theme]` field is set,
and the appearance popup saves one on the first change, so a user who had ever touched it could
never get the terminal-derived look back. `terminal` is therefore also a named palette, last in the
popup's cycle (so a neon theme on a silent terminal still reads as `neon`). The palette is kept in
a process-wide `OnceLock`, because `Theme::named` is called from many places that have no probe
result to hand; the merged `[theme]` is kept on `Config` so `main` can resolve it again once the
probe has run. The probe now runs on every start instead of only when no theme was set, at the cost
of up to 300 ms on a terminal that ignores OSC.

**Second follow-up.** `default` is also a name now, meaning the adaptive look (terminal colors,
else Catppuccin from the background, else neon), and it is the base for a `[theme]` with fields but
no name; `terminal` keeps the strict meaning. Because both depend on the probe, `main` resolves the
merged theme once after asking the terminal, which replaces the earlier special case for
`terminal`. An unrecognised name still falls back to the static neon palette.

**Verification.** `scripts/check` passes, including tests for the reply parser (one to four digit
channels, malformed specs, missing foreground, missing background or slot, silence) and for the
mapping (readable text colors, lifted dim slots, transparent backgrounds, the accent choice, a light
terminal, a missing foreground). Not tried: a real terminal, tmux, or a light wallpaper theme.

---

---

---

### Plugin Triggers: Several Keys, `:` Commands and `:plugin` (COA A)

**Date:** 2026-10-04 · **Author:** deltaog-117

**Context.** The plugin model stays additive: built-ins keep running natively and plugins in any
language add to them. A WASM tier, an action registry and "everything is a plugin" were weighed
first and deferred or declined (see the roadmap), because nobody is asking for those hooks yet and
the stage-1 host already covers any language. This cycle widens how a plugin is started.

| COA | Approach | Verdict |
|---|---|---|
| A | Several keys, `:` commands, `:plugin <name>`, and a Python example | Chosen |
| B | A, plus an on-demand state request and throttled events | Next cycle |
| C | Action names and `run_action` first, then A | Later, as its own cycle |

**Design.** `on_key` reuses the existing `one_or_many` deserializer, so a string and a list both
parse and old configs are untouched. `commands` is a list of single words. `PluginManager::spawn`
takes a `Triggers { keys, commands }` value, which grows without another signature change.
Two events carry the new data as optional fields (`key`, `command`, `args`), skipped when absent,
so existing plugins see byte-identical `init` and `key` lines apart from the new `key` field and
`API_VERSION` stays 1. A `:` line is offered to a plugin only after the built-in parser has
declined it, and only when it uses no shell syntax and splits cleanly, so a plugin can add a
command but never shadow `cd`, `q` or `mkdir`, and `:ls | wc` stays the shell's. `:plugin <name>`
is a parser variant of its own so it can report an unknown name instead of running a shell
command called `plugin`. When two plugins claim a key or a word, the one configured first wins.

**Example plugin.** `examples/plugins/dir_stats.py` closes the gap the roadmap named: nothing had
yet exercised a real external interpreter. It also shows the one subtle rule for a synchronous
plugin: events can arrive while a request waits for its reply, so it matches replies by `id` and
holds other lines until it is done.

**Trade-offs.** A plugin cannot yet register a command at runtime, only in config; a dynamic
`register_command` message would need a reply path and unregistration. `:plugin` arguments that
contain shell syntax (`*`, `|`) fall through to the shell, as every other built-in does. A key a
built-in owns cannot be claimed by a plugin until the user rebinds the built-in.

**Verification.** `scripts/check` passes. New tests cover the config (a key list, a bad key
skipped, blank and multi-word command words dropped), the wire format (no extra fields on a plain
event, the key spelled as in the config, command and args), the `:plugin` parser, an unknown plugin
name, and two end-to-end tests that drive the Python plugin through `PluginManager`. The real
binary was run in a PTY (a fresh config folder, `pyte` to read the screen): both keys, `:stats`,
`:plugin dir-stats touch made.txt` (the file appeared) and `:plugin nope` (listed what runs) all
behaved. The first keystroke after startup was sometimes lost in that harness, for a plain `j` as
well, which fits the terminal-probe artifact already noted in this log; not a plugin matter.

### Plugin `get_state`: A Published Snapshot (COA A)

**Date:** 2026-10-04 · **Author:** deltaog-117

**Context.** A plugin only knew the directory and selection that rode along on the event that woke
it, so a long-running plugin could not ask where the user was now. The state lives in `App` and
`BrowserState` on the render thread, while a plugin's requests are read by tokio tasks, so the
question was how a reply gets from one to the other.

| COA | Approach | Verdict |
|---|---|---|
| A | Publish a snapshot on a `watch` channel each tick; the reader answers from it | Chosen |
| B | The reader asks the render thread through a oneshot and `poll` answers it | Rejected |
| C | Return the state cached from the last event sent to that plugin | Rejected |

**Why A.** B is exact only to the tick it is answered in, so it is no fresher than a snapshot
taken at the end of the previous one, and it makes the plugin wait whenever the loop is busy or a
modal view owns it. C is stale by construction for the plugin that most needs the call. A also
builds the channel the throttled `dir_changed` and `selection_changed` events need next.

**Design.** `StateSnapshot` is a plain serializable value in `protocol`. `PluginManager` owns the
`watch::Sender`, each reader holds a `Receiver`, and `publish_state` uses `send_if_modified` so an
unchanged state wakes nobody and clones nothing. `App::poll_plugins` builds the snapshot only when
`PluginManager::is_empty` is false, so a session without plugins pays nothing. `get_state` is
answered inside `read_loop` and never reaches `execute` on the blocking pool; `execute` keeps an
error arm for it so the match stays exhaustive without a panic. `spawn` seeds an empty snapshot
with the start directory, so a plugin that asks while handling `init` gets a real answer. The view
mode lives in `main`'s loop, not in `App`, so its name is passed into `poll_plugins` rather than
moving the state.

**Trade-offs.** The reply can be one tick old, and the marks list is cloned each tick a change is
possible, which is cheap against a 100 ms tick but would matter with tens of thousands of marks.
The snapshot has no event for "changed" yet; a plugin has to ask.

**Verification.** `scripts/check` passes. New tests cover parsing `get_state` with no, empty and
null params, the snapshot's JSON shape and null cursor, `publish_state` reporting a change only
once, and three end-to-end tests: the Rust fixture sees a published cursor and then `none` after a
republish, the fixture sees the start directory before anything is published, and the Python
plugin reads cursor, marks, sort and view. The real binary was run in a PTY (a fresh config
folder, `pyte` for the screen, the harness answering the terminal probes including `ESC[5n`):
with two entries marked, `:plugin dir-stats state` showed `cursor=alpha.txt marks=2 sort=name
view=list`. The status line cuts a long message, so the example puts the short fields first.

### Plugin Events: Diffed Snapshots and a Folding Mailbox (COA B)

**Date:** 2026-10-04 · **Author:** deltaog-117

**Context.** After `get_state` a plugin could ask where the user was, but not be told when that
changed. The roadmap asked for `dir_changed`, `selection_changed` and `job_finished`, "batched or
throttled so holding `j` cannot flood a slow plugin".

| COA | Approach | Verdict |
|---|---|---|
| A | Diff the snapshot once per tick and queue the events on the plugin's existing queue | Rejected |
| B | A, with a latest-wins mailbox per plugin that the writer drains | Chosen |
| C | Push from every place the cursor or directory changes | Rejected |

**Why B.** A caps the rate at one event per tick but the queue is unbounded, so a plugin that
takes a second per event would still fall behind forever, which is the failure the roadmap named.
C is exact, but the cursor moves from keys, the mouse, search, undo, the live refresh and more,
and one missed site is a silent bug; the snapshot is already the one place that sees every change.

**Design.** `publish_state` compares the new snapshot with the one it replaces inside
`send_if_modified`, and posts to the mailbox of each plugin subscribed to what differs. The very
first publish fires nothing, since it is the browser arriving. `dir_changed` fires when `cwd`
differs; `selection_changed` when the cursor or the marks do, so a mark made without moving is
heard, and a directory change normally brings both. The mailbox (`plugins::mailbox`) holds one
`dir_changed` and one `selection_changed`; a new one keeps the earlier `from`, and a directory
burst that ends where it began is dropped. The plugin's writer task now selects between its line
queue and the mailbox, so a write that waits on a full pipe holds both back while the mailbox keeps
folding: a stuck plugin costs two slots of memory with no timers and no thread. Events are opt-in
(`events` in `[[plugin]]`, held as plain strings in `theming` and parsed by `plugins::EventKind`
at spawn, where an unknown name goes to stderr) so an existing plugin sees no new traffic.
`job_finished` is posted from the terminal branches of `poll_bulk`, not from a wrapper around it,
because a batch paste re-spawns itself and a conflict prompt pauses it, and neither is the job
ending; it goes through the ordinary queue and is never folded. Marks are now sorted when
published: they come out of a `HashSet`, and an unstable order would have made every tick look like
a change.

**Trade-offs.** A plugin sees where the cursor settled, not each step, and the order of a folded
event against a queued line (a `command` fired in the same instant) is not guaranteed. Delivery is
at most one tick behind. `job_finished` reports `completed` for a `:` command that ran and exited
non-zero; the exit status is in the status line, not the event.

**Verification.** `scripts/check` passes. New tests cover the event names, the `from`/`to` and job
shapes (null for no entry), the config list, the mailbox (a burst folds to one, a round trip
cancels, marks alone are delivered, directory first), a property test over any run of changes, and
end-to-end tests with the Rust fixture and a real `python3`: moves heard, nothing for the first
publish, nothing without a subscription, jobs in order, and 20,000 changes of 2 KB each published
at a plugin that never reads, in well under the five seconds allowed. The real binary was run in a
PTY (`pyte` for the screen, the harness answering the terminal probes): `l` into a folder showed
`entered sub`, `h` showed `entered tree`, and marking a file, yanking and pasting into the folder
showed `copy completed`, with the file on disk.

### Plugin Restart Policy: Respawn on Next Use (COA C)

**Date:** 2026-10-04 · **Author:** deltaog-117

**Context.** A plugin that exited got one log line if it crashed and nothing otherwise, but it stayed
in the list, so `dispatch_key` found it, reported the key as handled and wrote into a dead channel.
A key bound to it did nothing, with no word on why.

| COA | Approach | Verdict |
|---|---|---|
| A | No restart; show state, add `:plugins` and a manual restart | Rejected |
| B | A supervisor task per plugin restarting on a crash with backoff | Rejected |
| C | Start a plugin again the next time something is meant for it, with a crash-loop guard | Chosen |

**Why C.** It fixes the visible bug with no background task and no timers. It also makes the
one-shot style natural: the protocol is "read a line, print a line", so a script that handles one
key press and exits is legitimate, and starting it per use is what its author expects. B only pays
off for a plugin that does nothing but listen for events, and `:plugins restart` covers that until
someone needs more; B can be built on the list and the guard that C adds.

**Design.** `reap` now records how each process ended in a `Life` shared with the manager:
running, finished (status 0) or crashed (the status text and how long it ran). Nothing watches for
deaths; `ensure_alive` looks when a key, command or `:plugin` is about to be delivered, and for an
event only when the plugin has no keys or commands, since otherwise nothing would ever bring a
listener back. Finished resets the crash count and respawns. A crash goes through the pure
`life::after_crash`: a crash after 60 s of running restarts the count at one, any other adds one,
and the third in a row gives up. A plugin the host has given up on says so on each key or command,
with the command that fixes it, because the original bug was exactly a key doing nothing; events
to it are dropped silently so the status line is not flooded every tick. A process that cannot be
started at all (the binary has gone) is given up on at once, since a retry changes nothing. Each
process has its own queue, mailbox and `Life`, so nothing from a dead one is delivered to its
successor, and the old writer ends when its sender is dropped. `restart` marks the old `Life`
stopped and notifies `reap`, which kills the process and stays quiet about it. It then starts
afresh with the crash count cleared. The manager's dispatch methods now take `&mut self`, as
starting a process changes the plugin; every caller already held it mutably. The list and
`restart` are `:plugins` and `:plugins restart <name>`, not `:plugin <name> restart` as first
proposed, because that would take the word `restart` away from a plugin's own arguments.

**A bug the tests found.** A respawned listener's first change event was sometimes written before
its `init`, so the plugin never saw `init` and, in the test, exited on the event. The writer chose
between its queue and its mailbox at random; the select is now biased towards the queue, whose
first line is always `init`.

**Trade-offs.** A plugin started again has none of the old process's memory and sees `init` again,
which the docs now say. A process that dies in the instant before a key is sent is not noticed
until `reap` has recorded it, so that one key can be lost; the next one starts it. A plugin with a
key that has died misses events until a key or command brings it back. `names()` still lists every
configured plugin, so the "running:" in an unknown-name message can include a stopped one.

**Verification.** `scripts/check` passes. New tests cover the crash verdicts (the limit, the
60-second reset, a property test over any spacing of crashes and over any long run), the list's
wording, the `:plugins` parser (and that other `plugins` lines still go to the shell), and end to
end tests with the fixture, which can now exit on purpose: a one-shot plugin answers two keys from
two processes, a plugin that crashes at once is tried three times and then stopped (and keeps
saying so), `restart` revives a stopped plugin and replaces a live one without a crash message and
with the old process gone, a listener is started again by an event, and a plugin with a key is
not. The real binary was run in a PTY with a one-shot and a crashing plugin: two key presses ran
in pids 121333 and 121344, three presses of the crasher's key ended in the stopped message, which
the next two repeated, `:plugins` read `oneshot: finished, restarted 1x; crasher: stopped after
repeated crashes (exit status: 1), restarted 2x`, and `:plugins restart crasher` started it again.

### Plugin `register_command`: A Shared Registry (COA A)

**Date:** 2026-10-04 · **Author:** deltaog-117

**Context.** A plugin's `:` commands could only come from `commands = [...]` in config, so a plugin
could not add one while running. The roadmap asked for runtime `register_command`.

| COA | Approach | Verdict |
|---|---|---|
| A | One registry behind a lock, shared by every plugin's reader and the manager | Chosen |
| B | The reader sends the request to the render thread, which applies it on a tick and replies | Rejected |
| C | A reply-less notification applied on the tick, a refusal logged as a warning | Rejected |

**Why A.** The same reasoning as `get_state`: a plugin's request should not wait on the render loop,
and while the interface is handed to `:nvim` the loop is not ticking, so B and C would leave a
plugin blocked or unanswered for as long as the editor ran. A also gives one answer to "who owns
this word", which can be tested as a plain type. C was only ever the cheap option, and it leaves a
plugin unable to know whether it worked.

**Design.** `Registry` maps a word to its owner and whether it came from config or from a running
plugin, and holds the reserved words. `register` checks, in order, that a runtime word is made of
letters, digits and `-_.+` (a word with a space or a shell character could never reach a plugin,
since such a line goes to the shell), that it is not reserved, and that no other plugin has it;
the same plugin asking twice is accepted, which is what lets a respawned process register again.
Config words go through the same call, minus the character rule, which they never had, so a
config word that clashes is silently dropped as it always was and `commands` is no longer a list on
the plugin. `dispatch_command` asks the registry for the owner instead of scanning plugins. The
reader answers `register_command` and `unregister_command` itself, like `get_state`, and the
refusal text goes back as the error. Registered words are kept when a process ends, since lazy
respawn depends on `:blame` finding a plugin that has exited, and are cleared only by `restart`,
whose fresh process registers them again. The event-only test for respawn now asks the registry,
so a plugin that has registered a command is not started again by an event. The reserved list is
`command::BUILTIN_WORDS`; a test scans the source of `parse`'s `match` for its arms and compares,
which fails both when a built-in is added without being listed and when one is listed that does
not exist (checked by removing `touch` from the list). A `description` param is not read: storing
it with no place to show it would be speculative, and an unknown param is ignored, so a plugin may
already send one.

**A fixture bug the tests found.** The fixture took the next stdin line as a request's reply. After
a respawn the host had already queued the command that started it, so the fixture swallowed that
event as the reply and the real reply as an unknown message. It now matches replies by `id` and
sets events aside, as the Python example does and as the protocol note always said a client must.

**Trade-offs.** A registered word that clashes with a plugin configured later loses to the earlier
one, with no message to the user, only to the plugin that asked. A plugin that registers a word and
is then given up on keeps the word, and the command says why it does nothing. The lock is held for
a lookup or an insert, never across an await. Registering needs the plugin to be running, so a
word a plugin registers at `init` is not there until `init` has been handled, a few milliseconds
after startup.

**Verification.** `scripts/check` passes. New tests cover the two request shapes and the ignored
`description`, the registry's rules one by one (first come, repeat, taken, built-in, character
rule, config's looser rule, who may unregister, clearing, the errors' wording) and a model-based
property test over any sequence of registers, unregisters and clears, plus the reserved-word test
above and end-to-end tests with the fixture and a real `python3`: a word reaches the plugin that
registered it and not before; a built-in, a taken word and a bad name are each refused with their
reason and the word stays put; config and registered words answer alike and a config word cannot
be given up; an unregistered word stops reaching the plugin; a registered word still starts a
plugin that has exited; and a restart forgets the registered word until the new process asks
again. The real binary was run in a PTY: a plugin with no `commands` answered `:state`, another
was told `cd=that is a built-in command | hello=ok | a b=a command name is one or more letters,
digits, '-', '_', '.' or '+'`, `:hello world` reached it with its argument, and `:cd /tmp` was
still the built-in.

---

### Appearance Shape and a Paged Appearance Popup (COA B)

**Date:** 2026-10-05 · **Author:** deltaog-117

**Context.** The look could only change colors, glyphs, text styles and four frame styles. The goal
was to change its shape too (frames, spacing, effects), edited in a settings window like a
Hyprland appearance configurator, with saved themes keeping the shape. A terminal draws whole
cells, so Hyprland's pixel settings map only partly: rounding becomes the frame style, border width
becomes the frame weight, gaps are counted in cells, the shadow is drawn with cell colors, and
dimming uses the dim attribute. Blur and opacity have no equivalent.

| COA | Approach | Verdict |
|---|---|---|
| A | A Shape category in the existing two-level popup, `[shape]` in its own table | Rejected |
| B | A, plus the popup rebuilt as pages beside rows, and saved themes keeping the shape | Chosen |
| C | Named shape presets on one row, everything else by hand in the file | Rejected |

**Why B.** It was the user's choice: the point was a configurator that works like the one on the
desktop, which A gets only halfway (no page list, themes without shape) and C not at all (no live
fine-tuning).

**Where the shape lives.** Inside `Theme`, as `Theme::shape` and `RawTheme::shape`, not as a
fourth `local_*` table beside it. `border_type` and `separator` were already shape settings
inside the theme, and putting the rest there too means a saved theme (a `RawTheme` snapshot),
`local.toml`, the live preview, Reset and the "update or new" comparison all carry the shape with
no new state threaded through `main`. A separate table would have needed a new argument on
`persist_local`, `apply_appearance_outcome` and every call to them. The file still has its own
`[shape]` table, merged into the theme when loading, since a gap is not a color. Three places
compare themes and needed care: `theme_is_customized` ignores the shape, so a `[shape]` alone
keeps the colors following the terminal; `active_preset` compares colors only, so changing the
shape never makes a palette look inactive; and `effective_theme` keeps the configured shape under
a palette picked in the popup, which before replaced the whole base theme.

**Types.** A gap is `Gap<const MAX: u8>`, which can only be built through `clamped`, so a negative
or huge number from the file or a held key cannot reach the layout. `TitleAlign` is an enum with a
fallback on unknown names. `border_type` stays a string, as before; `none` maps to blank frame
symbols rather than no frame, so every pane keeps its one-cell border and `list_area`'s row
arithmetic is unchanged.

**Layout.** The gaps are applied in `view_mode::browser_layout`, which drawing, the drag and drop
target and every mouse handler already call, so the gapped columns and the rows a click resolves
to cannot disagree. `gaps_out` uses half as many rows as columns, because a cell is about twice as
tall as it is wide, and shrinks on a small terminal so the browser stays at least 20 by 6.

**The window.** `Page` replaces `Category`/`View`; pages switch with `Tab`/`Shift+Tab`, a number or
a click, and the popup is sized for the longest page so it never resizes while switching. The
Theme page merges the old Theme row, the Saved themes category and the two action rows. A preset
now replaces hand-picked colors (the old Theme row kept them on top of the new palette, which made
"pick dracula" not look like dracula) while keeping the frame style, separator and shape, which
belong to the Shape page. The save, rename, delete and color-picker flows are unchanged. Geometry
is one pure `areas` function used by drawing and by `hit`.

**Trade-offs.** Dimming has no strength setting, since the dim attribute is on or off. The shadow is
black from the 256-color cube rather than ANSI black, which a terminal may set to its own
background; on a light theme it is heavy. A saved theme from before shapes existed resolves to the
default shape. Choosing a palette now drops hand-picked colors, a change from before.

**Verification.** `scripts/check` passes. New property tests: a gap is always in range and stepping
undoes itself, a saved shape resolves back to itself over any base, the top table wins per field
and a raw shape survives TOML; the gapped layout never puts a pane off screen or over another,
and no gaps is exactly the old layout; the popup's page list, divider, rows and hint never
overlap, and `hit` never names a missing page or row. Unit tests cover the new frame styles (all
different, `none` keeping the inner area), title alignment, the shadow's placement and clipping,
the example file documenting every shape field and style, and the popup's keys, steps, toggles and
preset rule. The real binary was run in a PTY through `pyte` with a throwaway config folder: 23
checks passed, covering the five pages, frames at columns 5, 22/25, 58/61 and 94 with
`gaps_in = 2` and `gaps_out = 5`, a click on `charlie.txt` selecting it, every change written at
once, the saved theme's shape table, a palette keeping the gaps, Reset dropping them and the saved
theme bringing them back, the shadow appearing with a popup and going with it, and a click on
"2 Colors". The harness had to answer the startup probes' Device Attributes and status queries,
as a real terminal does. Release binary: +40,360 bytes (+0.28%), measured by building `HEAD` and
the change in two worktrees at paths of the same length, since panic locations embed the source
path and a longer path alone made the first measurement wrong.

---

### Configurable Border Gradient (COA B)

**Date:** 2026-10-05 · **Author:** deltaog-117

**Context.** The border gradient was a `gradient_borders` switch in `config.toml`: fixed colors
(accent to focused border), one sweep around the focused pane, read once at startup. With the
look now edited live and saved as themes, the gradient was the one piece of it outside both.

| COA | Approach | Verdict |
|---|---|---|
| A | Move the switch into `[shape]` and the Shape page, nothing else | Rejected |
| B | A, plus the two colors, where it applies and which way it runs | Chosen |
| C | B, plus a gradient turning around the frame on the animation tick | Deferred |

**Why B.** A only moves a switch. C redraws on every tick even when nothing changes, works against
the quieter looks the appearance work has been heading toward, and its speed setting belongs on a
Motion page that does not exist yet, so it went to the roadmap as a future item.

**Design.** The ends are theme colors, `gradient_from` and `gradient_to`, so a saved theme keeps
them and a palette pick resets them. They default to the keywords `accent` and `focused`, which
`Theme::resolve` turns into `accent_fg` and `border_focused_fg` at drawing time, the same idea as
`selection_fg = "keep"`. Literal copies of the two colors would have changed what users see:
anyone with a custom accent in `appearance.toml` would have had the gradient snap back to the
palette's. The keyword also means a saved theme's gradient follows that theme's own accent. Where
it applies and which way it runs are shape settings, `GradientScope` (`off`, `focused`, `panes`,
`all`, each including the ones before it) and `GradientFlow` (`around`, `mirror`, `across`,
`down`), so a palette pick keeps them. The old switch survives as the lowest layer: `false`
becomes `gradient = "off"` beneath `appearance.toml` and `local.toml`, and like any shape setting
it does not stop the colors following the terminal. `Config::gradient_borders` was removed rather
than kept beside the new setting, where it could contradict it.

**Drawing.** `style::paint_gradient` resolves the ends and calls `gradient::paint`, which takes the
flow; `mirror` is `1 - |2t - 1|` along the perimeter, and `across`/`down` use a cell's column or
row instead of its place on the perimeter. The focused column is still painted before its
scrollbar, as before. With `panes`, the side columns are painted once both are drawn and before
`dim_inactive` dims them; a parent column lit as the "up a level" drop target keeps its plain frame,
since that is what marks it. With `all`, each popup and menu paints its frame right after drawing
it. In the popup, editing an end set to a keyword starts from the color it stands for, so the
picker opens on it rather than on white, and the swatch shows the resolved color.

**Trade-offs.** `panes` gives every column the same gradient, so the focused pane is marked only by
its title and by `dim_inactive` if that is on. Grid tiles and the shell panes are not painted: the
tiles would be noise, and the shell panes already animate their own border on a focus change.
Without true color nothing blends and frames stay flat, as before.

**Verification.** `scripts/check` passes. New tests: each flow's endpoints (`across` and `down`
one color per edge, `mirror` meeting itself), a property test that every flow paints only the
frame's edge and stays between its two colors, the scope order, name round trips, keyword
resolution (following a changed accent, kept by a saved snapshot), the old switch beneath
`appearance.toml` and not pinning the colors, and the popup's new rows. The real binary was run in
a PTY through `pyte`, reading each frame cell's truecolor value: 39 shades along the focused frame
and one along the parent's bottom edge by default, gradients on the side columns with `panes`, one
color per side edge (`ff2bd6`, `00f0ff`) with `across`, `ff0000` on the left edge after setting
the start, a gradient on the popup's own frame with `all`, flat frames with `off`, all four in
`local.toml`, and `gradient_borders = false` in `config.toml` still giving flat frames. The earlier
shape scenario still passes all 23 checks. The release binary was not re-measured; this adds no
dependency.

---

### Plugin `run_action`: The Browser's Own Handling, Entered by a Plugin

**Date:** 2026-10-06 · **Author:** deltaog-117

**Context.** Plugins could read and change files and ask for state, but could not do what a key does:
sort, paste, undo, toggle hidden files. The roadmap's action registry item asked for string names
and a `run_action` entry point.

**Approach.** The render loop takes a plugin's request as a turn of its own and runs it through the
same handling as a key. An action works on state only the render thread holds: the browser, the
open popups, the clipboard and the prompts. Running it on the plugin's reader thread would need all
of that behind locks, and a second dispatcher for plugins would write every action twice and let the
two drift, so that a plugin's `paste` skipped a confirmation the key shows. This way the action is
the key's action by construction, and what the user would be asked is still asked.

**Design.** `theming::Action` gained `ALL`, `name`, `from_name` and `waits_for_key`. The names are
the `[keys]` words, so the vocabulary is the one users already know, and they are now part of the
plugin API and never change once released. `name` is an exhaustive `match`, so a new variant fails
to compile until named; `ALL` is a plain array a test checks, since the compiler cannot. The
plugin's reader parses `run_action` and `list_actions`; `list_actions` is answered at once from
names the interface declared at startup (`declare_actions`), and `run_action` becomes an
`ActionRequest` carrying a one-shot reply channel, queued for the render thread. `main.rs` took the
body of its key handling into one step that takes a `Browse`, either a key press resolved through
the keymap or an action a plugin asked for, so both go through the same `match`. A plugin turn
replaces the wait for a terminal event rather than adding to it, so a plugin's request is not
delayed by an idle terminal. `plugin_action::accept` decides first: an unknown name, a chord
starter, an open popup or view, a prompt, a chord waiting, a focused shell or a running job each
refuse with a reason, so the user's own state always wins. After an action runs, its reply waits
until `poll_plugins` has published the state it left, so a plugin that asks `get_state` right after
the reply does not see the old one. Dropping a request unanswered tells the plugin the interface
closed.

**Trade-offs.** An action that opens a prompt (`rename`, `create`, `sort`) is run and leaves the
prompt open for the user, and the plugin gets `ran`, not the prompt's outcome; arguments to an
action wait for the prompt to be split from the operation. The chord starters are refused because
they would leave the interface waiting for a key nobody will press. A plugin cannot run an action
while the user is in the middle of something, which is by design and means it must be ready to be
told no. The key handling moved as a whole, so the diff of `main.rs` is large while its behaviour is
meant to be the same.

**Verification.** `scripts/check` passes. New tests cover that every action's name is the config key
that binds it, that names are distinct and round trip, that only the chord starters wait for a key,
and properties over any name a plugin might send (exact match only: no case folding, no prefix, no
trimming); that plain browsing runs every listed action and that every other mode and any open popup
refuses each one with the reason named; the request shapes; and end to end with the fixture and a
real `python3`: a request waits for the interface and hears whether it ran, `list_actions` is
answered without the interface, a dropped request gets an error, and the Python example runs an
action by name and lists them.

---

### Plugin Cost Benchmarks: Ignored Tests Beside the Integration Tests (COA A)

**Date:** 2026-10-06 · **Author:** deltaog-117

**Context.** Stage 1.5 promised that plugins cost nothing to anyone not using them, a few
milliseconds to start and a sub-millisecond round trip per call, and said these were estimates to
measure after the stage. With `run_action` the stage had nothing left to add, so this cycle measures
them and closes it.

| COA | Approach | Verdict |
|---|---|---|
| A | `#[ignore]`d benchmark tests in `plugins/tests`, run by hand with `--release` | Chosen |
| B | A criterion bench in `plugins/benches` | Rejected |
| C | Fix the roadmap text and leave the estimates unmeasured | Rejected |

**Why A.** It is how the project already benchmarks (the disk-usage and archive figures come from
ignored tests), it adds no dependency, and it answers the question with numbers. B's saved baselines
suit CI, but spawning a process per iteration is noisy enough that its statistics would not earn the
new dev-dependency. C would have left the promise open.

**Design.** `tests/plugin_cost.rs` has three benchmarks. `spawn_to_first_log` times the call to
`spawn` until the plugin's `ready` log is polled, a median of 25 starts, for the Rust fixture and for
`dir_stats.py` under `python3`. `request_round_trips` has the fixture send 2000 back-to-back
`list_actions`, `get_state` or `run_action` requests and reports the mean, the host answering each
`run_action` as the render thread would. `an_empty_manager_costs_nothing_per_tick` calls what the
render tick calls on a manager with no plugin a million times. The waits spin on `poll` rather than
sleeping, since a sleep would be larger than what is measured. The fixture gained one flag,
`--bench=METHOD:N`, that sends the N requests at `init` and logs when it is done; timing stays in the
test, so the fixture still writes its JSON by hand.

**Results.** On a Ryzen 5 5500U, release build, three runs: about 0.9 ms to start the Rust fixture
(up to 1.25 ms on the noisiest run) and 23 to 31 ms for the Python example, almost all of it the
interpreter's own start; 14 to 22 µs per request and reply, with no meaningful difference between
the three methods; 22 ns per tick for an empty manager. The estimates held, with room: the round
trip is far under the millisecond that was budgeted, and a user with no plugins pays a few
instructions.

**Trade-offs.** The numbers are one machine's and include the fixture's own scheduling, so they bound
the cost rather than state it exactly. The round trip is measured with the render thread answering
in a tight loop; in the real interface a `run_action` waits for the next tick (up to 100 ms), which
is a property of the tick and not of the protocol. With no stored baseline the 5% rule is applied by
running twice and comparing, which suits a feature this small. `get_state` is measured against the
snapshot's default value, not a large directory's, so its cost with many marks is not covered.

**Verification.** `scripts/check` passes; the benchmarks are ignored there and were run by hand.

---

### Fuzzy Finder on `f` (COA A)

**Date:** 2026-10-07 · **Author:** deltaog-117

**Context.** `/` jumps the cursor to the nearest good match by name. What was missing is the
other way to find a file, which is to see the candidates: a list ranked as you type, over the
whole tree below, with a look at each one before going to it, as in Neovim's Telescope.

| COA | Approach | Verdict |
|---|---|---|
| A | A popup over a background index of the tree, ranked by path, with a preview, built on the existing scorer and popup patterns | Chosen |
| B | A: plus modes in one popup (directories, content grep, marks, history, multi-select) | Rejected for this cycle |
| C | Run `fd` and `fzf` in the shell overlay | Rejected |

**Decision.** `browser::finder` holds what is pure: `Candidate` (a path, its path below the root
and whether it is a folder), `score` (the `/` search's `Query::score` applied to the path, with a
bonus when the name itself matches), `Ranking` (a running ranking of a growing index) and `walk`
(the nearest-level-first walk, which hands each directory's entries to a callback). In `tui`,
`finder_job` runs the walk on the blocking pool, `finder_popup` is the pure state (query, index,
results, selection), and `finder_view` draws it. `main` owns the preview pipelines, so it draws the
preview pane from `FinderPopup::preview_subject`, the way it does for the trash browser, and
`finder_view` borrows `trash_view::layout` to split the panel. `Action::Find` is bound to `f`.

**Why this way.** A flat index and a ranking, not `find_below`'s nearest-level answer, because the
point is to see several candidates and narrow them by typing; the index is built once and each
keystroke only scores it again. `Ranking` scores each entry once and only the new entries as the
walk delivers them, and `set_query` starts over, so a walk still running costs one pass in total.
Entries reach the popup as the walk lists directories, so the panel is usable at once on a large
tree and the walk stops when the panel closes (dropping the job sets its cancel flag, as for `/`).
Everything goes through `Vfs::list_dir`, so the finder works on a remote machine and inside an
archive, which `fd` and `fzf` would not (C). Matching the path as well as the name gives `src/main`
a meaning; a 500 point bonus for a name match keeps `main` finding `src/main.rs` ahead of a path
that merely holds the letters. The popup and its `Ranking` are the only places that know there is
one source of entries, so a second source (B) is an addition and not a rewrite.

**Trade-offs.** Every key but the arrows and `Ctrl-N`/`Ctrl-P` goes to the query, so `j` and `k`
are letters and `Ctrl-D`/`Ctrl-U` scroll the preview in place of `J`/`K`. `Enter` shows the entry in
the browser and does not open it, because opening goes through the browser's own handling (the
associations, the `Open with` rules), which this cycle left alone. The index is capped at 16 levels
and 200,000 entries, shared with `/`, and does not read `.gitignore`, so a project with a large
`target/` can fill the cap, though the walk reaches such a folder last. The matched letters are not
highlighted because the scorer returns a score and not positions; adding positions means a
backtracking pass through the scoring table that must agree with it exactly. `trash_view::elide_left`
became `pub(crate)` to be reused, the only change to existing code besides the new action and the
wiring in `main`.

**Verification.** `scripts/check` passes (1256 tests). New tests: ranking properties (every result
matches, at most the limit, best first, the match count agrees with the index), incremental ranking
agreeing with scoring all at once, the walk (nearest level first, hidden entries, both limits,
cancellation), the job and its cancel flag, the popup's keys and preview subject, and the drawn
panel on a range of screen sizes including none. It has not been run by hand in a real terminal and
has not been timed on a large tree.

---

### Fuzzy Finder Off the Render Thread

**Date:** 2026-10-07 · **Author:** deltaog-117

**Context.** The finder scored every entry on the render thread on each keystroke, and rebuilt its
best-200 list from a copy of every match each time the walk delivered a directory. On a tree of
hundreds of thousands of entries, such as `/`, that stutters. The requirement was to stop the
stutter without listing less: no skipped directories, no lower cap, no partial results kept.

| COA | Approach | Verdict |
|---|---|---|
| A | Exact savings and an engine thread: narrow from the previous letter's matches, work in slices on a worker, send snapshots | Chosen |
| B | Time-slice the scoring on the render thread | Rejected: results are partial for longer, and the thread is still busy |
| C | Skip `/proc`, `/sys` and ignored folders, or lower the entry cap | Rejected: lists less |

**Decision.** `browser::finder::Ranking` keeps one level per typed prefix. A longer query only looks
at the previous query's matches (`Query::extends`: whatever holds `abc` in order holds `ab` in
order), entries that arrive later are scored once when reached, and `advance` takes a budget so the
work can be cut anywhere. `tui::finder_job` runs two jobs on the blocking pool: the walk, and an
engine that owns the index and the ranking. The engine scores 2048 entries at a time, looks at its
queue between slices so a new letter takes over within about 2 ms, and sends a `Snapshot` (the best
200, the match and indexed counts, whether it has caught up) at most every 40 ms, the first one after
a query change at once. `FinderPopup` holds the newest snapshot and no index. The folder preview is
read once per selection on the blocking pool, limited to 300 entries.

**Why this way.** The cost is in scoring, so the saving that changes the result least is not to
repeat it: typing narrows, deleting returns to a kept level, and a late entry is scored once. What
cannot be avoided, the first letter over everything, moves off the render thread, where its length
only delays the answer. Snapshots hold the best 200 only, so the render thread copies 200 entries
and not a quarter of a million, and a snapshot of an old query is recognised by its generation and
ignored, with the previous results left on screen so typing never flashes an empty list. The
selection follows its path across snapshots, so results arriving under the cursor do not move it.

**Results.** On a Ryzen 5 5500U, release build, 200,000 synthetic paths (the ignored test
`crates/browser/tests/finder_cost.rs`): the first letter costs about 190 ms in total and the longest
slice is about 2 ms; narrowing to two, three and four letters takes 64, 12 and 3 ms; deleting a
letter takes about a microsecond; picking the best 200 of 58,000 matches takes 1.5 ms.

**Trade-offs.** The first letter over a very large index still takes the better part of a second
of work to finish, during which the list shows the previous results and the header says
`ranking…`; it cannot be shorter without scoring less. Each prefix keeps its matches, up to a few
megabytes for the first letters of a large index. A query that is not an extension of the last one
(a letter deleted from the middle) is ranked from scratch. Two threads of the blocking pool are used
while the finder is open. The walk itself is unchanged and still stops at 200,000 entries. The
numbers are one machine's, from synthetic paths, and do not include the walk's own disk time.

**Verification.** `scripts/check` passes (1264 tests). New: a property test that types and deletes
letters in any order, with the index growing and the work cut into slices, and requires the same
ranking as scoring from scratch; engine tests for generations, a new query taking over a large
index, and the cancel flag ending the engine; popup tests for results staying until replaced, the
selection staying on its entry, and a folder's contents arriving in the background. It has not been
run by hand in a terminal on `/`.

---

### Devices View: The Storage Deck (COA B)

**Date:** 2026-10-07 · **Author:** deltaog-117

**Context.** `V` switched between four ways of showing a folder, and none of them knew the machine
had disks. Graphical file managers put the disks, sticks and shares in a sidebar; Ranger has
nothing. The aim was the same information, in a layout that is neither of those.

| COA | Approach | Verdict |
|---|---|---|
| A | A places rail on the left: Devices, Places, Remote | Kept on the roadmap |
| B | A strip of live cards above the columns: gauge, free space, read/write sparkline, you-are-here frame, slide-in | Chosen |
| C | An atlas: a zoomable map of all storage with a pin for where you are | Kept on the roadmap |

**Decision.** A new crate, `devices`, holds everything that knows about Linux storage, as parsers
over text plus one probe that reads the real files. `mountinfo` keeps the mounts worth showing
(real block devices and network shares, one per device, leaving out `/proc`, `/sys`, `tmpfs`,
snap loop devices, container layers and `/boot`). `udev` reads `/run/udev/data/b<major>:<minor>`,
which anyone can read and which covers a stick that is plugged in but not mounted, for its label,
filesystem and bus. `probe::Sys` joins those with `/sys/class/block` (size, removable, which whole
disk a partition is on) and `statvfs`; its locations are fields, so the tests point it at a copy
of the files. `activity` turns `/proc/diskstats` into rates and a 32-sample history. `udisks` runs
`udisksctl mount`, `unmount` and `power-off` with a timeout. In `tui`, `deck` is the state and
`deck_view` the drawing; `ViewMode::Deck` follows `Filmstrip` in the cycle, and
`view_mode::browser_layout` cuts a five-row strip off the top of the columns and returns it as
`BrowserLayout::deck`, so the drawing and the mouse read the same rectangle.

**Why this way.** Parsing text and keeping the probe's paths as fields means nearly all of it is
tested without a disk, and the real machine is checked once by hand. The scan runs on the blocking
pool once a second, and a scan still running is not started again, because `statvfs` on a network
share that has gone away can block for a long time and must never reach the render thread; the
deck exists only while its view is showing, so the machine is not polled for anyone who is not
using it. The keys follow the rest of the interface: the deck takes only `Tab`, the arrows,
`Enter`, `m`, `e` and `Esc`, and only after `Tab`, so `q`, `j` and `k` are still the browser's.
Cards are lit as "you are here" by the deepest mount that holds the browsed folder, so a folder
on a separate `/home` belongs to that card and not to `/`. A device appears by sliding in, not
by popping up, because a stick being plugged in is the one event on this screen that happens
without the user doing anything and should be noticed.

**Found against the real machine.** The first scan listed the root filesystem twice. A btrfs
mount reports a made-up device number, so the root was both a mounted device and, by its real
node (`dm-1`), a device that looked unmounted; and the hidden `/boot/efi` partition showed up as
unmounted. Devices are now matched by the node their mount source links to (`/dev/mapper/root`
is a link to `dm-1`) and every mount's source is counted, listed or not. Both cases are in the
fixture the tests use.

**Trade-offs.** Linux only, and mounting needs `udisksctl`; without it the cards show and nothing
can be changed. A network share can be opened but not mounted or unmounted from the deck.
Encrypted containers are not offered. The scan repeats every second even when nothing changed;
listening for udev events would be free but is a larger piece of work. The deck lives only in its
own view, not as a strip over the others. Cards need 16 columns to show their content, so a very
narrow window shows bare frames, and a window under twelve body rows gets no strip at all.

**Verification.** `scripts/check` passes (1312 tests). New: the parsers on realistic text (escaped
mount points, a device mounted twice, hidden mounts, a cut escape), the probe over a fixture
machine with a system disk, a mounted stick, an unmounted stick, a share, a hidden boot partition
and a mapped device, `statvfs` on the real root, the rates and history, the layout (cards side by
side, scrolling following the selection, a click landing on the drawn card, the slide-in ramp),
the drawing at sizes down to nothing, the keys (focus, choosing, what stays the browser's, what
network shares and fixed disks refuse and say), and the strip cut from the columns. Not done: a
real stick plugged in, and `udisksctl` actually run; the mount, unmount and eject path is covered
only up to the command line.

---

### Cargo-Aware Deck: Knowing It Fits Before the Copy

**Date:** 2026-10-07 · **Author:** deltaog-117

**Context.** The deck listed devices but did not react to anything the user was doing. The first of
the three follow-up ideas (cargo-aware deck, runway, wiring diagram) was chosen because it prevents
a real failure: a copy to a stick that is too small, or to a FAT32 stick with a file over 4 GiB,
is found out halfway through, leaving half a file behind.

| COA | Approach | Verdict |
|---|---|---|
| A | Cards say whether the carried files fit, and `p` sends them there or refuses with the reason | Chosen |
| B | The same, plus choosing a folder on the device to send to | Left for later |
| C | A confirmation dialog at paste time that warns when it will not fit | Rejected: it warns after the choice is made, and only for the one device chosen |

**Decision.** `cargo_fit` is pure: a `Load` (bytes, whether the count is exact, the biggest single
file, copy or move, where it comes from) and a `Device` in, a `Verdict` out. The verdicts are fits,
tight, short by so much, a file over the filesystem's limit, a move within one disk, not mounted,
and cannot tell. `App::load` supplies the load: the clipboard if there is one, else the marks
(a yank spends the marks, so it is never both), sized by `marked_size`. `Tally` gained `largest`, and
`marked_size::measure` returns bytes, exactness and the biggest file, with `total_of` kept as it was
for the header pill. The clipboard gets its own `MarkedSize`, so the pill is not disturbed. The
deck holds the load, gives each card its verdict, and a card shows it in place of the activity
line, dimmed when the load cannot go there. `p` returns `Outcome::Send(root)` for a device that can
take it; `main` yanks the marks if the clipboard is empty and calls `begin_paste_into`, so progress,
cancel and the overwrite prompt are the ones every paste has.

**Why this way.** Judging per device, up front and for every card at once, is the only way to see
at a glance which device can take the load. The check is a function of numbers, so its edges are
tested exactly: the byte that tips a load over, the FAT32 limit allowed to the byte and refused one
byte over, and a move within one disk needing no room at all. The tight band exists because a
filesystem rounds every file up to a whole block; a load that leaves less than 16 MiB or 2% of
itself is not called fitting. Refusing in the deck, with the reason, rather than letting the
paste fail, is the whole point; the paste itself is untouched.

**Trade-offs.** It is a prediction. Another program can fill the disk between the check and the
copy, and the 2% margin is a guess at block rounding, not a measurement of it. Only the FAT family's
file size limit is known; other limits (exFAT's forbidden characters, path length) are not checked.
A load too big to measure fully (the walk stops at its entry limit) is called `cannot tell` unless
the part measured is already too much. Sending goes to the root of the device, with no choice of
folder. Because a cut onto another disk is a copy and then a delete, it needs room like a copy.

**Verification.** `scripts/check` passes (1327 tests). New: the verdict at its boundaries (every
case above), the card lines and refusals including the ASCII glyph set (which a test showed was
printing a non-ASCII separator, fixed), the biggest file found in marked files and inside marked
folders, each card getting its own verdict, `p` sending to a device it fits and refusing one it does
not with the reason, a move within one disk allowed whatever is free, and the drawn cards. In the
real app, with a temporary home so no configuration was touched: a yanked file made the System card
read `✓ fits · 22G left`, and `p` on it started the paste, which failed on `/` for lack of
permission as it should. The refusing cards were not shown in the real app, as this machine has
no small or FAT32 device; they are tested only.

---

### Wiring Diagram: The Machine as It Is Wired (COA C)

**Date:** 2026-10-07 · **Author:** deltaog-117

**Context.** The deck lists devices, but it shows this machine as a single card, "System", when the
disk is a stack: an NVMe drive in a PCIe slot, a boot partition, an LVM physical volume, an LVM
volume, an encrypted container and a btrfs filesystem on top. Nothing in the deck says that
`nvme0n1p2` is not a place to put files, or what has to be unlocked before `/` exists, or that two
partitions on a stick are one thing to eject.

| COA | Approach | Verdict |
|---|---|---|
| A | A diagram only: layers as columns of boxes joined by lines | Rejected: blank on a small terminal |
| B | An indented tree only, like `lsblk` | Rejected: reads as a list, not a diagram |
| C | A when the columns fit, B when they do not, both from one model | Chosen |

**Decision.** `devices::topology` builds a graph, not a tree: a RAID array has several parents.
Nodes are buses, disks, partitions, stacked devices and shares. They are read from
`/sys/class/block` (kind, size, the removable flag, the disk a partition is on), the kernel's
`slaves` links (what a stacked device is made of), device-mapper's own `name` and `uuid` (whose
prefix says LVM, LUKS or multipath), the udev database (label, filesystem, what it holds) and
`mountinfo` (where, and how full via `statvfs`). The bus comes from where the disk sits under
`/sys/devices`, so `PCIe 02:00.0`, `USB 2-3` and `SATA ata1` are read, not guessed. Each node has a
primary parent, the first, which is where a drawing hangs it; the others are kept. `wiring_layout`
is pure: columns by longest chain from a root, rows by walking the tree with each parent centred
on its children, and lines as marks on the four sides of a cell that a lookup turns into a
box-drawing character, so the junctions come out right wherever lines meet. If the columns do not
fit the window it lays out an indented tree instead. `wiring` is the state (a scan once a second on
the blocking pool, selection by node name so it survives a refresh, the keys), `wiring_view` the
drawing. The mount, unmount, eject and fit-check actions were taken out of the deck into
`device_actions`, so both views behave the same.

**Why this way.** One model and two renderers means the hard part, reading the machine, is done
once, and a narrow window costs one small extra renderer instead of a blank screen. The selection
moves the way the page is laid out: left to what a node is under, right to what is under it, up
and down in a column. Ejecting a node ejects the disk it is on, unmounting every volume on it first
and then powering it off, because that is what pulling a stick out means; the diagram makes it
visible that two partitions hang off one disk. Activity runs along the line into a busy disk as a
mark, drawn from the same counters the deck's sparkline uses, at a fixed pace so the picture shows
that data is moving rather than flickering with each sample.

**Found by the tests and the real machine.** A line leaving a box began as a stub (`╶`) and not
as part of the line, because the first cell had nothing to its left; the cells next to a box now
reach into it. Dropping an empty card reader left its USB port behind as a box with nothing under
it; a bus whose disks are all dropped goes too. On the real machine the stack, the sizes, the mount
and the 95% gauge all came out right, and the `/boot` partition the deck leaves out is shown.

**Trade-offs.** A node with several parents is drawn under the first only; the other parents are
in the model but not drawn as lines. LVM and LUKS layers can be seen but not opened or closed from
here, as `udisksctl` does not handle them the same way. There is no SMART data, no bus speed. A
window too narrow for the columns loses the lines and the marks. The marks move at one pace
whatever the rate. Btrfs over several devices appears as separate filesystems.

**Verification.** `scripts/check` passes (1377 tests). New: the topology built from a fixture that
reproduces this machine (bus, disk, two partitions, LVM, LUKS and btrfs, a USB stick, a share, a
RAID array), the bus rules on the path forms the kernel gives, the layout (columns, centring, no
overlap, the tree's branches including a trunk that continues past a deeper branch, connectors
at every junction, a click finding the node), the state's keys in both layouts, what `Enter`, `m`,
`e` and `p` do and refuse, the drawing at sizes down to nothing, the mark moving along the line and
not off it, and the fit verdict on both layouts. In the real app on this machine, wide and narrow:
the stack drawn, the marks moving on the busy disk, `h`/`l` through the stack, `Enter` on a
non-filesystem refused with a reason, and `Enter` on the root going to `/`. Not tried with a real
USB stick, RAID or SATA disk: those are fixtures only.

---

### Wiring Diagram: Unfolding the Folders (COA A)

**Date:** 2026-10-08 · **Author:** deltaog-117

**Context.** The wiring diagram ended at the mount point: a box saying `/` and a gauge. The user
wanted to click the directory and open the rest of the tree, so that the picture which begins at
the PCIe slot would not stop where the folders begin.

| COA | Approach | Verdict |
|---|---|---|
| A | Unfold inline: folders become more boxes to the right, joined by the same lines, read in the background, the path to the current folder unfolded on opening | Chosen |
| B | A side tray of Miller-style folder columns beside the diagram | Rejected: the diagram and the folders would be two separate things |
| C | Draw only the path from the device to the current folder, with siblings on demand | Used as the starting state of A, not on its own |

**Decision.** The folders are not hardware, and the hardware is rebuilt from the machine every
second, so they live apart: `wiring_folders::Folders` holds which folders are open and what was
read in each, and `compose` returns a copy of the hardware with the open folders added below their
filesystem. `WiringState` keeps `base` (the last scan) and `composed` (what is drawn and moved
through), and rebuilds the second whenever either the scan or a folder listing changes. `NodeKind`
gained `Folder` and `More` (the "+N more", "loading…" and "cannot read" notes), and `Node` a path.
Reading a folder is `Vfs::list_dir` on the blocking pool, keeping the directories, hiding dotfolders
unless the browser shows them. The layout gained boxes of different heights, a parent that is
centred on its children but never above where its family begins, and room enough for a tall box
over one short child. Opening the view calls `follow`, which unfolds the filesystem and every folder
down to the folder being browsed and chooses it, once for each folder; a folder with more than
twelve subfolders always includes the one on the way.

**Why this way.** Because the folders sit in the same graph as the disks, all of what the diagram
already does applies to them without a second code path: lines, selection, scrolling, the narrow
tree, the bright "you are here" frame. A copy rebuilt on change is cheap (tens of nodes) and keeps
every reader of the topology looking at one thing. Reading in the background with a note in the
meantime means a slow disk or a share never holds up the keys. Closing a folder forgets what was
read below it, so reopening it shows the disk as it is, with no refresh to build. A click now
unfolds and `Enter` opens in the browser, because a click that both explored and navigated would
make exploring impossible without leaving the view; this replaces "a second click opens". `p` on
a folder sends into it, with the fit check of the filesystem it is on, which closes the gap the
cargo-aware deck left (it could only send to a device's root).

**Found by the tests and the real machine.** A test that browsed `/nowhere` found that the view
rightly followed it, since `/` holds every absolute path; tests that want no following use a path
no mount holds. Another test would have started a real `udisksctl` through `e` on a removable
disk, so those now use the fixed disk. The view test for a deep folder found a real problem: the
chosen folder was on screen but the subfolders unfolded to its right were cut off at the window's
edge. The view now scrolls sideways far enough to show the chosen box and the column beside it.
In the real app, opened in this repository, the diagram unfolded from `/` down through `home`,
`Desktop`, `Projects` (15 folders), `Programming` (46) and `minuteman` to `crates` (12), `h` and `l`
folded and unfolded it, and `Enter` on the first folder moved the browser there.

**Trade-offs.** Only folders, never files. Twelve is a fixed limit, and the "+N more" note opens the
folder in the browser instead of unfolding in place. Open folders are forgotten when the view is
left. What was read is not refreshed while a folder stays open. A mount point inside a folder (a
`/boot` under `/`) is listed as a folder and also drawn as its own filesystem. The folders carry no
sizes, so the diagram cannot say which is large; the disk usage view is for that. A very deep path
makes a wide canvas that scrolls sideways.

**Verification.** `scripts/check` passes (1406 tests). New: the folder reader (sorted without regard
to case, files and hidden folders left out, nested folders under their parent, closing forgetting
what was read, an unreadable folder saying why, the cap and the way-down rule, an answer for a
closed folder dropped), the layout (short boxes in the next column, no overlap with a tall box over
one short one, lines joining boxes at their middles, the tree not taking over when folders run past
the window), the state (click unfolds and folds, right and left unfold, go and fold, following on
opening and once for each folder, `Enter` on a folder and on the note, `p` into a folder with and
without room, the selection surviving a vanished folder, `m` and `e` on a folder), and the drawing
(signs, counts, the bright frame on the exact folder, sideways scrolling keeping the chosen folder
on screen, the tree, the note). In the real app as described above.

## 2026-10-08 — Wiring diagram: zoom and fit

**Context.** With many folders unfolded the diagram was taller than the window and only scrolled.
The request was zoom in and out, or a full-size view that adapts to the number of folders.

**Decision.** Both, in one mechanism: the layout takes a `Scale` (four levels, the smallest a
borderless line a node). The default picks the largest level that fits the window; `+`/`-` set a
level by hand and `0` returns to fit. The renderer stores the body height in the state so keys and
clicks lay out exactly as the drawing did.

**Trade-offs.** A terminal cannot scale text, so zoom is a set of discrete box sizes, and the
smallest shows only names. If nothing fits the height it falls back to the smallest level and
scrolls. Three drawing tests were pinned to the full level, since they test the full-size and tree
layouts and a fitted zoom would now shrink them.

**Verification.** `scripts/check` passes. New layout tests: fit shrinks to the height, a level
chosen by hand is kept, and lines join boxes at every level.

### Atlas: a Zoomable Map of Storage (COA A)

**Date:** 2026-10-09 · **Author:** deltaog-117

**Context.** The roadmap held three ideas for a storage view; the deck (B) and the wiring diagram came
first. The third, the atlas, was to answer "where did my space go" as a picture: a block per volume
sized by capacity, zoom into a volume for its biggest folders, then into a folder for its files.

| COA | Approach | Verdict |
|---|---|---|
| A | A new `V` view; volumes from `devices`, every level below scanned on demand by the disk usage scanner, a stack of levels so zooming out is instant | Chosen |
| B | A: plus a cached whole-volume tree per device and a corner minimap in the other views | Rejected for this cycle: cache invalidation, a size budget and a change to every view in one go |
| C | Volumes only; `Enter` opens the disk usage view | Not taken on its own: it is level one of A |

**Decision.** Three modules. `atlas_layout` is pure: `treemap(weights, area)` turns sizes into
rectangles of whole cells, and `neighbor`, `cycle` and `tile_at` move between and find them.
`atlas` is the state: the volumes (sorted biggest first, polled every two seconds on the blocking
pool), the chosen one by kernel name so it survives a rescan, and, once zoomed in, a `DiskUsageView`
plus a depth counter. `atlas_view` is the drawing. Nothing new scans the disk: the view that already
scans one folder at a time on the blocking pool, cancels on drop and keeps a stack of levels is the
engine for every level below the volumes, and the atlas only maps its rows to tiles. The depth
counter is there because the view's own `leave` climbs to the parent of the mount point once its
stack is empty, which on the map would walk out of the volume.

**The layout, and why not squarified.** The first plan, and what the COA table said, was a
squarified treemap. A squarified layout picks rows by real-valued aspect ratios, and turning that
into whole cells means rounding each row's thickness and each tile's length, which can leave a tile
with no cells, so "the tiles cover the map exactly" holds only when the rounding happens to agree.
A balanced binary split does not have the problem: the entries (largest first) are cut into two
groups of about equal weight, the map is cut across its longer side (a cell counted as twice as tall
as wide) at the matching integer position, clamped to leave a cell on each side, and each half is
laid out the same way. Every tile is at least one cell and the tiles partition the map by
construction. The price is blocks that are somewhat more elongated than a squarified layout gives.
A group worth fewer than six cells of the map is folded into one `+N smaller` block (also when the
map is too small to cut), which keeps a thousand specks from becoming a thousand one-cell tiles.

**Why this way.** The folded block is drawn but never chosen, so reachability needs its own
guarantee: arrows go to the nearest centre in that direction, which can leave a block out of reach
of the four directions in an odd layout, so `Tab` and `Shift+Tab` cycle through every block with a
tile in order, and a property test checks that cycling forward visits all of them. Colours are the
theme's own (`accent`, `danger`, `dir`, `file`, `status`), used as backgrounds, with the text
colour chosen by the brightness of what it sits on and a darker shade made by blending to black
where the colour is RGB (and dark gray where it is not). The chosen block gets a thick frame inside
its own edge rather than a different fill, because a volume's fill already encodes how full it is.
A block leaves its right column and bottom row bare when it is big enough, so two neighbours of one
colour stay apart.

**Found by the real binary.** A scan shows only what has finished: for the first seconds on the root
volume the map held the four links in `/`, and then `var/` at 99% once it finished, with `usr/`
arriving much later and changing every share. That is the scanner doing what it was built to do, and
the header says `scanning`, but it does make the early picture misleading, and it is listed as a
follow-up. The same run showed the project's known PTY quirk, a keystroke swallowed right after
startup by the graphics probe's reader, which a harmless `Esc` first works round. A test written
before the run expected `90%` for a volume with 800 of 900 bytes used, which is 88%: the test, not
the code, was wrong. The header said `1 volumes`, now `1 volume`.

**Trade-offs.** No cache and no minimap, so zooming into a big volume rescans it every time (the
root volume here took minutes to finish) and leaving it throws the scan away. Blocks under six
cells of the map are only a `+N smaller` block, and files in it are reached through the browser or
the disk usage view. Opening a file's block goes to its folder without the cursor on the file.
Unmounted volumes cannot be mounted from the atlas. Linux only.

**Verification.** `scripts/check` passes (1450 tests). New: the treemap (the tiles partition any map
for any sizes, every sized entry is accounted for exactly once, a block of its own deserves its
cells, a lone entry fills even a one-cell map, areas follow the sizes, a tall map is cut across its
height, moves land strictly in the direction pressed and cycling reaches every block), the state
(volumes biggest first with ties by name, the choice starting on the volume holding the browsed
folder and surviving a rescan, an unmounted volume saying why, zoom from volume to folder to file
and back out one level at a time, going out never climbing past the mount point, `o` on volumes,
folders and files, the pin following the browsed folder down into the rows, `a` and `r` belonging
to the atlas only once zoomed in, arrows and `Tab`, a click choosing and a second click zooming,
other keys passing through) and the drawing (the regions never overlap or leave the window, the
free part of a volume shrinking as it fills, text colours on light and dark backgrounds, labels and
shares, the frame on the chosen block, the empty states, and no panic for any window or number of
volumes). Against the real binary through a PTY read with `pyte` and `HOME` pointed at a scratch
folder: the volume map, `Enter` into `/` with rows arriving, `Enter` into `/var`, `Backspace` twice,
`o` moving the browser to `/var` with the pin following, and a real SGR mouse click zooming in.

### Wiring Diagram: Fitting the Whole Diagram in the Window (COA A)

**Date:** 2026-10-09 · **Author:** deltaog-117

**Context.** After the folders could be unfolded, a deep path made the diagram much larger than
the window: a column for every level and every unfolded sibling stacked down the page. The fit only
required the *hardware* to fit the width, so the view panned to the chosen box, the hardware went
off the left edge, and boxes not wholly inside the window were skipped while their lines were still
drawn: "some things appearing and others not". Reproduced in the real binary with this repository's
own path (`crates/tui`) at 150×40: a slice from the middle of the canvas, `Vida` at the left edge,
dangling lines, the hardware gone.

| COA | Approach | Verdict |
|---|---|---|
| A | Fit by budget: a cap on the siblings each folder shows and a squeeze on the box width, chosen so the whole diagram fits both ways | Chosen |
| B | Focus and context: columns and siblings far from the chosen node collapse | Rejected: the layout would move with the cursor, so the picture would jump as you navigate |
| C | Keep panning, but draw clipped boxes and pin the hardware | Its one real fix (clipped boxes) is in A; as a whole it still does not fill the screen |

**Decision.** `wiring_layout::fit(compose, width, height, zoom)` tries a cap on siblings (12, 8, 5, 3)
and, within each, the zoom levels from largest to smallest, and returns the first topology and
layout whose whole canvas fits. The cap changes which nodes exist, so `Folders::compose_capped(base,
cap)` builds the topology for a cap and the fit returns the topology with the layout; `WiringState`
keeps the pair (`composed`, `fitted`) and rebuilds it whenever the folders, the hardware, the zoom or
the body size change. The size comes from `set_size`, which `main` calls every turn before `poll` so
the first frame is already fitted; before a size is known (and in the many older tests) the state
behaves exactly as before, which is why `layout(width)` keeps its signature and ignores `width` once
fitted. A level's box width is squeezed to the columns (`Scale::fitted_to`): the window's width
shared out over the columns, never below 10 cells and never past one and a half times what the level
draws, and if even 10 does not fit the columns the diagram becomes the tree.

**Which siblings are cut.** Siblings further from where you are go first. `pick` keeps every name
on the way to the browsed folder and to the chosen one, then fills the rest of the cap with the
names nearest to those (earlier on a tie). It used to take the first twelve and swap the last for the
one on the way, which for a folder of 46 showed `1ytunes … copycat111 … minuteman`. The chosen
folder is taken from the selection's node id (a folder's id is its path), so choosing a far sibling
and then shrinking the window does not trim the cursor away.

**Why this way.** The result is deterministic given the size, which keeps the picture steady while
moving through it and makes "fits" a property a test can state for any window: the property test
asserts that the fitted layout holds the whole diagram whenever any cap and level could, and
otherwise uses the fewest folders and smallest boxes. Every cut is visible as the existing "+N more"
note, so nothing is dropped silently. Smaller boxes are preferred over fewer siblings (the loop is
over caps, then levels), so a deep path in a short window comes out as one-line boxes with every
sibling up to the cap rather than big boxes with three siblings each; that was a choice, and the
other order is a small change in `fit`.

**Found by the real binary.** The first fitted run drew the tree at 150 columns (fourteen columns do
not fit at ten cells each) but its deepest names were cut to `…`: the tree's name column was a fixed
40 cells and the indentation of nine levels took most of it. It now grows to the longest row, up to
three fifths of the width. A test for clipped boxes written from the first idea (look for a frame
character in column 0) failed on a correct drawing, because a clipped title row can legitimately be
blank there; it now looks for any drawn cell in the visible slice. Another test expected the full
text `PCIe 02:00.0`, which a squeezed box rightly cuts to `PCIe …`.

**Trade-offs.** At the narrowest, names are cut to a few letters, and the tree is the alternative
when the columns need more than the window has. A window too short for even the smallest cap and
level still scrolls, as before. Each refit composes the topology up to four times; it runs on a
change of size, zoom, folders or hardware, not per frame, and the topologies are tens to hundreds of
nodes. Open folders are never closed to make room, only trimmed.

**Verification.** `scripts/check` passes (1466 tests). New: the cap on `compose` and the choice of
which names survive (counts, the note's number, the chosen folder kept, nearest siblings first, more
anchors than room), `fitted_to` (stretch limit, minimum, exact boundary, and a property that the
columns fit the width), `fit` (boxes when the window is wide enough and the tree when it is not, fewer
siblings and a visible note in a short window, the fallback when nothing fits, a level chosen by
hand kept, and a property over any window, depth, siblings and zoom), the state (the fitted layout
whatever width is asked, a zero size leaving everything unfolded, `-` and `0` refitting) and the
drawing (the hardware and the deep folder on screen together, fewer siblings in a shorter window and
the browsed one kept, growing back, a chosen folder surviving a shrink, cut boxes drawn, any size
without a panic). In the real binary as described above.

### Operation Keys: Marks Act Where You Stand (COA A)

**Date:** 2026-10-10 · **Author:** deltaog-117

**Context.** Every operation used to need `v` marks and ignored the cursor row, and `y`/`d` only
filled a clipboard that `p` then had to drop. That made a single file cost three keys and a batch
cost mark, act, move, paste. The request: a bare `y`, `d`, `x` or `X` should act on the row under the
cursor, and with marks the key should itself do the work where you are standing.

| COA | Approach | Verdict |
|---|---|---|
| A | Marks present: yank/cut copy or move them into the browsed folder at once. None: the cursor row goes to the clipboard (yank, cut) or to the confirmation (trash, delete) | Chosen |
| B | Only add the cursor fallback; marks still go through the clipboard | Not taken: keeps the extra step that was the complaint |
| C | A, but marks already in this folder fall back to the clipboard | Not taken: the behaviour is explicit in A, and a same-folder cut is told "already in this folder" |

**Decision.** `App::yank_key` and `App::cut_key` are new and used by the keys only; `yank` and `cut`
stay clipboard-only because the right-click menu (which marks the clicked row first, so a menu "Cut"
would otherwise move it onto itself) and the deck and wiring `p` rely on them. The immediate path
reuses `begin_drop`, the background paste a drag and drop already uses, so progress, cancel, the
overwrite/skip/abort prompt and undo came free, and the clipboard is left alone. `begin_trash` and
`begin_delete_permanently` switched from marks-only to marks-else-cursor, which also changes `:trash`.
Compress and extract were left marks-only. The keys keep their places: `d` cuts and `c` cancels.
Confirmations take `y` and `Y`. `space` with no pane open
is now a pending leader whose `x` opens the trash and `t` the settings popup: that `t` branch was
dead code, because the pending leader was reset on every tick when no pane was open, so the reset
no longer clears it. The mini-shell panes were weighed for removal as immature and kept as they are, at the
author's choice; the leader gaining a use outside them is what the change did for them.

**Trade-offs.** A `y` with marks in the same folder is a copy onto itself, which `file_ops`
rejects as the same path, so nothing is duplicated or overwritten (a test pins that); the old
`v`, `y`, `p` there behaved the same. One key has two behaviours depending on whether anything is marked, which is the cost of
dropping the clipboard step.

**Verification.** `scripts/check` passes. New tests: the cursor row for `y`, `d`, `x`, `X`; marks
winning over the cursor; `y` and `d` into another folder, for a copy and a move, leaving no
clipboard and no marks; a same-folder cut keeping its marks; a capital `Y` for both confirmations;
the leader's hints with and without a shell. In the real binary under tmux:
two marked files cut into another folder, `X` then `Y` on the cursor row, `c` clearing a mark,
`space` then `x` opening the trash.

## 2026-10-10 — Deck view: the disk tray moves to the left

**Context.** The deck view put its devices in a strip of framed cards above the columns. The
request was a tray like Dolphin's or Thunar's, at the left, with a look of its own, and in this
view `hjkl` to browse and `Enter` (not `l`) to open a file or enter a folder.

| COA | Approach | Verdict |
|---|---|---|
| A | A places-style rail: one frame, devices under group headings, two lines each | Chosen |
| B | The existing three-line cards stacked in a column | Not taken: a pile of boxes, five rows a device |
| C | An icon rail that unfolds when it has the keys | Not taken: hover and fold state for little gain |

**Decision.** `browser_layout` gives the deck a rail of `RAIL_WIDTH` cells and lays the columns out
in the rest, using `BrowserLayout::split_columns`, which is the old column split lifted out of
`split_spaced`. `BrowserLayout::deck` keeps its name and now means the rail. `deck_view::rows` is
the one function that drawing and clicks both ask: it flattens the devices (already sorted by kind
by the scan) into headings and devices with their heights, picks the first line to show with a
pure `follow`, and stores it in the `Cell` the cards already used. A rail under 70 cells of body
width or 6 rows is dropped, so the keys must know whether it is there: `DeckState::key` now takes
`visible` and passes everything, unfocusing, when it is not. Keys: the first cut had `h` go left to the rail and `l` come back, with `l` doing nothing on the
files so `Enter` opened. That broke the habit every other view relies on, and the user asked for
`hjkl` back as they were. The rail now takes only `Tab` (toggle), the up and down arrows, `Enter`,
`m`, `e`, `p` and `Esc`; every other key, `hjkl` and the left and right arrows included, is the
browser's whether or not the rail has the keys, as the old deck did for `j`/`k`.

**Trade-offs.** A device's row is two lines, so the sparkline shrank to a three-bar
pulse shown only while the disk is busy and the mount path left the row. The rail takes 26 cells
from the columns.

**Verification.** `scripts/check` passes. Tests for the layout, rows, scrolling, clicks, keys and
drawing at any size; the rail was also drawn through the test backend and read. Not run in the real
binary: no terminal emulator was at hand to read a PTY.

---

## 2026-10-10 — Browser file chooser: `mman` behind the desktop portal

**Context.** A browser's "Save as" and upload dialogs come from the XDG desktop portal, not from the
file manager set as the default for folders. To have `mman` answer them, something has to speak the
portal's backend interface.

| COA | Approach | Verdict |
|---|---|---|
| A | `xdg-desktop-portal-termfilechooser` plus a wrapper script that opens `mman` in a terminal | Not taken: works at once, but leans on a third-party daemon and a shell script, and passes few of the portal's options through |
| B | Our own backend, `mman-portal`, a small D-Bus service that opens `mman` in a terminal | Chosen |
| C | `xdg-mime default … inode/directory` | Not taken: it only decides what "show in folder" opens; save dialogs never ask it |

**Decision.** Three parts. `shared::chooser` holds the contract both sides use: `PickKind`
(file, files, directory, save) and `Selection`, a non-empty list of absolute paths that can only be
built by `try_new` or `decode`, written as paths each ended by a NUL (the one byte a path cannot
hold, so a newline or invalid UTF-8 in a name survives) with an empty file meaning "backed out".
In `mman`, `--chooser-file` turns on a pick mode (`tui::chooser`) that changes only what two keys
mean: opening a file picks it (the single `open_file` function every Enter, double-click and
menu Open goes through, so none of them can launch the file by accident), and `Q`, which already
meant "finish here", confirms where the cursor is. A save is a `Prompt::SaveName` followed, if the
file exists, by `Prompt::ConfirmReplace`; the name is checked by `save_target`, which cannot leave
the folder on screen. `Q` was reused instead of a new `Action` so the key map, the config and the
HUD stay as they are. The new crate `portal_backend` exports
`org.freedesktop.impl.portal.FileChooser` and, per call, a `Request` object whose `Close` kills the
window. One call makes a private `0700` directory, starts `<terminal> … mman --chooser-file
<dir>/answer …`, waits, and reads the file: no file (window closed) is a cancel, an empty file is a
cancel, a file that is not an answer is a failure (response 2), never mistaken for a cancel.
`SaveFiles` picks a folder and names each file inside it, dropping any name that is not one plain
component.

**Trade-offs.** `mman` is a terminal program, so the backend still needs a terminal emulator; B
saves the wrapper script and gains the window class, title, cancel on `Close` and a typed reply,
not the terminal. The terminal has to stay in the foreground until the command ends, so ones that
hand the window to a server process and exit (a `gnome-terminal`, a ghostty with a running
instance) are left out; `MINUTEMAN_PORTAL_TERMINAL_CMD` takes any command that does wait. The
dialog's filters, extra choices and parent window are ignored. `scripts/vfs-gate` gained two
entries (`portal_backend/src/session.rs` and `terminal.rs`), because the daemon and `mman` talk
through a scratch file on this machine and the daemon looks for a program on this machine's
`PATH`; the chooser's write went into `cli.rs`, next to the `--cwd-file` one, with no entry of its
own.

**Verification.** `scripts/check` passes. Property tests: a `Selection` round-trips any bytes a
name can hold and decoding never yields a relative or empty pick; a file URI reads back as the same
bytes; a save target and a `SaveFiles` name never leave their folder. An end-to-end test starts a
private `dbus-daemon`, the real `mman-portal` and a stand-in `mman` script and calls
`OpenFile`, `SaveFile` and `SaveFiles` and `Close` over the bus (pick, cancel, window closed,
garbage answer, withdrawn request). The real `mman` binary was driven through a scripted PTY in
all four modes (a file, two marked files, a folder, a save with a suggested name, a save over an
existing file with `y`, and `q`) and the answer files read back byte for byte. Not run: a real
browser, the real `xdg-desktop-portal`, or a real terminal window; those are the next thing to try.

---

## 2026-10-11 — A small core and bundled plugins: the scaffold (M1)

**Context.** The aim is for the file operations (with the trash they go to) and the terminal
interface to be the only essential parts, and for the finder, search, preview, archives, devices
and the rest to be replaceable by a plugin, and shipped by default the way a browser ships its
system add-ons. The plugin protocol already let a plugin run the file operations and the
interface's actions, but not show anything, claim a key or take a built-in's place. The roadmap had
listed "everything is a plugin" as not planned, on the grounds of a 100k-line rewrite, an API as
big as the app, and serialization on hot paths.

| COA | Approach | Verdict |
|---|---|---|
| A | In-process traits and a cargo feature per feature crate | Not taken alone: fast and typed, but replacing a feature needs a rebuild |
| B | Every feature, defaults included, over the plugin protocol | Not taken as a rewrite: fully replaceable, but a pipe on every hot path and one very large API |
| C | In-process traits for what is hot, and the same slots exposed over the protocol so a plugin can take a name | Chosen, and moved one feature at a time |

**Decision.** The core is `shared`, `file_ops`, `trash`, `browser` (the directory model), `tui`
(including the mini-shell panes and the shell popup, which are kept), the plugin host, the config,
and a plain open. Everything else becomes a bundled plugin in M2 to M7 (the finder first, because it
forces the picker and key-claiming design the others reuse), each move measured against what it
replaces and leaving behavior unchanged. The earlier objection stays honoured by never doing it as
one rewrite and by keeping the browser, the operations, the trash and the shell out of it. M1 is
the scaffold only: `crates/plugins` became `crates/plugin_host` so it no longer shares a name with
the root `plugins/` folder for the bundled plugins; `crates/plugin_api` was cut out of
`protocol.rs` with the plain wire data, which the host re-exports under the old paths so no
caller changed except `tui`'s `use plugins::` lines; and the dependency-direction rule became a
test over `cargo metadata`.

**Trade-offs.** `plugin_api` depends on `shared` because `WireEntry` converts from `shared`'s
`DirEntryInfo`, and Rust's orphan rule puts that conversion in one of the two crates. The line
parser (`PluginRequest`, `parse_line`) and the event builder stay in the host because they use
`file_ops::ConflictPolicy` and `crossterm::KeyCode`, which a plugin must not need; splitting them
is M2's work, when a plugin first needs a client. The workspace does not list `plugins/*` yet,
since Cargo rejects a glob that matches nothing, so each plugin crate adds itself. The moved types
now derive both `Serialize` and `Deserialize`, so a Rust plugin can read what the host writes.

**Verification.** `scripts/check` passes (format, `vfs-gate` now scanning `plugins/` as well,
clippy with warnings denied, the whole workspace's tests). New tests pin the JSON spelling of every
moved type and round-trip a `StateSnapshot` as a property; `dependency_direction` runs the rules on
the real workspace and on made-up workspaces that break each one, so a rule that stopped firing
would fail. No behavior changed, and no interface was run by hand for this reason.

---

## 2026-10-11 — The fuzzy finder becomes a bundled plugin (M2)

**Context.** The finder was the first feature to move out of the interface because it forces the two
things every later one needs: a plugin that takes the place of a built-in action, and a list the
interface draws for a plugin. What made it more than a move was that its walk goes through
`Vfs::list_dir`, so it works over ssh and inside archives, while the plugin protocol's file
operations always read the local disk.

| COA | Approach | Verdict |
|---|---|---|
| A | A generic picker; the plugin walks the tree itself with `read_dir` and ranks | Not taken: a round trip per folder, every finder plugin redoing hidden-file rules and loop protection, and the local-disk-only `read_dir` |
| B | A generic picker plus a `walk` request the host runs through the browser's own `Vfs` and streams to the plugin in batches; the plugin ranks | Chosen |
| C | The finder in-process behind a `PickerProvider` trait, external plugins using the same slot | Not taken: no process boundary, and `tui` would depend on `plugins/`, against the rule that the core never names a plugin |

**Decision.** Three kinds of message, all in `plugin_api`. Providing an action: `provides` on a
`[[plugin]]` makes the key bound to that action send the plugin an `action` event (with the folder
and whether dot-files show) and do nothing else; the first plugin configured with the name gets
it. The picker: the plugin asks for one with `show_picker` and chooses its number, sends
`picker_items` (the best results, the counts, whether the walk is still going, whether it has
caught up), and hears `picker_query` (with a generation number) and `picker_closed`; the interface
drops an answer to any query but the newest, so typing never shows stale results. The walk: `walk`
runs `browser::finder::walk` on the blocking pool through the same `Vfs` the browser reads
through, sends each folder's entries as `walk_batch` (at most 1,000 to a line) and ends with
`walk_done`; `walk_cancel` or the process ending stops it. The plugin picks the numbers, so it
never waits on a reply to know which picker or walk an event is about.

In the interface, `finder_popup` and `finder_view` became `picker_popup` and `picker_view`, which
hold what was typed and the newest results and draw the panel under the plugin's title; the engine
that ranked off the render thread (`finder_job`) is gone from `tui`. The ranking moved to
`plugins/finder` unchanged, with its tests (including both property tests), and the matcher it
and the browser's `/` search share, `Query`, moved to `shared` so a plugin can use it without
depending on `browser`. The picker's panel can drop the preview pane (`preview = false`), which
needed `trash_view::layout_with`. The finder plugin is found next to `mman` or on `PATH`
(`tui::bundled`) and starts at launch unless a plugin of the same name or providing `find` is
configured. When it is not found `f` says how to install it, as the user asked, rather than
falling back to the old in-process finder.

**Trade-offs.** The plugin starts at launch as an idle process rather than on first use; lazy
start is a follow-up and the roadmap says so. `plugin_host` now depends on `browser`, for the
walk, which keeps the rule that the core never depends on a plugin but widens what the host
knows. A picker closed by choosing a result always means "show it in the browser"; letting the
plugin say what choosing does waits for a second plugin that wants something else. A plugin that
dies while its picker is open closes the picker, and its own exit line is what the status bar
shows. The host and the plugin each count generations, so a restart of the plugin (a crash and the
next `f`) starts clean with nothing carried over. The event loop wakes every 16 ms while a picker
is open or has just been asked for, not every 100 ms, since its results now arrive on the
plugin's schedule.

**Verification.** `scripts/check` passes. The ranking's tests moved with it; new tests cover the
finder's session (opening asks for the picker then the walk, a refusal puts it away and says why,
stale numbers are ignored, closing cancels the walk), the real `mman-finder` binary on pipes (open,
rank, narrow, close, and a host speaking another protocol version), every new message's JSON, the
host's side of each (an action reaching only its provider, a provider that cannot start being
reported, a picker opened and answered through the interface, a refusal reaching the plugin, a
walk over a real folder with and without dot-files), the picker popup and its panel, and the
bundled lookup. `dependency_direction` still holds. The real `mman` was driven through a
pseudo-terminal with `mman-finder` beside it: `f` lists every entry, typing narrows to the two
`main` files, a second result chosen with `Enter` leaves the browser in `docs/guide`, `Esc` closes
and `f` opens again; with `mman` copied somewhere alone `f` says how to get the finder and opens
nothing; with the finder killed while open the picker closes and `f` starts it afresh. Timing, 120,300
entries in 300 folders, release builds, three runs each (index complete after `f` / a typed query
ranked after its last letter): 0.68 s / 0.08 s before, 0.66 s / 0.04 s after. Not run: a remote
backend or an archive through the plugin (the walk uses the same `Vfs` the browser does, and the
walk's tests use an in-memory one), and a terminal other than the pseudo-terminal.

---

## 2026-10-11 — Structured search as a bundled plugin (M3)

**Context.** The roadmap's M3 said only "recursive search on the same picker slot". The first
reading, moving `/` onto the plugin, was rejected: `/` is part of what Minuteman is and stays in
the core, and the `f` finder stays the one plugin on that key. So M3 had to add something
neither does. `/` is a nearest-level fuzzy match on names, `f` is a fuzzy match on names and
paths; what is missing is matching by pattern and by what a file is (size, age, kind).

| COA | Approach | Verdict |
|---|---|---|
| A | A content-search plugin (`grep`) streaming matching lines | Not taken now: `walk` lists but cannot read, so it needs a host call to read file bodies, and over ssh and archives it is slow |
| B | A name-and-metadata search plugin: glob, regex, `size:`, `mtime:`, `type:` | Delivered first |
| C | B with a query grammar that leaves room for a content mode | Chosen: C, shipped as B |

**Decision.** `plugins/search` is a second bundled plugin, `mman-search`, providing a new action
`locate` bound to `L`. (`F` was the first choice; the keymap test uses `F` as the probe for "a key
no default binds", and the test was right, so the default moved instead.) The tree below the
folder comes from the same host `walk` the finder uses, now asked for with `metadata: true`:
`TreeEntry` gained optional `size` and `modified` and `WalkRequest` an optional `metadata`, filled
from the `size` and `modified` the listing already returns and `browser::finder::Candidate` used to
drop. A folder never carries a size, and a time before the epoch is dropped. With the switch off
the lines are byte-for-byte what they were.

The plugin is `query` (the grammar: every word is a typed term, `Span` is an inclusive pair that
cannot be empty, a glob is translated to an anchored regular expression and a literal is escaped,
all compiled once per query), `scan` (checks the growing index a slice at a time and keeps the
first 200 matches in index order, which is nearest level first, so nothing is sorted), `session`
(the finder's state machine with the scan in place of the ranking) and `run` (a copy of the
finder's driver loop). A word that cannot be read never fails the query: `Search::parse` returns
it in `ignored`, and the session reports a given set once rather than on every later letter.
Matching rules chosen: lower-case words ignore case, a capital makes them exact (a rule for
literals and globs only, since `\D` in a regular expression would trip it, so `re:` is case
sensitive and `(?i)` turns that off); an entry missing the data a filter needs does not match it;
`mtime:<7d` means changed less than seven days ago.

In the core: `Action::Locate` and its key, `begin_locate` sharing `begin_picker_action` with
`begin_find`, and a second row in the bundled table. Nothing in `crates/` names the plugin.

**Trade-offs.** `run.rs` is a copy of the finder's loop, not a shared one; extracting it waits
for the third plugin, as it would otherwise move the finder in a change about something else.
Words cannot contain spaces. The status bar says `ignoring size:` while a filter is still being
typed, which is accurate but noisy. Content search is not here and needs its own design for
reading a file through the host.

**Verification.** `scripts/check` passes. New tests: 33 in `search` (the grammar word by word,
the property that an added word never adds a match, that parsing never loses a word, that a path
typed as a word finds itself, that slicing the scan never changes the answer, the session's
opening, filtering, one-time reporting, refusals and stale numbers), 4 with the real
`mman-search` binary on pipes, the host's metadata mapping, the wire's optional fields, and the
bundled table with two plugins. Measured on 200,000 entries in a release build: a query takes 1
to 15 ms in all, the longest slice of 2,048 entries under 0.2 ms. The real `mman` was driven
through a pseudo-terminal in a folder of four entries: `L` opened a `locate` panel, typing
`size:>1k *.rs` narrowed it to the one file over a kilobyte (`1/4`) with its hex preview, and the
status bar named the half-typed `size:`. Not run: a remote backend or an archive through the
plugin (the sizes there depend on what those listings return), and a terminal other than the
pseudo-terminal.

---

## 2026-10-11 — Slots a plugin fills: `set_view`, and git status on it (M4a)

**Context.** M4 asks for features that decorate the interface (preview, git status, the system
HUD, marked size, inspect) to be plugin-provided. M2 and M3 only gave a plugin a picker, which is
a modal list. Nothing let a plugin put text in the status bar or the header. The git status was
chosen first because it is the smallest of them: a pure parser, one background job, and two
segments of the status bar.

| COA | Approach | Verdict |
|---|---|---|
| A | `set_segment` only: styled text for named status and header slots | Rejected: it is B without the pane half, so the preview would need a second mechanism later |
| B | One `set_view` primitive: a document of styled lines bound to a slot | Chosen |
| C | Move the whole preview pipeline first, through `set_pane` | Rejected as a first step: the preview is scrolled, streamed, hooked and has images, so "behavior unchanged" would be hard to show |

**Decision.** `plugin_api` gained `set_view { slot, view }`. A view is lines of spans; a span has
text, a tone and an optional glyph named rather than spelled (`branch`, `ahead`, `staged`, ...).
The plugin cannot send a color or a symbol: the user's glyph set (`unicode`, `nerd`, `ascii`)
decides what a glyph looks like, and where the set has none nothing is drawn, which is how the
ASCII set loses the branch symbol as it did before. The interface keeps one owner per slot; the
last plugin to fill it owns it, only the owner can empty it, and a slot whose plugin stopped is
emptied on the next tick, because a segment nothing can update is wrong rather than merely stale.

The text of a view comes from a separate program and ends up in the user's terminal, so
`SetView::checked` runs on every one the host reads: control characters (an escape in a name
would reach the terminal) and direction-changing ones are removed first, then the limits are
applied to what would be drawn. A property test covers that the cleaned text never holds either
kind, that cleaning twice changes nothing, and that the limits hold.

`plugins/git` is `tui::git_status` moved over: the same parser (its property tests came with it),
the same `git status --porcelain=v2 --branch -z` with no optional locks, the same 10 s kill, 3 s
refresh and cancel on leaving the repository. What changed is who drives it. The old code was polled
by the render loop with the browser in hand; the plugin is told about the folder and the cursor by
events and has to ask once for the cursor at start, so `client::Incoming::Reply` now carries the
reply's `result`. All the rules about when to run and what to send live in a `Session` that is given
the time and returns effects, so they are tested without running git: a repeat answer sends nothing,
leaving the repository empties both slots before any run answers, a run for a folder left behind is
cancelled and its late answer ignored, and a failed run keeps the last good answer.

**Two things that were wrong on the way.** The first draft of the bundled table's `replaced` rule
treated a configured plugin as standing in for a bundled one when it provided every action the
bundled one does. The git plugin provides none, and "every one of none" is true, so any configured
plugin would have switched git off. It now compares by name alone when there is nothing provided to
compare. Separately, my first PTY check killed its own shell: `pkill -f mman-git` matches the
command line of the shell that was running the check, which contained that text. It uses `-x` now.

**Trade-offs.** The state word of the selected entry trails the cursor by one round trip to the
plugin (a few milliseconds locally) where the old code answered in the same frame. A status block
is one color, so only the first span's tone shows; per-span tones will matter for panes. A missing
`mman-git` is silent, unlike the finder and search, which say how to get them when their key is
pressed, since nothing is pressed here. `git_status = false` still works, now by not starting the
plugin.

**Verification.** `scripts/check` passes. New tests: 29 in `git` (the moved parser and its
properties, the views for a repository and an entry, the session's rules above, the driver on
in-memory pipes), the wire shape and limits of `set_view` in `plugin_api` (four of them
properties), its parsing in the host, slot ownership and the glyph sets in `tui`, and the
bundled table with a third plugin. The real `mman` was driven through a pseudo-terminal in a
scratch repository: the right segment showed `main ~1 ?1`; a clean file had no state word and a
modified and an untracked one said so; `:cd` out of the repository removed both and `:cd` back
restored them; killing `mman-git` removed its text; with `git_status = false` no `mman-git`
process existed; and an `mman` copied alone with no plugin beside it browsed normally. Not run: a
remote backend (the plugin runs `git` on the local disk, as the old code did), and a terminal
other than the pseudo-terminal.

---

## 2026-10-11 — The header pill slot, and the system figures on it (M4b, first part)

**Context.** M4b lists four things: `header_pill`, `entry_badge`, a text `preview_pane` and
inspect. Of the two slots, the marked-size pill named in the roadmap cannot leave the core: the
paste checks use its total to see whether the files fit on the target disk, so a plugin would be
asked for a number the core needs to be sure of. The system figures (uptime, load, memory) are the
other pill the header draws, are off by default, and need nothing from the browser, so they were
the first to move and the right size to prove the slot with.

| COA | Approach | Verdict |
|---|---|---|
| A | Slots only (`header_pill`, `entry_badge`), the pane in a later milestone | Rejected |
| B | Slots first, then a text-only pane, images left for later | Chosen: this entry is its first step |
| C | The pane first, with the image hand-off | Rejected: the riskiest design first |

**Decision.** `Slot::HeaderPill` holds one line. The header draws it as a pill after the ones it
makes itself, in the tone of the first span (a normal tone keeps the old color of the system
pill). `plugins/hud` (`mman-hud`) reads `/proc/loadavg` and `/proc/meminfo` every two seconds and
sends the pill only when its words changed; the uptime is the plugin's own, which starts with the
session. `system_hud = true` still turns it on, now by starting the plugin, and the bundled table
gained a fourth row that provides nothing and follows no event. `tui::system_hud` is gone.

**Trade-offs.** Without `mman-hud` there is no pill and nothing says so (the same silence the git
plugin has). The size formatter is copied into the plugin, about 25 lines, rather than moved into
`shared`, so the plugin depends on `plugin_api` only; the two will drift if one changes. The
property test on the copy found that `format_size` prints six cells above about 1000 PiB, which
the original does too and no memory size reaches; the test is bounded there and says why.

**Verification.** `scripts/check` passes. New tests: the slot's wire name and line limit, the
plugin's parsing, formatting, summary and driver (on in-memory pipes), the bundled table with a
fourth row, and the header with and without a filled slot. The real `mman` was driven through a
pseudo-terminal: with `system_hud = true` the header ended in `up now  load 5.35 3.35 2.11  mem
7.4G/15G` and one `mman-hud` ran; with it off there was no pill and no `mman-hud`; an `mman`
copied alone showed no pill and browsed. Not run: a non-Linux host, where the pill would be the
uptime alone.

---

## 2026-10-11 — The entry badge slot, and a mark beside changed files (M4b, second part)

**Context.** The slots so far each hold one view of one to three lines; none belongs to a row.
`entry_badge` is a mark beside every changed entry of the browsed folder, so the wire needed a way
to say which entry a mark is for, and the git plugin, which already knows every path's state
(`Repo::state_of`), was the obvious first user.

| COA | Approach | Verdict |
|---|---|---|
| A | A separate `set_badges` notification with its own map and rules | Rejected: a second path beside `set_view` to own, clean and limit |
| B | Reuse `set_view`: `Slot::EntryBadge`, with an optional `key` (the entry's name) on `Line` | Chosen |
| C | The host asks the plugin for the badges of the visible rows | Rejected: a round trip per scroll, so marks would lag a frame, and request plumbing in the draw path |

**Decision.** A line of the `EntryBadge` view is one badge and names its entry in `key`; the host
indexes the keys once when the slot changes (`Slots::badge`), so a row's lookup per frame is a hash
probe, not a scan. The one-owner rule, cleaning, and "a plugin that stops loses its slots" all
applied unchanged. The per-view totals (64 spans, 512 characters) would have refused any folder
with more than a few dozen marks, so `EntryBadge` is bounded one badge at a time instead: at most
2,000 lines, one span of at most 2 characters each, a key of at most 255 characters, and a missing
key is an error rather than a line nobody can find. `mman-git` sends the badges of the folder it
knows the browser is in (`Repo::states_in`, sorted by name so an unchanged status is an unchanged
view and nothing is re-sent); a folder shows the merged state of what is inside it, as the status
bar's word already did.

**Departure from the plan.** The plan said the host would drop the badges on `dir_changed`. It does
not: if it did, the plugin's "send only when different" check would believe the host still held
them and never send the next folder's marks again when the set happened to be equal. The plugin
owns the slot, replaces it when it moves, and empties it when the folder has nothing to mark or is
not in a repository. The cost is the same one the status word has: for the length of one round
trip after changing folder, the old folder's marks can sit beside the new names.

**Trade-offs.** A staged file changed again since shows the modified mark, as one span cannot hold
both. The name column loses 3 cells (a space and two for the badge) in every row once any row has a
badge, so names stay aligned; it only does so when the name column stays at least 12 cells.
Badges are drawn in the middle column only, not the parent or preview lists, nor the grid, tile or
filmstrip views. An unfound `mman-git` still says nothing.

**Verification.** `scripts/check` passes. New tests: the slot's wire shape and bounds (including a
property test that no checked badge breaks them, and that control characters are removed before
measuring), the host's index (later line wins, only the owner can empty, a replaced or stopped
plugin leaves no badge), `Repo::states_in` (direct children and folders holding a change, and a
property test over random names), the plugin's badges and the session now sending three slots,
including leaving a folder with nothing changed. The real `mman` was driven through a
pseudo-terminal in a scratch repository with a modified, a staged, an untracked and a clean file
and a folder holding a change: `~`, `+`, `?` and nothing sat beside them and `~` beside the folder;
inside the folder the one file had `~`; back in the root the marks returned; killing `mman-git`
removed them and gave the width back; with `git_status = false` there was no badge and no
`mman-git`; and an `mman` copied alone browsed with none. Not run: a Nerd Font glyph set, and the
colors of the marks were not read off the screen.

---

## 2026-10-11 — The preview column becomes a plugin: text, hex and archives (M4b, third part)

**Context.** The preview column was the largest piece of the interface still doing a plugin's job:
`preview` loaded, tokenized, hex-dumped and listed; `tui::text_preview` read it off-thread and
kept the scroll; `tui::preview_view` drew it. The earlier slots carry a line or a badge, so the
open question was what crosses the process boundary for a megabyte of source with scrolling.

| COA | Approach | Verdict |
|---|---|---|
| A | A whole typed document (`Text`, `Hex`, `Archive`) pushed once; the interface keeps scroll, wrap, scrollbar and reveal | Chosen |
| B | The plugin owns scroll and wrap and sends only the visible rows, on `scroll` and `resize` events | Rejected: every key and wheel notch becomes a round trip, wrap logic moves out, and the feel changes |
| C | Add the slot and `read_file`, keep the built-in preview as a fallback | Rejected: nothing leaves the core, and the code would exist twice |

**Decision.** Three additions to `plugin_api`: the `preview` event (the file the column is about),
the `read_file` request (up to 4 MiB of a file from an offset, through the host's own `Vfs`, so ssh
and archives preview like the disk) and the `set_preview` notification (the document). The
plugin, `plugins/preview` (`mman-preview`), turns `read_file` into a read-only `Vfs`
(`RemoteVfs`: a window that starts at 8 KiB and doubles while reads run on, falling back after a
seek), which is what let `archive`, `head` (hex), `highlight` and the text-or-binary decision move
over with their tests unchanged. Each `preview` event starts a worker thread; the loop sends a
finished document only if no newer event arrived, and a cancelled worker stops asking at its next
request. The interface keeps the document, the scroll offset and the wrap cache, so scrolling
needs no round trip, and `hex_dump` (how many bytes fit in a row) stays in `tui` because it depends
on the pane's width.

**Departures from the plan.** (1) The plan had the plugin follow `selection_changed`. That is the
browser's cursor, but the preview column is also drawn for the trash browser's cursor and the
finder's highlighted result, so the interface now says which file the column is about with its own
`preview` event, sent only to the plugin that `provides = ["preview"]`. (2) `PreviewContent` was
first tagged `kind`, and an archive listing has a `kind` of its own (zip, tar): the two collide
when flattened. The unit tests did not see it because they never sent an archive through the real
serializer; the process test that runs the real `mman-preview` did, on the first archive. The tag
is `type` now, and there is a round trip for every variant. (3) A hook's precedence. Before, the
first hook naming an extension decided it, whatever its kind, and only then was it checked for
being text. Passing the plugin just the text hooks would have let a text hook run for an
extension an earlier image hook owned, so the interface takes each extension out of every later
hook before passing them. (4) A plugin that died with a document half made left "loading
preview…" up until the cursor moved. The interface now notices the process is gone and shows the
name; the next file starts it again. (5) The options table I gave before building mentioned a
`preview_scroll` event for `J`/`K` and the wheel; with scroll kept in the interface (COA A) it is
not needed, so there is none.

**Trade-offs.** The whole document is tokenized before any of it is sent, so a file near the 1 MiB
cap says "loading preview…" for about 3.4 s in a release build (`syntect`, exactly as before the
move; the megabyte reaching the interface as one 4.7 MB line adds about 0.1 s). A debug build
takes most of a minute on the same file. Sending plain text first and the colors after would hide
that, and is left as a follow-up. A plugin can read any file the user can, as it always could.
Without `mman-preview` the column shows only names, and a notice at startup says how to get it
unless the user configured a `preview` provider of their own. Dependencies: `syntect`, `zip`,
`tar` and `flate2` now belong to the plugin, and `preview` only decodes images.

**Verification.** `scripts/check` passes. Tests moved with the code (archive limits and the
hostile-zip property tests, the hex head with a named pipe, the text-or-binary decision, the hook
runner) and new ones were added: the wire shapes and limits of a document (a property test that
no accepted text holds a character that could act on the terminal), `read_file` against an
in-memory backend (a range, a past-the-end range, the cap, a folder never opened), `RemoteVfs`
(a property test that any mix of reads and seeks matches a plain reader, few round trips for a
straight read, a zip listed through it), the loop's bookkeeping (replies reach only the worker
waiting for them, a host that goes away wakes every worker), the interface's state (a document for
a file the cursor left is dropped, a reload keeps the scroll, no previewer means only the name), and
the hook arguments. The real `mman-preview` runs in a process test with the test as the host,
including the stale-document case (the first file's request held while the cursor moves on: only
the newer file is answered). The real `mman` was driven through a pseudo-terminal read with
`pyte`: source with keyword and comment colors apart, `J` four half pages down to row 72 and `K`
back to 36, a hex dump, zip and tar.gz listings, a `pdf` shown as its hook's text, a picture still
drawn by the core, the finder's preview of a file the cursor is not on, the last of a burst of
moves being the file shown, a change on disk reaching the column, a killed plugin restarting for
the next file and ending the wait while a debug-build megabyte was still tokenizing, and an `mman`
copied alone browsing with the notice. A 1 MiB file previewed in 3.8 s end to end in a release
build and scrolled at once. Not run: the trash browser's preview (same call as the others, by
construction), a remote host, and a real kitty.

---

## 2026-10-11 — Pictures in the preview column come from the plugin (M4b, fourth part)

**Context.** After the text preview moved, the interface still decided which files were pictures
(`image_preview::source_for`), ran the `[[preview_hook]]` programs that make a picture
(`tui::preview_hook`) and drew the result. The open question was what crosses the process boundary
for a picture, since a document of a megabyte of text was already the largest message.

| COA | Approach | Verdict |
|---|---|---|
| A | The plugin decodes and sends a downscaled RGBA buffer in the document | Rejected: 4 MB a 1000x1000 picture as hex on a JSON line, a size cap to invent, and the grid would multiply it |
| B | The plugin sends no pixels: the file itself, or a scratch file it made, and the interface decodes | Chosen |
| C | Only Inspect moves; pictures and thumbnails stay in the core | Rejected: half of M4b's last item, and nothing for the hook runner to leave |

**Decision.** `PreviewContent::Image { source }`, with `ImageSource::Original` (the previewed file,
read by the interface through the same `Vfs`, so ssh and archive paths keep working) or
`ImageSource::Handoff { file }`. The plugin decides what is a picture (`is_image`, then a hook
covering the extension) and runs image hooks, which needed `kind` on its `Hook` and a second flag,
`--image-hook`. `ImagePreview` stopped deciding anything: `update` only tracks which file the
column is about and clears on a change, and `show`/`forget` apply the plugin's answer. The text
pipeline now takes every file and `Previews::accept` routes the answer to whichever pipeline it is
for, so the "is this an image" test is made once, by the plugin.

**Departures from the plan.** (1) I said Inspect would go first inside this option. Reading it
showed it needs the owner, on-disk size and link count, which the wire does not carry (they are
local-only), plus a multi-line view; it is a cycle of its own and is left unchecked in the roadmap
rather than half-moved. (2) The deleting. A handoff is a file the interface deletes after decoding,
so a hostile plugin that could name any path could make the interface delete it. The host refuses a
handoff that is not directly in the temporary folder under the scratch prefix, the decoder checks
again and refuses a link (`symlink_metadata`), and `remove_file` on a link removes only the link.
A test with a link pointing at a real picture shows the picture surviving. (3) Two sides can drop
an answer, so both delete: the plugin when a newer file arrived, the interface when the file is no
longer the column's.

**Trade-offs.** Without `mman-preview` a picture shows only its name, like text; the filmstrip,
grid and tile views still decode in the core, which is correct for now (M7 decides the alternate
views). A hook's picture is rebuilt each time the file is asked for, since there is no cache. The
scratch name is predictable (pid and a counter), so another local user could race to create it in
a shared temporary folder; the symlink check and the regular-file check narrow that, and a
private runtime directory would close it, left as a follow-up.

**Verification.** `scripts/check` passes. New tests: the wire shape of both sources and a property
test that a handoff is accepted exactly for a scratch name directly under the temporary folder;
the hook runner (a picture left in place, nothing left behind on failure, empty output, 70 MB
output, a link, a timeout); `load::content` for a direct picture and a hooked file; the real
`mman-preview` process answering `original` and `handoff`; and the interface's decode (a handed
picture decoded then deleted, a non-scratch file neither read nor deleted, a link refused with its
target intact). The real `mman` was driven over a PTY read through `pyte`: a PNG drew its red, an
image hook turned a `.xyz` file into a green picture, a failing hook and a text file beside them
drew the name and the text, moving back redrew the first picture, nothing was left in the
temporary folder, and an `mman` copied alone showed names and the notice. Not run: a real kitty,
a remote host, and Inspect (not moved).

---

## 2026-10-11 — Inspect becomes a plugin (M4b, last part)

**Context.** The Inspect panel was the one piece of M4b left in the core. Moving it meant a
plugin could describe an entry that may live on ssh or in an archive, where a plugin has only
`read_file`, and could count a folder, which takes seconds on a big tree.

| COA | Approach | Verdict |
|---|---|---|
| A | A `stat` host request plus an `inspect` document; the plugin words the rows | Chosen |
| B | The core gathers the metadata; a plugin only formats and orders the rows | Rejected: the plugin would barely do anything, and the `lstat` and the walk would stay in the core |
| C | A generic multi-line pane view the core fills from a plugin's layout | Rejected: still gathers in the core, and needs a view type that does not exist yet |

**Decision.** Three new wire pieces in `plugin_api::inspect`: `stat` (answered by the host from
`symlink_metadata`, `read_link`, `exists` and `local_path`, with owner and group left as ids so the
plugin names them only when `local_path` says they are this machine's), `tally` with its
`tally_cancel` and `tally_done`, and `set_inspect` with the rows. The plugin is a state machine over
lines (`Session`), tested without any stream, with a thin loop (`run`) and a process test that plays
the host against the real program. `tui` keeps `InspectPanel` (the entry, the last rows, a
placeholder) and the drawing.

**Departures from the plan.** (1) I meant to count a folder with the host's existing `walk` and
metadata, so no new request was needed. Reading `browser::finder::walk` showed it follows symlinks
(its listing uses `stat`), stops at 16 levels and 200,000 entries, and drops unreadable folders:
the panel's numbers would have changed and a link loop would have been counted 16 deep. So the
count stayed the one `tally_dir` the marked-size pill uses, moved to `shared::tally`, and became a
request of its own. (2) A request the host awaits inline would hold up the plugin's reader for the
whole count, including its own `tally_cancel`. `tally` therefore returns at once and ends with an
event, the way `walk` does, with a `Tallies` registry shaped like `Walks`. (3) The formatting
helpers (`format_size`, `mode_string`, `group_thousands`, times) are used by both the lists and the
plugin, so they moved to `shared::format` (which gained the `chrono` dependency) rather than being
copied; `hud` re-exports `format_size` so its callers did not change. (4) A failed `stat` used to
set a status-line message; the plugin has no way to set one, so the panel shows an `Error` row
instead, which also means the panel never waits for ever.

**Trade-offs.** One more process on a fresh install, and "Inspect" now does nothing but say where
to get the plugin when it is missing (the status line cuts that message at 120 columns, as the
finder's already does). The biggest file (`largest`) is not on the wire since nothing in the panel
used it. Rows arrive twice for a folder (counting, then totals) and the second replaces the first;
the interface drops rows for any entry but the one it is open on.

**Verification.** `scripts/check` passes. New tests: the wire shapes and a property test that
cleaned rows never hold unsafe characters or exceed their limits; `stat` against a memory backend
(sizes, allocated bytes, links, a broken link) and the local disk (`local_path`); the `tally` job
(totals, a refused repeat, a stop); the plugin's rows for each kind of entry, its session (a stale
reply dropped, a count stopped by the next entry or by closing, a failure shown), and the real
`mman-inspect` process. The real `mman` was driven over a PTY read through `pyte`: a file's panel
(type, size, permissions, owner), a folder counted to `3 files, 1 folders, 18B`, a link, a broken
link, reopening, the plugin process running, and an `mman` copied alone showing where to get the
plugin while the browser stayed responsive. Not run: a real kitty, an ssh folder, and the inside
of an archive.


---

## 2026-10-11 — Compress and extract become a plugin (M5)

**Context.** M5 was written as "archives and undo leave `file_ops`; trash management becomes a
plugin". Two decisions were taken before any code. First the slicing: the user ruled that the trash
is a fundamental feature and stays whole in the core, and chose the cut that also keeps undo there,
since an undone copy goes to the trash and every change is recorded by the core that performs the
revert. M5 therefore shrank to archives alone. Second the boundary. The archive code was not
"just local files": `compress` and `extract` took `&dyn Vfs` and called nine of its methods, and the
interface ran them on its own `Vfs`, so compressing from and extracting to an ssh folder worked. A
plugin is a separate process with no `Vfs`, and the host's requests (`read_file`, `stat`, `walk`,
`tally`) are read-only.

| COA | Approach | Verdict |
|---|---|---|
| A | The plugin runs the unchanged code against `LocalVfs` on local paths; ssh and in-archive paths are refused | Chosen, with the regression accepted by the user |
| B | The host serves the nine `Vfs` calls to the plugin over the wire, streamed | Rejected: that is a Vfs-over-the-wire protocol, M7's design problem done early and without its decision on process boundaries |
| C | A, plus the core keeps its archive code for remote paths behind a feature | Rejected: it duplicates the code instead of moving it |

**Decision.** `plugin_api::archive` holds the wire: an `archive` event carrying a request number and
the work, `archive_cancel`, `archive_progress` (the entry reached and a running total) and one
`archive_done`. The result names which jobs *wrote something* (`finished`), because the interface
trashes originals per job: the old code trashed a job's sources right after its archive was
complete and stopped at the first failure, and sending only the finished indices keeps that end
state without the plugin touching the trash. `Format`, `Level`, `Conflict` and `Password` live in
`plugin_api` because both sides name them; `Password` gained a serde form that refuses control
characters and anything over 256 characters, wipes the parser's string when it is handed one, and is
masked in `Debug`, so a logged event or `{:?}` of a request never shows it.

The plugin is `Session` (a thread per request and a map of cancel flags, writing through one
channel), `work::execute` (the jobs in order, with the password checks an extract makes before it
writes), and a `run` loop with a forwarding thread so one place owns stdout. The interface turns the
plugin's answers back into the bulk operation it already had: a new `BulkMsg::Entries` carries a
total (not a step, because the plugin thins its messages to one per 40 ms, so counting messages
would have undercounted a thousand-fold), `archive_done` sends `Done` after trashing on the blocking
pool, and a `password_needed` answer ends the operation and opens the existing password prompt with
the request kept to start again.

**Departures from the plan.** (1) The first design had the interface ask `needs_password` before
starting, as it did. The plugin answering `password_needed` is better: the archive read left the
render thread, and one code path serves the first request and the retry. (2) I planned to keep
`ConflictPolicy` and `Outcome` shared between `file_ops` and the archive code. A plugin may depend
only on `plugin_api` and `shared` (a test over `cargo metadata` enforces it), so the plugin got its
own `ArchiveError`, `Outcome` and progress callback, `Conflict` came from `plugin_api`, and `mkdir
-p` moved to `shared::create_dir_all` with `file_ops::create_directory_all` as a wrapper. (3) A
plugin that dies mid-request would leave the operation busy for ever, since the host does not watch
for deaths. The interface now gives up on a request whose plugin is not running, after a second's
grace because the plugin's last lines may still be on their way when its exit is seen.

**Trade-offs.** One more process on a fresh install, and `:compress`/`:extract` do nothing but say
where to get the plugin when it is missing. ssh and in-archive paths are refused until M7. The
password crosses a pipe as one JSON line: only `Password` is wiped, not the line or the parser's
copies. Cancel is still checked per entry, so one huge file runs to its end first (as before). A
plugin killed with `SIGKILL` cannot remove the hidden temporary file it was writing (the real
archive name is never half-written).

**Verification.** `scripts/check` passes. The interface's 21 archive tests could not keep running
the work in-process, so they were rewritten (and eight added) around a recording plugin (`sh -c 'cat > file'`) that
stands in for `mman-archive`: they assert the exact request the interface sends (jobs, format,
level, policy, password) and what it does with each answer (finished, cancelled, failed in words,
password needed, wrong password, a stale answer dropped, trash only for finished jobs, a stopped
plugin, no plugin, a remote or in-archive path); the work itself is covered where it lives, by the 38
moved archive tests, the new `work`, `session` and wire tests (including properties on cleaning and
on the password round trip) and a test that plays the host against the real `mman-archive` process.
The real `mman` and `mman-archive` were driven over a PTY read through `pyte`, with `tar` and `7z`
checking the files on disk: pack and unpack, the sealed zip (made, opened by `7z` with the password
and refused with another, extracted through the prompts), `Esc` on a 40,000-file compress, the
plugin killed mid-request, and an `mman` copied alone. My first run of the kill scenario failed
because the harness pressed `Enter` and killed the plugin before the job had started (the listing
of a 60,000-file folder made the interface slow to take keys); waiting for `compressing` first made
it a real test, and it passed. Not run: a real kitty, an ssh folder, and the inside of an archive
(both now refused).

---

## 2026-10-11 — The rail of devices becomes a plugin (M6)

**Context.** M6 was written as "devices, the places rail, disk usage", with the note to first check
how far the deck is tied to the browser keys. The check found it is not: `DeckState::key` is a
self-contained state machine that `main` offers each key first, and `h/j/k/l` were never its. The
ties were elsewhere. The deck scanned the machine itself and ran `udisksctl` itself; wiring and
atlas shared its action code, the atlas imported its `holding` and the disk usage scanner, and the
disk usage scanner feeds the atlas. Two decisions were taken before any code: the rail of devices
only (Places was never built and stays for later), and wiring and atlas keep reading the machine
until M7.

| COA | Approach | Verdict |
|---|---|---|
| A | A plugin fills the rail: it scans and mounts, the interface keeps the rail, the selection, the keys and the fit check; disk usage stays | Chosen |
| B | Move the whole storage cluster (rail, wiring, atlas, disk usage) behind a general pane description | Rejected: it is M7's `SetPane` design done early |
| C | A `storage` cargo feature in the core | Rejected: nothing becomes replaceable and the core does not shrink |

**Decision.** The wire is `plugin_api::devices`: a `devices` event with a `watch`, `open`, `toggle`
or `eject` command; `set_devices` with the whole list (each device with its rates and history, and
a busy flag); `devices_note` with a message or a folder to go to. Rates and history are the
plugin's (it keeps `Activity`); the slide-in of a new device stays the interface's, since it is
about what the user sees, and so does `cargo_fit`, because `p` must refuse a copy that will not fit
before it starts and that needs the carried files, which the plugin has no way to see. The
interface sends no scan: `DeckState` lost its scanner, its runtime handle and its action queue and
gained `accept` (the list), `note` (the plugin's news) and `Outcome::Do` (a command for the
plugin). A plugin may depend only on `plugin_api` and `shared`, so the `devices` crate became
`shared::devices` (files moved with `git mv`, only `crate::` paths changed), and the steps to open,
toggle and eject became `shared::devices::plan::Plan`, which `device_actions` (still used by the
wiring diagram) and the plugin both run, so the refusal texts and the messages exist once.

**Departures from the plan.** (1) I first thought of keeping `Actions` in the deck and sending the
plugin only data. Then a mount would still run in the interface, which defeats the move, so the
plugin owns the actions and the interface only asks. (2) The scan has to survive a `statvfs` that
blocks on a dead network share; the plugin scans on its own thread and takes commands on a
channel, so a stuck scan delays the next list but never an action or the host's lines.

**Trade-offs.** One more process on a fresh install; with it missing the rail says so and mounting
does nothing. The plugin sends the whole list every second, a few hundred bytes. It idles when the
rail is not showing but does not exit. A plugin that dies is started again at most every three
seconds, and one that was never found is not retried. The wiring and atlas views still carry their
own scan and `device_actions` until M7, so the code that reads `/proc` and runs `udisksctl` has
two callers.

**Verification.** `scripts/check` passes. Tests: the wire (shape, the checking, a property that
nothing unsafe, over-long or repeated survives), `Plan`, the scanner (silent until watched, a list
at once, a refresh, stops when told, ends when the host goes), the session (open, toggle, eject,
refusals, a gone device, one action at a time, busy in the list), the real program played by a
stand-in host, the deck over a list and notes, and the bundled entry. The deck tests that
scanned in-process were rewritten around lists. The real `mman` and `mman-devices` were driven over
a PTY read through `pyte`: the rail drew this machine's root volume with its gauge, the `▌` mark and
the activity pulse, `Tab` then `e` was refused in words, the plugin killed with `SIGTERM` was
started again, and an `mman` copied alone showed "mman-devices was not found" and the hint. Not
run: a real mount, unmount or eject (no stick or share here), a real kitty, a network share gone
away.

---

## 2026-10-11 — Browsing inside an archive is served by the archive plugin (M7, first part)

M5 moved compress and extract into `mman-archive` and left the browsing of an archive's inside in
the interface (`vfs_archive`, a `Vfs` over the zip and tar code), refusing it for the plugin until
M7 decided how a plugin serves files. This part answers that for archives. The hard part is that
the `Vfs` trait is synchronous and used from the render thread, while a plugin is a process that
speaks lines.

**The decision.** The host asks and the plugin answers, one `vfs` request per `Vfs` call
(`metadata`, `list`, `scan`, `read_link`, `open`, `read`, `close`), each reply carrying the
request's `id`. This is the only message in the protocol that runs from the host to a plugin; the
rest have the plugin ask. A plugin that `provides` `vfs-archive` gets the requests for every path
at or inside an archive (`shared::archive_path::route`). What the plugin needs from the archive file itself it asks of the host with
`read_file`, which already existed for the preview plugin and moved to `plugin_api::remote` as
`RemoteVfs`, so an archive on an `ssh://` machine opens the same as a local one.

**Why not the other direction.** Serving the interface's nine `Vfs` calls to the plugin over the
wire was rejected in M5 and M6 as a protocol built before its decision on process boundaries; this
is that decision, and it came out the other way round: the plugin is the server because it owns
the archive code, and the interface keeps only a router.

**Process boundary.** `plugin_host::ServeLink` is made before any plugin starts, since the `Vfs`
that holds it is built first. A caller blocks on a std channel; the reader task of the plugin's
connection delivers the reply. A request ends by exactly one of its reply, its process ending or a
timeout of 120 s. The link counts each start of the plugin as an epoch; a request that names a
handle is written only if the handle's epoch is the current one, checked under the lock that
`attach` takes, so a handle from a dead process can never reach a restarted one. The plugin bounds
its open files at 32 (each can hold an unpacked entry, since a zip or gzip entry cannot seek) and a
read at 256 KiB. The interface reads a file in windows, so a small read costs no round trip.

**Departures from the plan.** The crate `vfs_archive` was not removed: it is now the router and the
translation between the wire and the `Vfs` types, because the browser, the preview and the disk
usage view still want a `Vfs`. The codecs, the index and the engine moved to the plugin, and the
path syntax (`address.rs`) to `shared::archive_path`, since both sides need it and a plugin may
depend only on `plugin_api` and `shared`.

**Trade-offs.** Browsing an archive now needs `mman-archive`; without it the status line says how
to get it and nothing else changes. Every listing is a round trip and a large archive's table of
contents is read through the host. Archives stay read-only, so compress and extract inside one
remain refused by name.

**Verification.** `scripts/check` passes. Tests: the wire, the link (replies in any order, a process that
ends waking what waits on it, a handle from an earlier process never sent to a later one), the
plugin's server (reading by handle, the cap on one read, handle numbers never reused, the limit on
open files), and the plugin as a real process (a zip and a tar.gz read through the wire, an archive
that changed listed again, an unknown request refused). The 120 s timeout itself has no test. The real `mman`
and `mman-archive` were driven over a PTY read through `pyte`: a zip opened as a folder, its
folder and file listed, the file's text in the preview column, and `Q` leaving the browser inside
`t.zip!`. Not run on the real binary: the plugin killed mid-listing, a very large archive, an
archive on an `ssh://` machine, and reading the missing-plugin message off the screen.

## 2026-10-12 — Ssh served by a plugin, step 1 of 3 (M7b)

Three ways were weighed. A: move `vfs_ssh` into a plugin on the archive plugin's wire. B: the same,
with writes, streaming copies and the live refresh built into the plugin from the start. C: keep
the crate in the core behind a cargo feature. A was chosen: Minuteman started as a better Ranger,
ranger has no native ssh browsing, and the async stack (`tokio`, `openssh`,
`openssh-sftp-client`) is the largest dependency in the core for a minority of sessions. C moves
nothing; B is the hardest to verify in one go.

**The staging.** Reads first and writes later as two commits would break remote copy, move and
delete in between, so the work is: (1) groundwork with no behavior change, (2) the plugin built
and tested but not wired in, (3) the switch and the deletion of `vfs_ssh` from the core. No
commit leaves ssh worse than before.

**Step 1.** The address parsing (`target.rs`, 422 lines, only `thiserror`) moved to
`shared::ssh_path`, as `archive_path` did, because the interface tells a remote path apart in a
dozen places (`is_remote`) and must keep doing so without the SSH stack; `vfs_ssh::target` is now
a re-export so nothing else changed. `plugin_api` gained the `vfs-ssh` word and the requests that
change things: `home`, `create` / `write` / `flush` (a file written by offset, because a zip goes
back to patch a header), `create_dir`, `create_file`, `touch`, `set_mode`, `symlink`, `rename`,
`replace`, `copy_file`, `remove_file`, `remove_dir_all`. A file written by `write` shares the
limit of open handles with one being read. The host has a second `ServeLink` and picks the link
from the word a plugin provides, so a plugin answers only for its own kind of path. The archive
plugin refuses the new requests; it is never sent them.

**Open for step 2.** What a large copy costs over JSON lines and hex bytes (the in-process path
reads 256 KiB windows directly), measured before step 3. A half-written file when the plugin dies
mid-copy. `rename` between two machines must keep answering `CrossesDevices`, which the move
fallback and undo rely on.

**Verification.** `scripts/check` passes. New tests: every changing request survives the trip as
a line (bytes as hex), and a plugin that provides `vfs-ssh` is reached by its own link and not by
the archive one. Nothing runs differently yet.

---

## 2026-10-12 — Ssh served by a plugin, step 2 of 3 (M7b)

Three ways to build it were weighed. A: copy the `vfs_ssh` engine into `plugins/ssh` behind a
`Server` and keep the planned wire (JSON lines, hex bytes, 256 KiB windows). B: the same engine
with binary frames for reads and writes. C: rewrite on a lighter stack (the `ssh` and `sftp`
programs, or a pure-Rust ssh crate). A was chosen and the wire measured before anything else was
decided; C was the worst fit because the present stack exists to leave host keys, `~/.ssh/config`
and `ProxyJump` to OpenSSH.

**Shape.** The plugin (`plugins/ssh`, package `remoter`) holds the engine with three changes: its
import of the address types now comes from `shared::ssh_path`, `ConnectOptions::from_env` reads the
test variables, and a stream's `seek` to where the cursor already is does nothing. The last one is
there because the host names an offset on every window, and a real seek on the SFTP client's file
first waits for the writes in flight. It moved the plugin's upload from 20 to 23 MiB/s, which is
less than hoped: the larger part of the gap is the wire, below. The host side is a new core crate, `vfs_remote`, not a module of
`vfs_archive`: the two share an idea but not a purpose, and one is read-only. `PluginReader` was
copied from `vfs_archive` with the message changed, which is duplication to remove when the next
backend needs a third copy (it belongs in `plugin_api`, taking the error mapping as an argument).
`PluginWriter` is new: `io::copy` hands over 8 KiB at a time, so a writer gathers up to a window
and sends it when full, on a seek that does not continue, on `flush` and on drop.

**Where the tests live.** A core crate may not depend on a plugin, nor a plugin on a core crate
(`dependency_direction` reads dev-dependencies too), so the whole chain is tested without either
naming the other: `vfs_remote`'s tests build `mman-ssh` with `cargo build` when it is not next to
the test binary, start it as a child, and speak the protocol to it from a small stand-in for
`ServeLink`. The `sshd` helper is shared with the plugin's tests by a `#[path]` include.

**Measured.** 256 MiB through a loopback `sshd`, release builds, stand-in host:

| | in-process | plugin | plugin / in-process (time) |
|---|---|---|---|
| download | 16 MiB/s | 20 MiB/s | 0.78 |
| upload | 56 MiB/s | 23 MiB/s | 2.4 |

The plugin's engine with no wire does 69 up and 25 down, so the wire costs about 7 ms a window:
encoding and decoding 256 KiB of hex take about 3 ms, and the rest is two pipes and the host
waiting for each reply. That makes a ceiling near 35 MiB/s when each window waits for the last.
Whether it matters is a question about links: it is invisible on anything slower than about
30 MiB/s and a real loss on a fast LAN. Not decided here. The two cheap answers are binary frames
(removes most of the 3 ms) and sending the next window before the reply to the last (hides the
rest, needs a post-without-waiting call on `Peer`). Both should be measured again through the real
`ServeLink` in step 3, since the stand-in is not it.

**Not done, deliberately.** `main.rs` still uses `RoutedVfs`, so a user sees no change and needs no
new binary yet. The plugin is not in the bundled list. The `MMAN_SSH_*` variables exist so a test
can point the plugin at its own server and key; they are not configuration.

**Verification.** `scripts/check` passes. The `Vfs` conformance suite, plus the copy, move, drop,
browse, home and strict-host-key scenarios that `vfs_ssh` has, run through `SshVfs`, the real
program and a real `sshd` (nine, all passing). The plugin's own tests: every changing request
against the real disk, the handle limit, a read handle that cannot write and the reverse, the
refusal of a move between machines, a property test of writes at random offsets, and the line
protocol of the real binary. `SshVfs` has unit tests with an in-memory plugin, including a
property test that a file written in any pieces and seeks equals the same writes made locally.

## 2026-10-12 — Ssh upload through the plugin, pipelined (M7b)

**Decision.** Step 2 left the upload figure open: 23 MiB/s through the plugin against 56 MiB/s
in-process, because the writer sent a 256 KiB window and waited for its reply before the next.
Chosen: send the next windows before the reply to the last (over binary frames, which change the
wire in three places, and over accepting the figure). Author: deltaog-117.

**What changed.** `Peer` gained `submit`, returning a `Reply` that is waited on later; its default
runs `call` and hands back the settled result, so every stand-in that never overlaps requests stays
correct without change. `ServeLink::call` is now `submit` followed by `wait`, with the epoch check
and the write under the same lock as before. `PluginWriter` keeps up to four windows in flight
(1 MiB): a fifth makes it read the oldest reply first. `flush` and drop read every reply, and a
close is only sent after that, so the plugin never closes a handle with a write still unrun.

**Why order does not matter.** Every `write` names its own offset and the plugin seeks to it under
the file's lock, so the plugin running two windows in the other order gives the same file. It
could cost a real seek, and the plugin's writer skips the seek when the offset is where it
already is; the measured run did not suffer from it.

**Measured** (256 MiB, release, loopback `sshd`, through the test harness's process stand-in, not
`ServeLink` itself): upload 109 MiB/s against 56 in-process (0.52x the time, was 2.4x); download
unchanged at 19 against 16. Both copies arrived byte for byte. The real host must still be
measured once `main.rs` uses it, in step 3. Reads were not pipelined: a read-ahead needs the
reader to guess the next offset, and download already beats in-process.

**Verification.** `scripts/check` passes. New tests: a lagging peer that runs a request only when
its reply is waited for shows windows overlap, never more than five at once, and all arrive; a write
refused after it was sent is reported by `flush`; the close comes after the last write; and
`ServeLink` writes a second request before the first is answered and gives each reply to its own.

---

## 2026-10-12 — Ssh served by a plugin, step 3 of 3: the cut-over (M7b)

**Decision.** Three courses were weighed: one cut-over (A), the wiring first and the deletion a
cycle later (B), or keeping the old backend in the core behind a cargo feature as a fallback (C).
Chosen: A, the course the roadmap had already recorded. C would have undone that decision and left
the heaviest dependency in the core; B only postponed the deletion. Within A the order kept the old
backend alive until the last measurement, then deleted it.

**What changed.** `main.rs` builds `ArchiveVfs` over `SshVfs` over the local disk, with a second
`ServeLink` for the ssh plugin that `App::with_ssh_link` hands to the plugin manager. `ssh` is a
bundled plugin providing `vfs-ssh`, so it is found beside `mman` or on `PATH` and its install line
comes from the same table as the others. The 12 places that used `vfs_ssh` only to parse an address
now use `shared::ssh_path`. `vfs_ssh` is deleted with its tests (the same scenarios run against
`SshVfs` in `vfs_remote`), its entry in `scripts/vfs-gate`, and the in-process side of the
throughput test.

**Missing plugin.** The error a request gets while no `mman-ssh` runs is now recognisable
(`vfs_remote::is_plugin_absent`, by the exact text it carries), and `poll_nav` replaces it with the
install line, as archives already do. Matching the text is blunt, but it keeps the interface from
depending on the plugin API's error type and the constant sits beside the code that makes it.

**A start on a remote address.** `mman ssh://host/dir` used to work because the backend lived in
the process. The plugin is not running when the browser is first built, so the browser opens the
working folder and a move to the remote folder is begun once the plugins have started; requests
are queued on the link's channel and run when the plugin reads them.

**Measured** through the real `PluginManager` and `ServeLink` (256 MiB, release, loopback
`sshd`): download 22 MiB/s against 16 in-process (0.71x the time), upload 122 against 53 (0.44x).
Both copies arrived byte for byte. The earlier stand-in host gave 19 and 73, so the real link is
faster than the stand-in.

**Verification.** `scripts/check` passes. New tests: the bundled `ssh` entry and its install line,
and that only the absent-plugin error is taken for one. Driven through a pty against the compiled
`mman`: with no `mman-ssh` beside it the status line says how to get it; with it, a refused
connection is reported by the plugin.

---

## 2026-10-12 — A canvas for views a plugin draws, step 1 of 3 (M7c)

**Decision.** Three courses were weighed for the alternate views: all of them stay in the core
behind a cargo feature (A), all of them move out through a new pane vocabulary (B), or split by
what each view reads (C). Chosen: C. Grid, tiles, details and filmstrip are layouts of the browsed
folder; they share its cursor, marks, thumbnails and hit-testing, and a plugin drawing them would
have to mirror all of that. The atlas and the wiring diagram scan the machine on their own and keep
their own state, which is the shape of the devices and inspect plugins that already moved. They are
about 5,000 of the 7,600 lines. If the vocabulary proves too costly, C degrades to A for those two
views only.

**Staging.** Step 1 (this entry) is the primitive, with the core atlas still drawing when no plugin
provides it. Step 2 is `plugins/atlas` and the deletion of the core atlas; step 3 is the wiring the
same way. Step 1 is wired in rather than only built, because an unused module fails the
project's clippy gate, and because "a plugin provides it, else the core draws" is the behavior the
last two steps need anyway.

**The vocabulary.** A canvas is a grid of cells with four kinds of part: a filled block (with an
optional level, filled from the bottom, for a volume's gauge), a frame with a title, a line of
text, and an orthogonal path that can ask for a travelling mark. Every part is painted by meaning:
a tone, one of 16 hue numbers, a mark (chosen, here). The interface owns the colors: the 16 hues are
16 of the theme's own fields in a fixed order, a frame takes the theme's border style, and text
written over a filled block turns black or white by the block's brightness, so a plugin never
names a color and a view that moves out of the core cannot drift from the theme.

**Keys.** A plugin cannot be offered a key and hand it back, because the answer arrives a frame
late. Each canvas names the keys it claims, and the interface forwards those and keeps the rest for
the browser. The atlas passes `Esc` only at its top level; that becomes a claim list without `esc`
at that level. Clicks and the wheel inside the body all go to the plugin, in cells of the canvas.

**Checking.** A plugin may be hostile and its text reaches the terminal, so `Canvas::checked`
cleans control and direction-changing characters, and refuses (rather than clamps) a part with no
area or outside the canvas, a slanted path, a hue or level that does not exist, and more parts,
text or claims than the limits. That is what lets the renderer index a buffer without arithmetic of
its own; a property test paints random canvases into random areas and compares every cell outside
the area before and after.

**Left open.** `double` on a click is on the wire and always false for now: the atlas does not need
it, and the browser's double-click clock lives in `browser_mouse`. The wiring may. A view cannot yet
ask for the focus to leave it, nor draw an image.

**Verification.** `scripts/check` passes. New tests: the wire shapes and every refusal, a
property test over `checked`, the parser cleaning and refusing in `plugin_host`, an integration
test against the fixture plugin (open, canvas and note back, key, resize), 18 for the renderer, and
the pane's claim, hint and ownership rules. Driven through a pty against the compiled `mman`, with
the fixture standing in as the atlas plugin: its text appeared on a hue background in black or
white, its note reached the status line, a claimed key, a resize and a click reached it, and `q`,
which it did not claim, quit the browser. With no plugin configured, the core atlas drew as before.

## 2026-10-12 — The atlas as a plugin, step 2 of 3 (M7c)

**Decision.** The atlas needs the folder scanner that the `u` view also runs, and a plugin may
depend on `plugin_api` and `shared` only. Three courses were weighed: move the scanner into `shared`
and let the plugin run it on the local disk (A), have the plugin sum the host's `walk` stream (B), or
keep the scanner in the core and stream its rows to the plugin through a new request (C). Chosen: A,
the same shape as M5 (archives) and M6 (devices): code moves unchanged and the plugin runs it
against `LocalVfs`. B would have needed a wire change first, since `walk` carries no on-disk size,
hard-link or device information, and the atlas's figures would then have disagreed with the `u`
view's. C builds a protocol for one caller and leaves the scanner in the core after its only
second user has gone.

**What moved.** `scan_level` and its row types went to `shared::usage` with their tests and the
`/usr` benchmark. Moving only the scanner left `DiskUsageView` (the stack of levels, the cursor,
the sort, the cancel-on-drop job) behind in `tui` on a `tokio` blocking pool, and copying it into
the plugin would have been 300 lines of the same logic twice. So it moved too, as
`shared::usage_view`, onto a plain thread and `std::sync::mpsc`; it never needed `tokio` for
anything but `spawn_blocking`, and `open` loses its runtime-handle argument. The treemap, the
state and its tests moved to `plugins/atlas` with a small `Rect` of its own in place of ratatui's.
`holding` (which device a folder is on) went to `shared::devices` since the deck and the plugin both
need it. The wire grew one field: `open` carries `here`, because the plugin has to know the folder
the browser is in to mark it, and a canvas event has no working directory.

**What changed in the picture.** A plugin says tone, hue and mark and never a symbol, so the core
version's per-kind volume icons, its `◆` and its `·` separators are plain ASCII now (`*`, `|`,
`...`). The chosen tile used to be drawn with a frame in a color readable on whatever the tile was;
a canvas frame's color is the theme's accent, which is the same color as the used part of a volume,
so the chosen tile is instead painted in the selection color, still split by fullness, with the
frame on top. Keys: the core let `Ctrl`/`Alt` chords fall through to the browser; the host forwards
only the keys a canvas claims, with no way to hand one back, so a claimed key held with `Ctrl` or
`Alt` is now ignored. `Esc`, `Backspace` and `a` are claimed only while zoomed, which keeps `Esc`
closing the view at the map of volumes and `a` opening the appearance popup there.

**Limits.** A canvas holds at most 4,000 parts and 64 KiB of text, and a big window of tiny volumes
can want more, so the builder emits the chosen tile first and then the biggest, and leaves the
smallest bare once either budget is reached. A property test builds the canvas for any window and
any volumes and runs the host's own `checked` over it.

**Verification.** `scripts/check` passes. New and moved: 26 for the treemap and state, 12 for the
canvas builder (including the property test and a crowd of 6,000 volumes), 4 against the real
`mman-atlas` process (open, claims, a key answered with a note, zoom and resize, quiet when closed,
exit with its input), and the scanner and view tests in `shared`. Driven through a pty against the
compiled `mman` and read through `pyte`, 16 checks passed: `V` reached the view; the totals line,
the pinned volume, the used/free shades, and the hint appeared; `a` at the map opened the browser's
appearance popup; `Enter` zoomed in and the title named the volume; `a` switched to apparent size;
`Backspace` came back; a second click on the chosen tile zoomed; `o` then `Q` left the shell in the
volume's mount point; and a copy of `mman` alone showed the install line and kept browsing. Leaving
the view during a scan of `/` took the plugin from 140 CPU ticks in three seconds to none, so the
scan is cancelled. This machine has one listed volume, so the multi-volume layout is covered by the
unit tests and not by a pty session; not tried in a real kitty.

**Left open.** Step 3, `plugins/wiring`. The hint reaches the status bar when the first canvas
arrives, a frame later than the view switch used to show it.

---

## 2026-10-12 — The wiring diagram as a plugin, step 3 of 3 (M7c)

**Decision.** The diagram needs the machine's topology and activity, a folder tree it unfolds
below a filesystem, the device actions of the deck, and the answer to "does what I carry fit
here". Three courses were weighed: let the plugin own all of that on the local disk, as the atlas
does (A); keep the scan, the folders and the actions in the core and send the plugin a snapshot,
leaving it the layout and the drawing (B); or keep the diagram in the core behind a cargo feature
(C). Chosen: A. B needs a host-to-plugin data channel and a device note, and leaves most of the
code where it was, with the logic split across the wire; C is the fallback the roadmap allowed
and would have left `canvas-wiring` with no provider. A costs the one thing the core version did
that a plugin on the local disk cannot: unfolding a folder of a remote machine. Nothing on that
screen is a remote path (a mount point is local), so the loss is the diagram not following the
browser into an `ssh://` folder.

**What moved.** `cargo_fit` (pure) and the deck's `Actions` went to `shared`: the deck still needs
both, and the plugin needs both, so one copy is better than two. `Actions` ran its `udisksctl`
jobs on `tokio`'s blocking pool; it now uses a thread, as `usage_view` did, and `new` loses its
runtime-handle argument. The layout, the folder unfolding and the state moved to `plugins/wiring`
with their tests; ratatui's `Rect` is a small one of the plugin's own, keys are the host's key
names, and the folders read through a `Vfs` on threads. The scan stands behind a `Machine` trait
so a conversation can be played without the real disks, as in the atlas.

**The wire.** Two things were missing. The diagram follows the folder the browser is in (it
unfolds toward it and lights it) and the browser moves while the view shows, since `Enter` on a
folder goes there; and it judges the carried files against each filesystem, which only the core
knows. `open` already carried `here` once. Instead of growing `open` or adding two inputs, one
`context` input says the folder, whether hidden entries show and what is carried, and is sent
after `open` and whenever any of them changes. The atlas ignores it.

**Painting lines.** The core drew the connectors by adding up which sides of a cell had a line
and choosing a box-drawing character. A canvas path is a corner-only walk, so a trunk with three
branches would have drawn each branch's corner over the trunk. Rather than make the plugin draw
junctions out of single cells, the painter now adds up the directions every path leaves a cell in
before it chooses the character, which makes a trunk and its branches look as they did and gives
every future canvas the same. The plugin lets each line run one cell into the box at its ends,
where the box covers it, so the first and last cell of a line are junctions too.

**What changed in the picture.** ASCII symbols only, as with the atlas: `+` and `-` for folded
and unfolded folders, `[]` and `<>` for icons, `|` for the row you are in, "..." where a name is
cut. The gauge is a bar of blocks, the used part bright and the rest the same tone dimmed. The
chosen box is in the selection color. A box cut by the window's edge is left out whole, because
the host drops a frame that is not entirely inside the canvas and the core's trick of painting it
off screen and copying the visible part has no equivalent; the scroll keeps the chosen box whole.
Lines are cut to the window by the plugin, since a path with a point outside the canvas is
refused. The flowing mark now moves at the host's pace.

**Limits.** At most 4,000 parts and 64 KiB of text, and unfolding many folders could want more, so
the builder stops adding boxes when either budget is nearly reached. A property test builds the
canvas for any size from 1 by 1 up, any zoom and any chosen node and runs the host's own
`checked` over it; it found a row's right-hand figure running a cell past the window in a narrow
tree, now fixed.

**Verification.** `scripts/check` passes. New and moved: 61 for the layout, the folders and the
state, 10 for the canvas builder (including the property test and the line clipping), 5 against
the real `mman-wiring` process (open, claims, a verdict on a carried load, the folder the browser
is in unfolded and opened, resize, quiet when closed, exit with its input), 2 for the painter's
junctions. Driven through a pty against the compiled `mman` and read through `pyte`: from `/boot`
the diagram drew as framed boxes with junctions and gauges, with a yanked file each filesystem
said `fits` and how much was left, a click on the root filesystem unfolded it and `Enter` opened
`/`, `-` and `0` answered, `V` went on to the atlas, and a copy of `mman` alone showed the install
line. From a deep folder on `/tmp` the same view fell back to the indented tree, as it did before.

**Left open.** The mark flowing along the line into a busy disk was not seen in a pty, because
the disk was idle; the path that carries it is covered by a unit test. Not tried in a real kitty.
M7 is done: the grid, tiles, details and filmstrip stay in the core as layouts of the folder being
browsed.

---

## 2026-10-12 — Undo and redo off the render thread

**Decision.** Undo ran each step on the render thread, so taking back a large cross-device move
(a copy, then a delete) froze the interface. Three courses were weighed: hand the entry out as a
value that runs on a worker and is handed back (A); keep the history behind a mutex and run the
whole call in the worker (B); or run cheap undos inline and send only slow ones to the worker (C).
Chosen: A. B holds a lock across a file copy, so the stall only moves to the next `record`; C
keeps the blocking path and needs a guess about which undos are slow.

**What changed.** `History::begin_undo`/`begin_redo` take the entry off its stack as a `Batch`;
`Batch::run` carries out the steps and tells a callback which path is next, which may stop it;
`History::finish` puts the done steps on the opposite stack and the rest back where they came
from. The model is one rule: every change is in exactly one of the undo stack, the redo stack or
one batch. `History::undo` and `redo` are the same three calls run in a row, so the existing
round-trip property test and the partial-failure test cover the new path. The interface adds
`BulkKind::Undo`/`Redo` and a `BulkMsg::History`, so the busy gate, the status line and `Esc` work
as for a paste.

**Verification.** `scripts/check` passes. New: three tests in `file_ops` (a cancelled undo, a
cancelled redo, the callback seeing each path first) and two in the interface (undo is a
background operation that holds others, and a cancel loses nothing). Existing undo tests now wait
for the worker. Installed with `cargo install --path crates/tui` and driven through a pty: 300
files of 1 MiB moved from `/tmp` to the home disk, `z` put all 300 back, `Z` moved them again.

**Left open.** The "undoing…" line was not seen in the pty: the move took 0.4 s and the harness
read the wrong screen row. A worker that panics leaves the operation busy and the batch lost, as
the trash does. A keypress that records a new change while an undo runs is held back by the busy
gate; if one ever got through, a stale redo entry could appear.

## 2026-10-13 — Folder listings off the render thread

**Decision.** `draw` listed the folder under the cursor itself, once per frame, and every
`browser.reload` after a file operation listed two folders on the spot. On the local disk that is
free; for a folder inside an archive or on an `ssh://` machine each one is a round trip to a
plugin, with a 120 s timeout when it hangs. Three courses were weighed. A: have the archive plugin
rebuild a changed index in the background and answer "refreshing" meanwhile. B: move the interface's
own listings onto the blocking pool. C: a progress gauge with cancel for the first open, plus an
index cached on disk. Chosen: B. A fixes one backend and cannot serve the old index without
risking reads at the old offsets of a file that changed; C leaves the render-thread calls where
they are and adds a file format that has to be trusted.

**What changed.** `tui::dir_preview::DirPreview` lists the folder under the cursor on the blocking
pool when the cursor reaches it and every 500 ms after (5 s for a slow one), holding the frame at
most 20 ms for a first listing so a local folder is never drawn empty; `entries(folder)` returns
nothing unless the listing is of that very folder. `shared::is_slow` (remote, or inside an
archive) is the one test for "do not wait for this". `App::resync` replaces the nine
`browser.reload` calls: a folder on this disk is re-read as before, a slow one is passed to
`LiveRefresh::kick`, which lists again at once through the path that already applies a listing by
path and keeps the cursor. A kick during a running listing is kept, so a result that started
before the change cannot be the last word. `toggle_hidden` and Refresh follow the same rule
(`BrowserState::flip_hidden` flips the filter without reading). The local disk behaves exactly as
before, which is why no existing test changed.

**Trade-offs.** On a slow folder a paste or rename now shows its result a moment later, when the
background listing lands, instead of blocking until it does; marks that point at deleted paths are
dropped then too, and only if the listing differs from what is shown. A folder's preview can be
up to its interval stale after a change made elsewhere, as the file lists already could.

**Verification.** `scripts/check` passes. New tests: `is_slow` as properties, six on `DirPreview`
(including a stand-in `Vfs` that sleeps and counts its listings), two on `LiveRefresh::kick`, one
on `arrange_preview`. The real `mman` was driven over a PTY read through `pyte`: the preview of a
local folder, a file created behind its back appearing there, and a folder inside a zip listed.
Then `mman-archive` was stopped with `SIGSTOP` and `j`/`k` moved the cursor between two archive
folders; the same script against the previous build leaves the cursor where it was.

**Left open.** `goto` and `reveal` (a `/` hit or bookmark far away) and the trash view's folder
preview still list on the render thread. A very large `.tar.gz` is still read whole to open.
`ssh://` was not driven by hand, only through the same stopped-plugin path an archive uses.

---

## 2026-10-13 — The history popup

**Decision.** The roadmap's undo follow-ups began with "no popup lists the stack to pick from".
Three courses were weighed: a read-only list (A); a list where a row goes back to that entry
by running every newer one first (B); and undoing any single entry on its own (C). Chosen: B. A
lists what nobody can act on. C breaks the rule `History` relies on, that each revert is checked
against the state the entries after it left behind, so a rename followed by a trash of the
renamed file could not be undone out of order.

**What changed.** `History::summary` lists both stacks, next-to-run first, after ending the batch
being recorded. `history_popup` is pure state: rows, a cursor, a scroll window worked out from the
cursor alone, and `Key::Run(Walk)`. The walk is a counter in `App`: after each step finishes
`Done`, `poll_bulk` starts the next from the same `begin_undo`/`begin_redo` calls, so the busy
gate, `Esc` and `finish` behave as for a single `z`, and anything but `Done` ends the walk with
the usual rule that done steps stay done and the rest stays recorded. `:history` and a new
`history` action (unbound) open it; `history` joined the reserved `:` words so a plugin cannot
claim it.

**Trade-offs.** Going back several entries is one operation to the user but several to the
plugins, which are told of each step's end. Going back to a row far down runs everything above
it, which is the point but can surprise; the row says how many entries it takes ("undo 3").

**Verification.** `scripts/check` passes. New: two tests of `History::summary`, six on the popup
(one property: the cursor stays inside the window for any moves), two on the popup's drawing and
hit-testing, three on `App` (a walk of two, a walk that stops at a failure and keeps the rest,
and the empty case) and the `:history` parse. The real `mman` was not driven by hand.

**Left open.** No default key. A click selects a row but does not run it, so a stray click cannot
undo files. The popup is a snapshot and does not follow the history while it is open, which is
safe because it is modal.

---

## 2026-10-13 — Search hits and the trash preview off the render thread

**Decision.** The last cycle left `goto`/`reveal` and the trash view's folder preview listing on
the render thread. Three courses were weighed: route every caller through the background move
that `:cd` already uses (A); do only the two a user sees, the search hit and the trash preview,
and leave the way back from a search blocking (B); and make `goto`/`reveal` non-blocking inside
`BrowserState` with a pending slot (C). Chosen: A. B leaves a known gap. C rebuilds `NavJob` in
a crate that has no runtime handle and changes the API the tests and `vfs_remote` use.

**What changed.** `BrowserState::plan_reveal` returns the `MovePlan` to a path's folder, landing
on the path, or `None` when the path is in the folder already shown; `select_path` moves the
cursor there. `reveal` is built from the same two, so it behaves as before for callers that
block. `show_search_match`, `restore_origin` and the finder's reveal call `App::begin_move`;
`SearchOrigin` gained `path` so the way back can land on the entry the search began on. A move
still on its way is dropped (`nav = None`) before a newer hit or `Esc` starts, since a second
`begin_move` is otherwise ignored and the old one would land last. The trash browser polls
`Previews::dirs` with its own cursor's folder and the frame reads `entries`. `poll_search`,
`show_search_match`, `step_search`, `update_search` and `draw` lost a `vfs` parameter they no
longer use.

**Trade-offs.** On a slow backend the search status says "Found" a moment before the folder has
changed, and a hit that then fails to open leaves that status behind the error. A trip that fails
reports through the status line instead of resetting the search to idle. The local disk is
unchanged: `begin_move`'s grace lets a quick folder land before it returns.

**Verification.** `scripts/check` passes. New tests: `plan_reveal` and `select_path` in the
browser, and four on `App` (a hit in another folder is a background move, `Esc` goes back in the
background to the same row, `Esc` drops a trip that has not landed, and a hit that cannot be
opened says so). The trash preview change is drawing code and has no test of its own. The real
`mman` was not driven by hand.

**Left open.** `open_file` still calls `goto` to open an archive file; it needs the archive's own
failure message, which `begin_move` does not give.

---

## 2026-10-13 — Opening an archive file off the render thread

**Decision.** `open_file` was the last caller of the blocking `goto`, left over because
`begin_move` shows the ssh plugin's hint whenever a plugin is missing. Three courses were weighed:
a failure-kind argument on `begin_move` (A); a separate `begin_archive_move` (B); and working the
message out from the target in `poll_nav` (C). Chosen: C. A and B make each of about eight callers
remember which message is theirs, and `:cd` into an archive already got the wrong one.

**What changed.** `MovePlan::target` gives the path a move goes to, as given. `NavJob` keeps it.
`poll_nav` has one new arm: a target inside an archive (`shared::is_inside`) while no archive
plugin runs gets `archive_open_failure`, which names the bundled plugin to install. Anything else
fails as before, with the caller's prefix. `open_file` plans `plan_goto(mount_root)` and passes
"cannot open the archive" as the prefix, the text it showed before.

**Trade-offs.** A typed relative path is judged as typed, before it is resolved, so a path that
only becomes an archive path after resolution gets the plain message. With the plugin running, an
archive that cannot be read shows "cannot open the archive: …" as before; a bad path inside one
shows the caller's prefix.

**Verification.** `scripts/check` passes. New tests: `MovePlan::target`, the hint for a move into
an archive with no plugin running, and an ordinary failure keeping its own text. The real `mman`
was not driven by hand. `open_file` itself has no test; it only builds the plan.

**Left open.** None of the folder reads on the render thread that the roadmap listed.

## 2026-10-14 — Opening a file on a remote machine

**Decision.** `launch` refused any `ssh://` path ("copy it here first"), since a program on this
machine cannot read one. Three courses were weighed: fetch a copy into a cache folder and open the
copy (1); the same plus an upload of the copy when the program exits, after asking (2); and mount
the machine with `sshfs` and open the mounted path (3). Chosen: 1. It reuses the streaming copy and
leaves the plugin as the only thing that talks to the machine. 2 can only know when a terminal
program exits (a window program is started detached), and overwriting a file on a machine the user
does not own wants its own cycle. 3 needs `sshfs` and FUSE and a second connection.

**What changed.** New `crates/tui/src/remote_open.rs` gives the cache path, `$XDG_CACHE_HOME/
minuteman/remote-open/<hash of the address>/<file name>`, so the extension survives and two
`notes.txt` on different machines never meet. It makes the folders and sweeps those older than a
day, all through `Vfs` (`scripts/vfs-gate` allows no direct disk use there). `launch` now checks
the program first (so a missing one is reported before a download), then starts a new
`BulkKind::Fetch` job: a folder is refused on the worker, a file is copied with
`ConflictPolicy::Overwrite` (a second open replaces the copy), and `poll_bulk` hands the finished
copy to `launch` again with the same command and terminal flag. The status says the file is a copy
and edits are not sent back. Also: `ssh://host/~` and `ssh://host/~/rest` now expand to the login's
home, in `SshVfs::resolve_typed`, through the `Home` request a bare `ssh://host` already used.

**Trade-offs.** Edits to the copy are lost on purpose, and the status line says so. The copy goes
whole before anything opens, so a large file waits; `Esc` abandons it, but only after the file in
flight (the same limit as a paste). The cache is cleared by age, not on exit, so a program still
holding a copy open is never pulled from under it.

**Verification.** `scripts/check` passes. New tests: six on `App` through a `Vfs` that serves
`ssh://box/...` from a folder (fetch then open, a second open replaces the copy, a folder is not
fetched, a missing program is reported before any copy, no cache folder, and a file inside an
archive on a remote is still refused), four on `remote_open`, and one on the `~` forms. Not driven
by hand: no ssh server was available, so the real `mman` and `mman-ssh` were not run for this.

**Left open.** Writing the copy back; a progress gauge for a large file; the other items under
"Remote browsing, follow-ups".

---

## 2026-10-15 — Giving up on a remote read at the connection

**Decision.** `Esc` on a move to an `ssh://` folder dropped the interface's job, but the request
it had sent kept going. In `mman-ssh`, `Link::session` held the session lock for the whole connect,
up to its 10 second timeout, so every later request to that machine waited behind it: the live
refresh, a preview, and the user trying again. Three courses were weighed: a shared connect inside
the plugin with no lock held across it (A); A plus a way to give up a request across the wire
(B); B plus killing the connecting `ssh` process (C). Chosen: B. A would still leave a thread of
the interface waiting for each abandoned answer. C would mean starting the master `ssh` outside
the `openssh` crate (it starts it with a plain `.status()`, which a dropped future does not kill)
and attaching with `Session::resume`, which ties us to the crate's control-socket layout. After B
the orphaned `ssh` holds nothing and ends at its own timeout.

**What changed.** New `shared::cancel`: a `Cancel` token, which a job enters on the thread doing
its work, since the `Vfs` methods take no token, plus `recv` and `wait_until`, waits that give up
within `POLL` (20 ms) of the token being set. A thread with no token waits as before. On the wire,
`plugin_api` gains `METHOD_VFS_CANCEL` (`vfs_cancel`, a notification naming the request id),
`cancel_line`, `parse_vfs_cancel` and `PeerError::Cancelled`. A plugin that does not know the line
ignores it, so `API_VERSION` stays 1. In the host's `ServeLink`, a request that is already given
up is not sent, a wait that is given up sends `vfs_cancel`, and a request whose caller leaves (a
cancel or a timeout) is now *abandoned* rather than forgotten. The table keeps it until the reply
comes, so a file opened for a caller who left is closed (`Pending::answer` hands back the
`close`). If the reply landed just before the caller left, the caller drains it and closes the
file itself. Only a cancel tells the plugin; after a timeout the work may be a long copy worth
finishing. In `mman-ssh`, `run` keeps a token per request id and runs each request inside its
token. New `connecting::Slot` holds the session: one attempt per machine, run as a task of its
own, joined by whoever needs it, and aborted when its last waiter leaves.
`Executor::run_cancellable` aborts read-only (`Retry::Safe`) calls, which the SFTP client documents
as cancel safe. Calls that change something still use `run` and finish. In the interface,
`NavJob`, `LiveRefresh`, `DirPreview` and `SearchJob` each own a token. A job is given up when it
is dropped or when the browser or cursor has left its folder, and a listing that was given up is
never shown as an empty folder.

**The model**, written before the code. Host, per request id: `Waiting{opens}` →
`Abandoned{opens}` → gone, or `Waiting` → gone; the step to gone is the reply or the process
ending. Invariant: a handle opened for a caller who left is closed exactly once. The table's lock
decides whether the reply or the abandoning came first, and exactly one side sees the reply.
Plugin, per machine: `Idle` | `Live(s)` | `Connecting{attempt, waiters ≥ 1}`, every change made
under one lock. Invariants: at most one attempt runs per machine; an attempt someone waits for is
never aborted; an aborted attempt is never installed. The property test over the host's table found
one real gap while I wrote it. A caller that drained a reply already delivered decided from the
reply's shape whether it held a handle, so it could have closed a handle that no request of its
own had opened. `give_up` now takes `opens` from the request.

**Trade-offs.** A waiting thread wakes every 20 ms to look at its token; it only does so while a
token is entered, and those threads are blocked on the network anyway. The token travels with the
thread rather than through the `Vfs` signatures. That keeps every backend and caller unchanged, but
a job must remember to enter its token; the four that read a remote folder in the background do.
A read of an open remote file is not stopped halfway, because `RemoteReader` holds the SFTP file
across the call. It is one window of at most 256 KiB.

**Verification.** `scripts/check` passes. New tests: nine on `shared::cancel` (one a property test
over nested scopes); one on the `vfs_cancel` line; five on the host's link, among them a property
test that an orphaned handle is closed exactly once over random orders of replies, give-ups and
takes (its regression file keeps the case it found); two on `Executor::run_cancellable`, plus its
panic case; six on `connecting::Slot` (a shared attempt, a failure, the last
waiter aborting it, a waiter leaving while another stays, forgetting a session, a panicking
attempt); an end-to-end one on the real `mman-ssh` process, where a `vfs_cancel` ends a request
stuck connecting to a server that accepts and never speaks; and one each on `NavJob`,
`LiveRefresh` and `DirPreview` through a slow stand-in `Vfs` (moved to `tui::test_vfs`). Driven by
hand through a pty, against the same silent server: `:cd ssh://127.0.0.1:<port>/` shows
`BUSY opening …`, `Esc` shows `cancelled`, and the same `:cd` again reached the server 0.01 s
later. The build of the previous commit took 8.07 s, waiting out the first attempt.

**Left open.** Killing the given-up `ssh` process (C); giving up a file's preview read through the
preview plugin, thumbnails, image previews and the marked-size walk; the rest of "Remote browsing,
follow-ups".

---

## 2026-10-16 — Answering `ssh` when it asks

**Decision.** A remote machine whose host key was not in `known_hosts`, or that wanted a password
or a key's passphrase, could not be opened: `mman-ssh` only used keys and the agent, and `ssh`
refused the rest with "Host key verification failed" or "Permission denied". Three courses were
weighed: `SSH_ASKPASS` with a prompt relayed to the interface (A); only `-o` options, the
existing accept-new switch for host keys and a hint for passwords (B); and a two-step flow that
shows the fingerprint and writes `known_hosts` itself, with a password pushed to `ssh` through a
pty (C). Chosen: A. It is the one course that covers host keys, passwords, passphrases and
one-time codes through one path, and it leaves every trust decision and the `known_hosts` file to
`ssh`. B would trust keys blindly and still not do passwords. C would re-implement `ssh` badly
(hashed hosts, ports, `~/.ssh/config` aliases).

A turned out to hide a second choice. The `openssh` crate starts its master `ssh` with
`-o BatchMode=yes` on the command line, which beats everything else and forbids every prompt, so
askpass would never run. Three routes: start the master ourselves and attach with
`Session::resume` (A1); put a wrapper `ssh` first on `PATH` that drops the option (A2); or drop
the crate and run `ssh -s sftp` directly (A3). Chosen: A1. A2 depends on the crate's argument
order and on `PATH` lookup, and would break silently; A3 is a rewrite of the 687-line `link.rs`.
A1 is a supported entry point of the crate and is the route the 2026-10-15 entry already named
for killing a given-up connect.

**What changed.** `plugin_api` gains `ask` (`METHOD_ASK`, `AskSpec` with a kind, `Secret` or
`Confirm`, and a prompt cleaned of control and direction characters, and `AskAnswer`, where a
secret is the zeroizing `Password`). `plugin_host` parses it into `PluginRequest::Ask` and, since a
person typing a password can take minutes, answers it from a task instead of inside the loop that
reads the plugin's stdout. It reaches the render thread as `UiRequest::Ask`, and the interface
shows `ask_popup`: titled with the plugin's name as the *host* knows it, so a plugin cannot pass
itself off as another; a dot per character, never the text; for a yes or no, only `y` accepts
(`Enter` does nothing, so a key pressed for something else cannot trust a key). It owns the keyboard
ahead of every other popup and swallows the mouse; a second question queues behind the first.
In `mman-ssh`, `master.rs` makes a private folder per attempt in the runtime folder, starts the
master without batch mode with `SSH_ASKPASS` set to its own executable and
`SSH_ASKPASS_REQUIRE=force`, and attaches with `Session::resume`; `askpass.rs` is both halves of
the helper, a socket in that folder that relays a question to the host, and the helper mode that
`main` enters when `MMAN_SSH_ASKPASS` names the socket, so there is no second program to install.
The attempt runs with `kill_on_drop`, so giving up a connect now kills its `ssh` (the gap the last
entry left open). `HostKeys` gains `Ask`, now the default; with no one to ask it behaves like
`Strict`, and then `BatchMode=yes` is kept, so nothing can wait on a terminal.

**Found on the way.** OpenSSH does not set `SSH_ASKPASS_PROMPT=confirm` for the host-key question,
only for some others, so it first arrived as a typed secret; the helper also takes any prompt
holding `(yes/no` as a yes or no. The test harness that stood in for the host never answered `ask`,
so the old strict-host-keys test waited out its timeout; the harness now answers as a user would
(declines, or trusts when a test says so), which kept that test's meaning and gave a test that a
trusted key connects and is remembered, through the real plugin. `scripts/vfs-gate` listed the two
new files, which only touch this machine's runtime folder, with that reason.

**Trade-offs.** Control folders are no longer cleaned by the crate: a failed attempt removes its
own, and a later attempt sweeps those ten minutes old that hold no control socket (longer than the
five minutes a question waits). A secret passes through the host as an ordinary JSON string, which
is not wiped; the `Password` type is, and so is the line the helper reads it from. `ssh` asks a
password three times, and a question times out after five minutes, neither of which can be set.

**Verification.** `scripts/check` passes. New tests: the wire and its cleaning (3); the request
parsing in the host (1) and through a fixture plugin, answered and declined (2); the popup's keys
(4) and its drawing, including that a typed secret never appears (3); the relay and helper (4) and
the folder, sweep, policy and log handling in `master.rs` (5); against a real `sshd` six more:
an unknown key trusted and remembered, refused and not remembered, backed out of, nobody to ask
(refused at once), a key passphrase right and wrong; and the same trust path through the real
`mman-ssh` as the host's harness sees it. The cancel test now also checks that the server sees the
connection close, and fails if `kill_on_drop` is removed. Driven by hand in the real `mman`
through a pty against a loopback `sshd`: `:cd ssh://127.0.0.1:<port>/…` shows the box with the
fingerprint, `n` ends in "cannot connect … Host key verification failed" with `known_hosts`
empty, and a second `:cd` with `y` lists the folder and writes the key. No `ssh` process was left
afterwards. Not driven by hand: a password login (the test `sshd` takes keys only; a key
passphrase takes the same path and is tested) and a one-time code.

**Left open.** Bookmarks and a history of machines; a limit or a setting for the tries and the
wait; trusting a *changed* key (`ssh` refuses and reports it, which is right); a mouse reply to the
box.

---

## 2026-10-18 — A menu to switch plugins on and off

**Decision.** Plugins were switched off only by editing `config.toml` and restarting: `git_status`
and `system_hud` for two of them, and for the rest by removing the program. Three courses were
weighed: a menu that switches plugins and keeps the choice (A); that plus adding and removing the
user's own plugins by form (B); and B plus installing, fetching and deleting programs from the
menu (C). Chosen: B, built in steps, the first being the switch itself. C would run third-party
code from inside the interface, and a plugin runs with the user's own permissions and has no
sandbox. The menu is core, not a plugin, so it still works when a plugin is broken. Deleting a
bundled program from the menu stays on the roadmap as its own last step, with a confirmation that
names the file and the command that reinstalls it.

**What changed.** `space p` opens the menu: one row per plugin, the user's `[[plugin]]` tables
first, then the bundled ones, with where each comes from and where it stands. `Enter`, `space`,
`h` or `l` switches the row: off stops the process on purpose (it is not a crash) and frees the
`:` words it held; on starts it. The switched-off names are kept as `disabled_plugins` in
`local.toml`, never in the hand-written `config.toml`, and a name left out means on. `PluginManager::remove`
and `Registry::forget` are the host's side. A row the menu cannot switch says why instead: a
bundled plugin whose program is not installed, and `git` and `hud` while `git_status` or
`system_hud` in `config.toml` decides them. A refused action (`f`, `L`, Inspect, compress and so
on) now says the plugin is switched off and where to turn it on, instead of how to install it.

**Trade-offs.** `hud` is off by default through `system_hud`, so the menu cannot turn it on until
that setting is moved into `local.toml`; the same holds for `git` with `git_status = false`.
Switching off a plugin that serves `ssh://` or archive paths while browsing one leaves the
browser to report the failure on its next read. A plugin written by hand in `config.toml` can be
switched off but not edited or removed from the menu.

**Verification.** `scripts/check` passes. New tests: the roster's rules, including a property test
on the saved list (6); the popup's keys, including a property test that a toggle is always in
range (4); `Registry::forget` (1); the config round trip (1); and, across a real process
boundary, that a removed plugin is stopped, forgotten, not restarted by its key and can be
spawned again (1). Driven by hand in the real `mman` through a pty with the config folder pointed
at an empty directory: `space p` listed eleven plugins, `Enter` on `finder` printed "switched off"
and wrote `disabled_plugins = ["finder"]`, `f` then said the finder plugin is switched off, and
switching it on again printed "switched on" and emptied the list. Not driven by hand: that the
finder's picker opens after being switched back on, and the menu in a window narrower than 80
columns.

**Left open.** Adding a plugin by form, removing one, deleting a bundled program, and the
manifest, drop-in directory and `mman plugin` commands under "Easier plugin authoring and adding".

---

## 2026-10-18 — `git` and `hud` in the plugin menu

**Decision.** What is left of the plugin manager was weighed as four small cycles in a set order
(A), the user's own plugins first (B), or everything in one go (C). Chosen: A, so each diff stays
reviewable and the destructive step, deleting a bundled program, is reviewed on its own. The order
is: `git` and `hud`, deleting a bundled program, keeping `[[plugin]]` tables in `local.toml` with
removal, then the add form. This is the first.

**What changed.** The menu used to lock `git` and `hud` whenever `config.toml` decided them. Now
`system_hud` and `git_status = true` only say whether they start on, and the menu switches them
like any other plugin. Most plugins start on, so `local.toml` keeps the names switched off
(`disabled_plugins`). The system figures start off, so a list of the ones that are off cannot say
"switched on"; a second list, `enabled_plugins`, does. Which list a plugin uses is its
`default_on`, and a name in the wrong list changes nothing. `git_status = false` is the one
setting the menu does not override: it exists for anyone who wants git never to run, so the row
stays locked and says why. `Config::save_disabled_plugins` became `save_plugin_switches`, which
writes both lists.

**Trade-offs.** `system_hud = false` is not a veto, unlike `git_status = false`. The example
config has always carried `system_hud = false`, so a veto would have locked the figures off for
everyone who copied it. A user who sets `system_hud = true` and later switches it off in the menu
has two places that say different things; `local.toml` wins, as it does for the other settings.

**Verification.** `scripts/check` passes. New and changed tests: the roster's rules for `git`
(veto and switchable) and for `hud` (off by default, switched on, started on and switched off, a
name in the wrong list ignored), the mirror helper for the on list, and the config round trip for
both lists. Not driven by hand in the real `mman` this time: the new rows were not toggled
through a pty, so the end-to-end switching of `hud` is covered by the unit tests only.

**Left open.** Deleting a bundled program, a user's own plugins in `local.toml` and their
removal, and the add form.

---

## 2026-10-18 — Deleting a bundled plugin's program

**Decision.** Three ways to delete a bundled program from the menu: `d` with a confirmation that
names the file, its size and the reinstall command (A), the same but moving the file to the trash
(B), or only printing the `rm` and install commands (C). Chosen: A, as the roadmap describes it.
The ownership check and the confirmation are the safeguards; B would leave a program in the trash
that cannot be read back on macOS, and C does nothing the user cannot already do.

**What changed.** `d` on a row of the plugin menu asks inline, replacing the hint line with the
question; only `y` deletes, and `n` or `Esc` drops the question without closing the menu, so a key
pressed for something else cannot delete. `plugin_program` finds the file (a path with a `/` as it
is, otherwise the first match on `PATH`), refuses anything that is not a plain file owned by the
running user, and names the reinstall command from the plugin's name. A link is refused rather
than followed, and `/usr/bin` programs from a package manager are refused because root owns them.
The file is checked again at the moment of `y`, the running plugin is stopped first, and the row
becomes one whose program is not installed. A plugin of your own is refused: its program is not
Minuteman's to delete.

**Trade-offs.** The reinstall command assumes a plugin's folder has the plugin's name, which is
true of all eleven bundled ones. Taking the user id needs one `unsafe` call to `geteuid`, the
first in the interface crate; the alternative, reading the owner of `/proc/self`, does not exist
on macOS. `scripts/vfs-gate` lists `plugin_program.rs` as a file that is local by nature, with the
reason, since a program on this machine's disk is not a path a `Vfs` serves.

**Verification.** `scripts/check` passes. New tests: the rule (regular file and owner, as a
property), a name found on a path and a path taken as it is, a file of the user's named and
deleted while its neighbour stays, a file of another user's and a link both refused and left, and
the popup (`d` asks, only `y` confirms, `n` and `Esc` keep the menu open). Not driven by hand in
the real `mman`: the inline question was not seen in a terminal, so its layout is untested.

**Left open.** A user's own plugins in `local.toml` and their removal, and the add form.

---

## 2026-10-18 — A user's own plugins in `local.toml`, and removing one

**Decision.** The roadmap fixed the shape (the menu writes only `local.toml`, never the
hand-written `config.toml`), so no COAs were asked. `RawLocal` gained `plugin: Option<Vec<
RawPluginSpec>>`, kept the way `disabled_plugins` is: `None` means "keep what is on disk", so the
appearance and settings popups, which build a `RawLocal` from scratch, cannot erase it.

**What changed.** `RawPluginSpec` is now public and serializable (empty lists skipped, so a saved
table reads like a hand-written one). `Config.plugins` is `config.toml`'s tables followed by
`local.toml`'s; `Config.local_plugin_names` says which came from where. A name already used
(by `config.toml`, or earlier in `local.toml`, or empty) is dropped from `local.toml`, since the
menu works on names and a hand-written entry must win. The roster's `Entry` gained
`kept_by_menu`. `d` on a row now means "remove": a bundled row deletes its program as before, a
`local.toml` row asks "Remove plugin X from the menu?" and `y` stops it, drops its row, and
calls `Config::forget_local_plugin`, which also takes the name out of both switch lists; a
`config.toml` row is refused with the reason. The program the plugin runs is never touched. The
cursor is clamped after a removal so it cannot sit on a row that no longer exists.

**Trade-offs.** One key for two kinds of removal keeps the menu's hint line short, at the cost of
the question text being what tells them apart. The file rewrite goes through the existing
`save_local` (whole-file re-serialize, which the file's own doc already accepts as it is never
hand-edited), so `local.toml` comments would be lost, as before.

**Verification.** `scripts/check` passes (one `vfs_remote` ssh test failed once with a refused
key and passed on the rerun and in the full run; it touches nothing changed here). New tests: a
table round-trips and lands after `config.toml`'s, a duplicate or empty name is not taken, forgetting
drops the table and its names and nothing else (the pure `without_plugin`, so the test never writes
the real config), the roster marks only `local.toml`'s plugins, and the cursor clamps. Driven in
the real `mman` through a PTY with an isolated `XDG_CONFIG_HOME`: three plugins (one in
`config.toml`, two in `local.toml`) listed; `d` showed the question, `n` kept it, `y` removed the
row and printed "removed"; `local.toml` then held the other plugin and `disabled_plugins`
untouched; `d` on the `config.toml` plugin said "declared in config.toml; remove it there".

**Left open.** The add form, which will write the same tables.

---

## 2026-10-18 — The plugin menu's add form

**Decision.** Three ways were put forward and COA A chosen: a form with one text row per field of
a `[[plugin]]` table, modelled on the compress form. A step-by-step prompt (B) cannot go back to
fix an earlier answer, and a `:plugin add` command (C) is not the form the roadmap promised and
makes args and lists awkward to quote. A form also suits the `plugin.toml` manifest still to come,
whose job will be to prefill these same rows.

**What changed.** `add_plugin_form` holds the rows and checks what the text alone can tell: the
name (letters, digits, `-` and `_`, 40 at most), a command, keys that `parse_key` accepts (now
public from `theming`), events `EventKind` knows, and arguments split on spaces except inside
double quotes. `PluginsPopup` owns the form: `a` opens it and, while it is open, it has every key,
so `q` or `d` typed into a name cannot close the menu or remove a row; `Outcome` is no longer
`Copy` because `Add` carries the table. `plugin_roster::conflict` is the pure check that needs the
rest of the program: a name any listed plugin has, bundled or not; a key the keymap binds or
another plugin fires; a `:` word in `BUILTIN_WORDS` or another plugin's. `App::add_plugin` runs it,
then starts the program, and only then writes `Config::add_local_plugin`, so a typo in the command
leaves nothing behind in `local.toml`. The row goes in after the user's own plugins and before the
bundled ones, which is where the next launch will list it. `provides` is not checked for clashes
on purpose: claiming a bundled plugin's action is how it is replaced.

**Trade-offs.** Typing every list into a text row is slower than choosing from a list, but the
only lists with a closed set are events and keys, and both are validated with the reason named.
A key or word that clashes is refused rather than left to the first plugin to claim it; the
registry would have ignored the second silently, which is harder to notice than a form that says no.
Arguments cannot hold a double quote; none of the bundled plugins need one.

**Verification.** `scripts/check` passes. New tests: the form reads every field from its row, says
what is wrong for six bad inputs and clears the message on the next key, wraps its focus; a
property test that a built table always has a valid name and parseable keys, and another on the
argument splitter; `PluginsPopup` opens the form with `a` even with no rows, gives it every key,
and hands back an add that a refusal keeps open with the reason; `conflict` for each kind of clash;
`with_plugin` keeps the others and clears a stale switch; the form is drawn over the list with its
hints, text and error. Driven in the real `mman` through a PTY with an isolated `XDG_CONFIG_HOME`:
`a` showed the form; `j` was refused with "key j is already bound to MoveDown" and the form stayed
open; `9` was accepted, the row appeared as "yours, on" and `local.toml` held the `[[plugin]]`
table with `on_key = ["9"]`.

**Left open.** The `plugin.toml` manifest and the rest of "Easier plugin authoring and adding".

---

## 2026-10-18 — Drop-in plugins and the `plugin.toml` manifest

**Decision.** Three ways were put forward for "Easier plugin authoring and adding" and COA B
chosen: the manifest and the drop-in folder together. The manifest alone (A) would have had no
reader but the add form, so its format would have gone untested against real use; the full
`mman plugin new/check/add` set first (C) leans on a manifest and a discovery rule that did not
exist yet, so it risked being redone. Two defaults were put to the user and accepted: a drop-in
whose name is already taken loses and is reported, and a drop-in is listed switched off until the
user switches it on, since a folder copied in starts a program.

**What changed.** `theming::dropin` reads `plugin.toml` with the keys of a `[[plugin]]` table
(flattened from `RawPluginSpec`, so the two cannot drift) plus a `description`. `name` falls back
to the folder's name; `{dir}` in `command` and `args` is the folder, and a `command` that begins
with `./` is taken from it. `DropIn::from_manifest` is the only way to get a `DropIn`, and it
refuses an empty or spaced name and an empty command, so the rest of the program never sees one.
`scan` goes through `plugins/` in the config directory in folder-name order, skips a folder with
no manifest without a word, and turns every bad one into a line instead of an error, so one broken
plugin hides no other. A manifest over 64 KiB is not read. `Config::load` runs the scan (the pure
`from_sources` does not, so its tests still touch no file) and the result reaches `App` through
`with_dropins`. `plugin_roster::with_dropins` lists them after the user's own plugins as origin
`drop-in`, `default_on = false`, so switching one on goes to the same `enabled_plugins` list as the
system figures and needs no new field in `local.toml`. A name that a user's or a bundled plugin
(installed or not) already has, or an earlier folder has, is left out with a line saying which
kind kept it. `d` on a drop-in says its folder is the thing to delete. `scripts/vfs-gate` lists
`dropin.rs` with the reason, as it does `config.rs`.

**Trade-offs.** Reusing the `[[plugin]]` keys means a plugin moves between a manifest and
`config.toml` by copying lines, at the price of calling the key list `on_key` rather than the
friendlier `keys`. A key or `:` word that a drop-in shares with another plugin is not checked when
it is found, as the keymap is not known there; the add form checks them, and the first plugin to
start keeps the key. `{dir}` is explicit where running the plugin with its folder as the working
directory would have been implicit, and would have changed what the plugin sees as its cwd.

**Verification.** `scripts/check` passes. New tests: every key of the manifest is read; the name
defaults to the folder's; `{dir}` and `./` resolve; six kinds of bad manifest are named; a
property test that whatever text is read, an accepted manifest has a name without spaces or
slashes and a command; a scan lists folders in order, passes over folders without a manifest,
reports the bad ones, keeps the first of two folders with one name, and treats a missing folder
as empty; a manifest over the cap is refused; in the roster, a drop-in is listed after the user's
own and off, on when `enabled_plugins` names it, and never takes a taken name.

**Left open.** `mman plugin new`, `check` and `add`; reloading plugin config without a restart; an API version in
the manifest; a check of keys and `:` words when a drop-in is switched on.

---

## 2026-10-18 — The add form takes a folder

**Author:** deltaog-117

**What.** The plugin menu's add form has a new first row, "Folder". Enter on it, or moving off it
with a new path in it, reads `<folder>/plugin.toml` and writes the plugin's fields into the other
rows, which stay editable. The manifest's description shows on the line under the rows.

**Why this shape.** Three ways were weighed: a Folder row on the existing form (chosen), a
two-step flow with a read-only preview, and the first plus path completion and a picker over the
drop-in scan. The first is the smallest change that matches "point at a path and the form fills
itself in", and the other two can be built on it.

**How.** The form stays pure, as `compress_popup` is: it never reads a file. `Key::Load(path)`
asks, `main` resolves the text (`resolve_dir`: `~`, a path relative to the browsed folder, and the
path of a `plugin.toml` meaning its folder), calls `DropIn::load`, and answers with
`PluginsPopup::fill` or `refuse`. That keeps one parser for the manifest, so `{dir}` and `./`
resolve exactly as they do for a drop-in found at launch. A path is asked for once: pressing Enter
on the same text again is an add, not a second read. The form now opens on the Folder row, so two
popup tests and the menu draw test press Tab before typing a name.

**Trade-offs.** The arguments row writes an argument with a space in double quotes, and has no
way to write one that holds a `"`; filling it would drop the quote and add a different plugin, so
`fill` refuses such a manifest and changes nothing. The saved command is the resolved one, absolute
and still showing `dir/./run.sh` for a `./run.sh` command; it runs, but is not tidy, and the
existing drop-in test asserts that spelling.

**Verification.** `scripts/check` passes. New tests: Enter on a new path asks once and a fill
round-trips to the manifest's own table; leaving the row loads a new or edited path and an empty
one loads nothing; a quote in an argument is refused and leaves the rows alone; path resolution
for `~`, relative, absolute, `plugin.toml` and no home; a property test that any arguments written
into the row read back unchanged. Against the real binary on a PTY through `pyte`: `space p a`, a
typed folder and Enter filled six rows with the arguments as `--x "a b"`; a manifest key the
interface already binds was refused with the reason; a free key was added, the status said
`plugin blame: added`, and `local.toml` held the table.

**Left open.** `mman plugin new`, `check` and `add`; reloading plugin config without a restart; an
API version in the manifest; a check of keys and `:` words when a drop-in is switched on.

---

## 2026-10-18 — Adding a plugin by picking, not typing

**Context.** The add form had eight text rows and only the Folder row could save typing. The goal
was an add flow that asks for nothing typed: the user points at a plugin and the manifest supplies
the rest, the way a drop-in already does.

**Decision.** Chosen as COA B of three: a found list and a review card (A), plus choosing the
folder in the file browser itself. The popup's old `form` became an `Add` flow with three stages,
`Found`, `Review` and `Form`; the form is the "Manual…" row at the end of the list and is
otherwise unchanged. `plugin_source::find` looks at the browsed folder and the folders directly
in it (one level, 500 at most, so a folder of repositories cannot slow the list), and the caller
attaches each plugin's `conflict` reason before the user sees it, so the card can refuse an add
before anything is started.

**Choosing a folder.** The chooser (`--chooser-file`) is a launch mode that writes an answer to a
file, so it could not be called from inside a running app. Instead the popup has a `picking` flag:
while it is set the menu draws nothing and hears nothing, the status bar shows `PICK PLUGIN`, and
`main` gives the browser the keyboard except for two keys. `Enter` asks `plugin_source::choose`
what the row under the cursor is: a folder with a manifest is chosen, any other folder is entered
by the browser as usual, and a file is refused (it would otherwise be opened). `Esc` ends the
pick. The mouse reaches the browser too while picking, because the menu's "any click closes it"
rule is now skipped. A double-click enters a folder rather than choosing it; choosing is `Enter`.

**Start and test.** `PluginHost::spawn` returns once the process is started, and the handshake
arrives later on the render loop, so the test starts the program, stops it again and reports
`starts`. It does not check the handshake or the API version; that belongs to `mman plugin check`,
which is still open.

**Trade-offs.** `Esc` on the review card goes back by scanning the browsed folder again, so a
plugin chosen from another folder is not on the list after backing out of it. The card is
read-only: to change a keybinding the user edits the manifest, or picks "Manual…". `scripts/vfs-gate`
gained an entry for `plugin_source.rs`, because plugin folders are programs run on this machine.

**Verification.** `scripts/check` passes. New tests: the found list orders the folder itself
first and its subfolders by name, looks one level down only, leaves out listed and unreadable ones,
and finds every plugin folder exactly once (property test); `choose` returns chosen, enter,
refused and not-a-folder; the popup tests cover review, test, refused add, cancel, picking hiding
the menu, and a refused folder explaining itself; two draw tests check the found list and the
review card. Against the real binary on a PTY through `pyte`, with `HOME` pointed at a scratch
folder: the list showed `bad` (cannot be added yet) and `blame`, the card for `bad` showed its
key conflict and `Enter` on Cancel went back, the card for `blame` showed `cat`, `:blamer` and the
buttons, `t` then `Enter` said `blame starts`, Add wrote the `[[plugin]]` table to `local.toml`,
`Tab` showed the `PICK PLUGIN` pill with the menu hidden, `Enter` on `far` went in and `j` `Enter`
on `gem` opened its card, and `Esc` returned to the list.

**Left open.** A handshake check in Start and test; `plugin_dirs` in `config.toml` to scan folders
the user names; a double-click that chooses; `mman plugin check` and `add`.

### Plugin Folder Picker: `Q` Chooses the Folder You Are In

**Problem.** Browsing for a plugin folder chose only with `Enter`, and `Enter` on a folder without
a `plugin.toml` went into it. Standing inside the plugin folder, nothing chose it, and a folder with
no manifest never said why it was not a plugin.

**Decision.** `Q`, the key the `--pick` folder chooser already uses, chooses the folder under the
cursor, or the browsed folder when the cursor is on a file (`plugin_source::choose_here`). It never
goes into a folder, so a folder with no readable manifest is refused with the reason.

**Verification.** `scripts/check` passes. New test: `Q` takes a plugin folder under the cursor,
refuses a plain one by name, takes the browsed plugin folder from a file, and refuses with nothing
under the cursor.

### Large `.tar.gz`: a Time Budget and a Listing Kept on Disk

**Problem.** A `.tar.gz` cannot be listed without inflating it end to end, so every open of a big
one paid the full cost again whenever the archive plugin started afresh, and one that inflated to
more than 512 MiB could not be opened at all.

**Options.** (A) keep the listing on disk, keyed by the archive's path, size and time; (B) record
gzip checkpoints so a read can resume mid-stream, which needs a deflate decoder that exposes its
window; (C) stream the listing in batches over the plugin protocol. A was chosen with a change to
the cap: B is the real cure but adds a codec's worth of new code reading untrusted input, and C
changes the protocol every plugin shares.

**Decision.** The byte cap became a time budget (`INDEX_BUDGET`, 120 s): what an archive costs is
the work of inflating it, and the memory it can fill was already bounded by the entry limit. The
indexer records the entries it reads (`Index::build_recording`); a `.tar.gz` that took 250 ms or
more has them written to the cache folder (`cache.rs`), atomically and private to the user, and
on the next open they are replayed through the same builder (`Index::from_entries`), so the entry
limit and the tree's rules hold for stored entries too. Any problem with the cache (missing,
damaged, another version, another size or time) is a miss and never an error. `cache.rs` is in the
`vfs-gate` allow-list with the reason, as the app's own cache folder is local by nature.

**Verification.** `scripts/check` passes. New tests: a listing is stored and read back, a changed
size or time or another path is a miss, a damaged file is a miss and the next store replaces it,
the folder and files are private and only 32 stay, a stored listing is what a second run lists
and a changed archive is read again, a quick archive and the other formats leave nothing on disk,
and a `.tar.gz` past its time is an error. The ignored benchmark: a 1 GiB archive opens cold in
about 0.3 s and from the cache in about 0.1 ms.

**Left open.** The first open of a very large archive, and opening a file inside one, still
inflate from the start; checkpoints (B) would remove both. A hand-edited cache file can show a
wrong listing, since the cache is trusted as the user's own.

---

### Plugin Start Check: a Passive Half-Second Probe

**Date:** 2026-10-18
**Author:** deltaog-117
**Status:** Confirmed

**Problem.** "Start and test" in the plugin review card only showed that the program could be
spawned; it was then dropped at once. There was nothing to verify beyond that, because a plugin
never answers `init`: each bundled plugin compares `api_version` and, on a mismatch, logs an error
and exits. The manifest also could not say which protocol version a plugin was written for.

**Options.** (A) a passive probe: start, send `init`, watch for a short window, stop; report that
it stayed up, finished, crashed, or logged an error; (B) an additive `ready` message from plugin to
host, which needs a change in all nine bundled plugins and leaves older ones "unknown"; (C) a
static check with no process (command resolves, manifest parses, version in range, no collisions).
A was chosen: it says something new about a running program without touching the wire that every
plugin shares, and its report type can carry B's `ready` later.

**Decision.** `plugin_host::probe` is std-only: a reader thread forwards the plugin's stdout lines
over a channel, and the wait is one timed receive of 500 ms, not a poll, so it costs nothing
between uses and ends the moment the process does (the output closing). The verdict is `Stayed`,
`Finished` (status 0 is allowed, as everywhere else), `Crashed(status)` or `ClosedOutput`, plus
every `error`-level log line; any logged error fails the check even if the exit was clean. The
process is always killed and reaped. The manifest gained an optional `api_version`; the review
card shows a mismatch as the candidate's issue, which already blocks Test and Add before anything
starts. `mman plugin check <name or folder>` takes a folder holding a `plugin.toml` or a name the
menu lists, runs the same probe, and for a manifest also checks the version and the keys and `:`
words against the interface and the other plugins (the plugin's own roster entry excluded); it
exits 1 on any problem.

**Verification.** `scripts/check` passes. New tests: the probe against `sh` programs that stay,
exit 0, exit 3, log an error then exit 0, close their output and run on, and do not exist; the
probe returns long before its window when the process exits; `init` reaches the program; a
property test that a report is healthy only when nothing was logged and the program stayed or
finished cleanly; the manifest's `api_version` parsed, optional and typed; the version mismatch
text; the CLI grammar; `check` on healthy, wrong-version (nothing started), crashing, missing,
conflicting, malformed and unknown targets; and `test_plugin` and `candidate` on the app. By hand
against the compiled `mman`: `plugin check git` (the real `mman-git`) starts and keeps running.

**Left open.** "Start and test" still runs on the interface's thread, so a healthy plugin holds the
screen for the half second. The version check is not applied when the add form is filled from a
folder, and `check` is not given a plugin's keys and words for a bundled or `config.toml` plugin
(they were settled at load). `mman plugin new`, `plugin_dirs`, a double-click to choose a folder
and reloading config by hand-edit are unchanged.

## 2026-10-18 — Start and test runs in the background

**Context.** "Start and test" in the plugin review card called the half-second probe on the
interface's thread, so a healthy plugin froze the screen and every key for the whole window.

**Decision.** Chosen as COA A of three: a `ProbeJob` (`probe_job.rs`) in the shape of `NavJob`,
rather than a bare thread owned by the popup (B) or running the probe through the plugin host's
event stream (C), which would blur a passive probe with a running plugin. `App::test_plugin` became
`begin_plugin_test` (the conflict check, then the job starts and returns at once) and
`poll_plugin_test` (the main loop collects the verdict). The card shows `testing <name>…` meanwhile.
A second Start and test while one runs is refused with "still testing <name>". Leaving the card,
closing the menu or clicking away drops the job, and a verdict is shown only on the card of the
plugin it belongs to. The probe has no cancel token, so a dropped job's program is stopped by the
probe itself at the end of its window.

**Verification.** `scripts/check` passes. New tests: starting returns long before the window ends
and the verdict follows; a program that ends at once gives a failed verdict. The app's start-and-test
test now goes through `begin_plugin_test` and `poll_plugin_test`.

**Left open.** The version check is not applied when the add form is filled from a folder;
`mman plugin new`, `mman plugin add`, `plugin_dirs`, a double-click to choose a folder and reloading
config by hand-edit are unchanged.

## 2026-10-18 — `mman plugin new` and `mman plugin add`

**Context.** A plugin could be checked from the shell but not started or installed from it: a new
author wrote `plugin.toml` and a program from nothing, and the only way to install a folder was the
plugin menu's picker.

**Decision.** Chosen as COA B of three: `new` plus `add` for a local folder, over `new` alone (A) and
over also cloning a git URL (C). C fetches third-party code that runs with the user's permissions
with no sandbox to hold it, which the roadmap already rules out. The `rust` skeleton is left out: the
generated crate would need `plugin_api`, which is not published, so it would build only inside a
checkout. `PluginName` (`plugin_new.rs`) is a newtype whose only constructor accepts ASCII letters,
digits, `-` and `_`, so the generated manifest and program never need quoting and a name cannot be
`..` or hidden; it is stricter than a manifest requires on purpose, and `add` applies it to a
manifest's name too, since that name becomes a folder. The skeleton program answers the `command`
event by matching `"name":"command"` in the line, which needs no JSON parser in `sh`: the host
writes the name first and escapes any quote inside a path, so the text cannot appear inside a value.
`add` runs the checks `plugin check` runs (API version, name, keys, `:` words) before it copies,
copies under a staging name and renames into place, and bounds the copy because the folder is
somebody else's (no links or special files, no `.git`, 5,000 files, 64 MiB, 16 levels). It copies
rather than links so the plugin survives the original moving. `Config::dropin_dir` became public so
the folder is named in one place. Both new files are on the `vfs-gate` list, as they write to the
app's own config folder.

**Verification.** `scripts/check` passes. Property tests: any accepted name yields a manifest the
loader reads back; a copy holds every file with its contents. Unit tests cover the name rules, both
programs answering a `command` event and nothing else (the Python one is skipped on a machine with no
`python3`), an existing folder left alone, a failed write leaving nothing, links, the depth limit,
keys and versions refused before copying, and the command-line forms. One existing test used `plugin
new` as its example of an unknown subcommand; it now uses `plugin remove`. By hand against the
compiled `mman` with a throwaway config folder, then over a PTY with both plugins switched on: `:hello`
and `:py` each put `hello from <name>` on the status bar. The switch was set in `local.toml`, not by
clicking through the menu.

**Left open.** The `rust` skeleton, `add` from a git URL, `plugin_dirs`, a double-click to choose a
folder, the version check when the add form is filled from a folder, and reloading config by hand-edit.

---

## 2026-10-18 — `plugin_dirs`

**Context.** The plugin menu's found list scanned only the folder being browsed, so a person who keeps
their plugins in one place had to browse there every time.

**Decision.** Chosen as COA A of three: a `plugin_dirs` list in `config.toml`, read at startup, whose
folders are scanned after the browsed one with the same one-level rule. B (a key in the picker saves
the folder to `local.toml`) is a larger surface (a key, a save, a way to remove) and can be added on
top of this list without changing it. C (scanning in the background into the roster) changes the model
the roster and the popup depend on. `Config::plugin_dirs` keeps the text as written; the expansion
lives in `plugin_source::configured_roots`, which takes `home` as an argument, so it is tested without
touching the environment. It drops a blank or relative entry because a relative folder would mean
something different for each browsed folder. `find_all` lists a plugin folder once even when the
browsed folder is one of the configured ones, and applies the menu's `skip` list to all of them.

**Verification.** `scripts/check` passes. New tests: the expansion (home, trimming, relative and
repeated entries, no home), the scan order and the single listing with a missing folder in the list, and
the config field's default and parsing. Not tried by hand in the running interface.

**Left open.** Adding or removing a folder from the menu, and reloading `config.toml` without a restart.

---

### Plugin Quick-Start Page

**Date:** 2026-10-19
**Status:** Confirmed

**Context.** `plugin new`, `plugin check`, `plugin add` and `plugin_dirs` landed over four commits, and
the only place that described them together was the roadmap and the changelog, which are written for
the maintainer, not for someone who wants to write a plugin.

**Decision.** A single page, `docs/plugins.md`, in the order an author meets things: make, check,
switch on, manifest, conversation, add, where the menu looks. It was written after running each
command against the built binary with a throwaway config folder, so the output lines it quotes
(`hello starts and keeps running`, `written for plugin API 99; this build speaks 1`, the exit codes)
are the real ones. It states only what was seen; the full event list and the canvas and `ask`
requests are left out until they have a reference of their own. The README links to it from the
plugin section.

**Verification.** `scripts/check` passes. No code changed.

**Left open.** A reference for every event and request a plugin can use, and a Rust skeleton.

---

### Sandboxed WASM Plugins: a Read-Only First Cut

**Date:** 2026-10-21
**Status:** Confirmed

**Context.** Plugins are programs run with the user's own permissions. That is right for code the
user wrote, and wrong the day someone shares a plugin with a stranger. The roadmap held a sandboxed
tier back until that day; the plugin commands are now complete enough that the tier can be built
against a stable protocol instead of guessing.

**Options.**

| COA | Approach | Advantages | Disadvantages | Difficulty |
|---|---|---|---|---|
| A | `wasmi`, same line protocol over in-memory pipes, read-only trust | Small, pure Rust, no change to what a plugin says | Slower than a JIT; no writes yet | M |
| B | `wasmtime` or Extism with a host-function API | Fast; ready-made plugin kit | Roughly doubles the binary; a second API beside the protocol | H |
| C | A, plus a per-plugin permission manifest allowing writes now | Covers more plugins | A permission format that must then be kept stable, designed before any user asked | H |

**Decision.** A. The module talks the protocol a process plugin already talks, so the host's
request handling is shared and only the transport differs. The feature is off by default because
nothing else needs `wasmi`, and a build without it says how to turn it on rather than failing
quietly. `launch.rs` decides process or module once, so nothing after it can mix the two.

What a module may ask is settled in one place, `Trust::check`. Its match has no wildcard arm, so a
request added to the protocol later fails the build until someone decides about it. The first cut
answers reads below the browsed folder (after `canonicalize`, so a link or `..` that leads out is
refused), the interface requests, and refuses every change to a file and `run_action`, which can
reach a key's delete. The refusal carries a reason the plugin can log. The check happens before the
host reads, so a link swapped in by another program between the two could still get through; a
module cannot make links itself, which is the part this can promise.

The module runs on its own thread because the WASI layer is synchronous. Memory is capped at
64 MiB. Time is metered in slices of fuel, and after each slice the thread checks whether the host
asked it to stop and whether the module has read its input within one second. A module waiting for
input burns no fuel, so a plugin that sleeps between events for hours is fine; one that computes for
over a second without reading is ended, with the reason reported. The limit is wall-clock time
checked at slice ends, so a slow build only notices it later.

**Measured** (release build, `wasm_cost`, run by hand): a module starts to its first log line in a
median of 312 µs over 25 starts, and one `get_state` round trip takes a mean of 31.5 µs over 2,000.
The binary-size cost (estimated at about 2 MB) was not measured on a release build.

**Verification.** `scripts/check` passes, now including clippy and tests with `--features wasm`.
New tests: a symlink in the browsed folder that leads out, a delete refused inside the browsed
folder, a read outside it refused while a `:` word can still be registered, a module that hogs the
CPU, a file that is not a module, ignored arguments, and the build-without-the-feature message.
`vfs-gate` allows `sandbox.rs` and `wasm.rs` their local-disk access with a reason each. Not tried
by hand in the running interface, and no module compiled from Rust or another language has been run;
the tests use hand-written WAT.

**Left open.** Writes behind a permission manifest, an API version handshake for modules, a worked
Rust example, and the binary size on a release build.

---

### Rust Example WASM Plugin

**Date:** 2026-10-22
**Status:** Confirmed

**Context.** The sandbox shipped with tests written in hand-made WAT, so nothing compiled from a
real language had ever run on it. A plugin author also had no Rust example to copy.

**Decision.** `examples/plugins/wasm_stats` is a plain Rust program on `std::io`, built for
`wasm32-wasip1`. It is outside the main workspace on purpose (own `[workspace]` table): the
workspace builds for the host, a plugin should show only what a plugin needs, and it depends on
`serde_json` alone, not `plugin_api`, whose dependencies were not built for that target. The
cost is that the request and event shapes are written as JSON in the example instead of shared types;
the unit tests in the example pin the shapes it relies on.

A test in `plugin_host` builds it and runs it through `PluginManager`. It skips, saying why, when the
rust target is not installed: the target is a fact about the machine, and making `scripts/check`
fail without it would block every contributor who never touches plugins.

**Verification.** `scripts/check` passes; the example's own tests and clippy pass; the module is
about 100 KiB; the new test passed against the real sandbox (the handshake reply, the folder
count, and the refused write leaving no file); `mman plugin check` on the folder says it starts and
keeps running. Not tried: the module in the running interface by hand.

**Found.** Nothing wrong in the sandbox; a module compiled by rustc started on the first run.

**Left open.** The host does not yet refuse a module that declares another API version (the example
does it itself), writes behind a permission manifest, and the release-build size cost.

---

### Add Form Refuses a Manifest for Another API Version

**Date:** 2026-10-23
**Status:** Confirmed

**Context.** Filling the add form from a folder read the manifest but never looked at its
`api_version`, so a plugin written for another protocol could be filled in and edited until Add,
where it was refused late. The found list, `mman plugin check` and `mman plugin add` already
refused it.

**Decision.** `AddPluginForm::fill` now asks `plugin_roster::api_mismatch` first and returns the reason, which
the popup already shows under the form. It refuses before touching any row, so what the user had
typed stays. This was the smallest place: one check at the point the manifest enters the form, with
the same message and the same function as the other three paths, so they cannot drift apart.

**Verification.** `scripts/check` passes; a new test covers a mismatch leaving the rows alone and
a matching version filling them. The first full run showed one failure in the `trash` crate's
restore test that did not repeat on a clean tree or on the next run. Not tried: the form by hand.

**Left open.** The check of keys and `:` words when a drop-in is switched on.

### Undo for Compress and Extract

**Date:** 2026-10-23
**Status:** Confirmed

**Context.** The plan was to record a compress and an extract with the existing `Created` and
`Trashed` changes. Reading `history.rs` showed that was wrong for the output: `Created` is removed
only while it is empty and is redone as an empty file, and an archive or an unpacked folder is
neither.

**Decision.** A new `Change::Produced { path, product, held }`. Undo sends the output to the trash
and keeps the handle in `held`; redo restores it from there. To keep that handle, `Change::revert`
now returns the change as it stands, as `apply` already did, and an undo batch hands the refreshed
changes to the redo stack. The interface records a job's output only if the path did not exist when
the request started and the plugin lists the job as finished, so a replaced or skipped output, which
cannot be put back, and a cancelled job are never recorded. The originals a compress trashes, and
the archive an extract trashes, are now tracked like any trash, so undo restores them. Considered
and set aside: recording only the trashed originals, which would leave an undone compress with its
archive still on disk. The key for `:history` is `U`, which nothing else uses.

**Verification.** `scripts/check` passes. New tests cover undo and redo of an archive and of a
folder with contents, a missing output staying on the stack, and the interface not recording a
replaced or cancelled job. Not tried: the real `mman-archive` by hand, or a compress that trashes
its originals through the form.

**Left open.** Undoing one entry on its own, an overwrite, a permanent delete, and a history that
survives a restart.

### Event-Driven Refresh

**Date:** 2026-10-23
**Status:** Confirmed

**Context.** The live refresh re-listed the browsed folder twice a second, which costs a `readdir`
and a `stat` per entry and lags a change by up to half a second. The roadmap asked for a watch
method on `Vfs` so a remote backend can offer its own. Three courses of action were weighed: a
watcher in `tui` that only nudges `LiveRefresh`, a `Vfs::watch` method, and applying each event as
a patch to the listing.

**Decision.** `Vfs::watch(path, wake) -> Option<WatchGuard>`, default `None`. The watcher is only a
cue: `wake` sets a flag and the existing list-and-compare code stays the source of truth, so a
lost event, an inotify overflow or a backend without watching costs only latency, never
correctness. `notify` is an optional dependency of `shared` (`watch` feature, on only in `tui`) so
the plugin programs do not link it, and it is built without its default features. Access events are
dropped, because listing a folder opens it and would announce itself in a loop. Announced listings
are spaced 100 ms apart; the safety net is 5 s for a watched folder. Patching the listing from
each event was set aside: rename pairs and overflow need a full re-list anyway.

**Verification.** `scripts/check` passes. New tests: a created file wakes the caller, listing does
not, a dropped guard stops the wakes, a missing folder cannot be watched, a watched change shows
within 300 ms where polling would take 450, and a backend that declines is polled. In a pty run of
the real release binary, eight files made from outside appeared after 0.11 to 0.19 s each, and
0.14 to 0.58 s with the watcher compiled out. The binary grew from 9,839,568 to 10,059,728 bytes,
more than the 50 to 150 KiB guessed beforehand.

**Left open.** Watching the preview column's folder, a remote or archive folder, and telling the
user when the watch limit is spent.

---

### Undo One History Entry on Its Own

**Date:** 2026-10-24
**Status:** Confirmed

**Context.** The history popup could only go back to a row, undoing every entry newer than it.
Three courses of action were weighed: run the chosen entry alone and let each step's own checks
refuse a conflict (A), also undo the later entries that touch the same paths (B), and refuse up
front when a later entry shares a path (C).

**Decision.** A. `History::begin_undo_at(depth)` and `begin_redo_at(depth)` take one entry out of
its stack, and `Batch` remembers its position so what is left of a refused or cancelled entry goes
back to that place instead of the top. `u` on a popup row asks for it; `Enter` keeps meaning "go
back to here". Every `revert` already checks its target and aborts instead of overwriting, so an
entry whose later neighbour changed the same path fails with the existing message and nothing is
lost. B was set aside as a surprising side effect, C because it would duplicate those checks and
could refuse an undo that would have worked.

**Verification.** `scripts/check` passes. New tests: an entry in the middle is undone and lands on
the redo stack while the newer one stays, a refused middle entry returns to its own place and the
file is untouched, a depth past the stack takes nothing, and `u` on an undo and a redo row asks
for that row alone.

**Left open.** An undone middle entry sits on top of the redo stack, so redoing it comes before
older redo entries; mouse selection of a single row.

---

### Undo History Between Sessions

**Date:** 2026-10-25
**Status:** Confirmed

**Context.** The undo and redo stacks lived in memory, so closing `mman` lost them. Three courses
of action were weighed: a hand-written line format in `file_ops` saved after each change (A),
`serde` and JSON with a mirror type for the trash handle (B), and saving only on exit and on a
timer (C).

**Decision.** A. `History::encode` and `decode` are pure text functions in a child module of
`history`, so they reach the private stacks without widening anything; the tui owns the file and
the thread. A needs no dependency, the format is a few line kinds, and `Trashed` wraps a foreign
type that serde could not derive anyway. C was set aside because a crash is exactly when undo is
wanted. Four choices follow from what a saved handle means. A trash handle is the trash's own name
for an item, so on load it is kept only when an item with that name and time is still listed
(`trash::entries`, read once for the whole file), and an entry that names a gone item is dropped
whole instead of failing at undo time. A handle in the redo stack is already spent (the item was
restored by the undo), so none is written and a placeholder (`Trashed::spent`) stands in. A path
that is not absolute (another machine's) or not UTF-8 is never written, so a restored change cannot
run against whatever folder the next session starts in. A batch still being recorded is written as
the entry it will become, without the redo stack, matching what committing it does; an undo or redo
in flight is not written at all, since its entry is on neither stack until it ends. `History` got a
revision counter so the loop writes only when something changed. The writer keeps the newest text
it was handed, writes it to a temporary file and renames it over the real one, with mode 0600 in a
0700 folder because the file lists the user's paths.

**Verification.** `scripts/check` passes except `a_plugin_that_stops_without_an_answer_ends_the_operation_after_a_moment`,
which also fails on the commit before this change in this environment (its `sh` stand-in plugin
never exits within 5 s). New tests: every kind of change round-trips, a gone or reused-name trash
item drops its entry and keeps the others, a pending batch is written without the redo, a remote or
non-UTF-8 path is never written, a damaged line drops only its entry, a handle in the wrong stack
is refused, the capacity keeps the newest, other text is refused by its first line, property tests
for the round trip over awkward path characters and for `decode` never panicking, a real trashed
file restored by a history that was saved and loaded, file modes, a failed write reported once, and
no temporary file left. In a pty run of the debug binary, a `:touch` was saved at quit, listed by
`:history` in the next session, undone from the popup there with `u`, and listed under redo in a
third session.

**Left open.** Two `mman` at once overwrite each other's history; a file from a newer format is
read as empty and then overwritten; a write that fails is reported in the status line but not
retried until the next change.

---

---

## 🧠 Usage Guidelines

Write a new entry here before committing to a major design choice (new dependency, new crate
boundary, UI library, etc.), or when revisiting/reversing a previous decision.
