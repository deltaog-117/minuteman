# Minuteman

**Fast as its namesake militia: a Ranger-inspired terminal file manager in Rust, ready at a
moment's notice.**

---

## 💡 About

Minuteman is a terminal file manager for Linux, inspired by [Ranger](https://github.com/ranger/ranger)
but built in Rust to fix the thing that makes Ranger painful in practice: it gets slow fast once
you're moving many files or large ones. It's named after the American Minutemen militia, prized
for being combat-ready at a moment's notice — the same idea applied to file operations.

This is early — v0.1.0 established the project's architecture, and every High Priority roadmap
item (core file operations, async bulk ops with progress, the interactive shell overlay, the full
theme system, and inline image preview) is now built. It is **not yet** a daily-driver
replacement for Ranger or Yazi. See [ROADMAP.md](ROADMAP.md) for what's still ahead (built-in
trash, native SSH/SFTP browsing, and a stable multi-language sandboxed plugin system).

---

## ✨ Features (v0.1.0)

- 🔹 **3-pane miller-column browser** – Ranger-style parent / current / selection-preview layout.
- 🔹 **Vim-style navigation** – `j`/`k`/`h`/`l`/Enter/`q`.
- 🔹 **Async bulk file operations** – paste and delete run in the background (`tokio`), so a
  large copy/move/delete never freezes the app; copy/move show live progress and are cancellable
  with `Esc`. Permanent delete (with confirmation), rename, and create file/directory round out
  the core operations, with an overwrite/skip/abort prompt on conflicts.
- 🔹 **Interactive shell overlay** – `s` drops you into a real, fully interactive `$SHELL` in the
  browsed directory; `exit` returns to the TUI exactly where you left it, not Ranger's
  auto-close-after-one-command.
- 🔹 **Themeable UI** – a real palette (borders, titles, directory/file colors, selection
  highlight, status bar), not just two colors. `[theme] name = "dracula"` swaps the whole
  palette; individual color fields still override it.
- 🔹 **Inline image preview** – select a `.png`/`.jpg`/`.gif`/etc. file and it renders directly in
  the preview pane, using your terminal's best available graphics protocol (Kitty, iTerm2,
  Sixel) or Unicode halfblocks as a universal fallback. Decoding and rendering never block the UI.
- 🔹 **Config-driven keybindings** – read from `~/.config/minuteman/config.toml`, with per-field
  fallback to built-in defaults if the file is missing, partial, or fails to parse.

---

## 📋 Requirements

- Rust 1.85+ (edition 2024)
- A terminal emulator

---

## 📦 Installation

```bash
git clone https://github.com/<username>/minuteman.git
cd minuteman
cargo build --release
```

---

## 🚀 Usage

```bash
cargo run -p tui
# or, starting in a specific directory:
cargo run -p tui -- /path/to/dir
```

### Default keybindings

| Key         | Action                                       |
|-------------|-----------------------------------------------|
| `j`         | move down                                     |
| `k`         | move up                                       |
| `l` / Enter | enter directory                               |
| `h`         | leave directory                               |
| `q`         | quit                                          |
| `y`         | yank (copy) selection                         |
| `m`         | cut (move) selection                          |
| `p`         | paste                                         |
| `d`         | delete selection permanently (confirm `y`/N)  |
| `r`         | rename selection                              |
| `n`         | create — trailing `/` makes a directory       |
| `s`         | shell — drop into `$SHELL` in the current dir |
| `/`         | search — jump to the first matching entry as you type |
| `:`         | command — `:q`/`:quit` to exit, `:cd <path>` to jump to a directory |

While a prompt is active (rename/create/delete-confirm/conflict/search/command), `Enter` submits
and `Esc` cancels; the keybindings above are not resolved until the prompt closes. `Esc` while
searching also restores the selection you had before the search started. While a paste or delete
is running in the background, every key except `Esc` (cancel — copy/move only; delete can't be
cancelled mid-flight) is ignored until it finishes, including `q`.

---

## ⚙️ Configuration

Minuteman reads `~/.config/minuteman/config.toml` if present. Any key you don't specify keeps
its default.

```toml
[keys]
move_down = ["j"]
move_up = ["k"]
enter = ["l", "enter"]
leave = ["h"]
quit = ["q"]
yank = ["y"]
cut = ["m"]
paste = ["p"]
delete = ["d"]
rename = ["r"]
create = ["n"]
shell = ["s"]
search = ["/"]
command = [":"]

[theme]
# Selects a built-in base palette ("default" or "dracula"); an unrecognised name falls back
# to "default". Any color field below overrides just that field on top of the chosen palette.
name = "default"
selection_bg = "blue"
selection_fg = "white"
border_fg = "gray"
title_fg = "white"
dir_fg = "blue"
file_fg = "white"
status_fg = "gray"
```

Colors are one of: `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, `gray`
(`grey` also works). An unrecognised color name resets to the terminal's default.

---

## 📁 Project Structure

Feature-first Cargo workspace — each crate owns one capability and depends only on `shared`:

```
crates/
├── shared/          # Vfs trait + local filesystem implementation (read-only infra)
├── browser/          # miller-column navigation state
├── theming/           # config/keybinding/theme loading
├── tui/                # binary crate — render loop, input dispatch, wiring
├── file_ops/           # copy/move/delete/create/rename on Vfs, with progress/cancel support
├── shell_overlay/      # spawns a real, interactive $SHELL, inheriting stdio directly
├── preview/            # detects + decodes image files (terminal-agnostic; text preview: WIP)
├── trash/              # (stub, WIP) trash + undo history
├── plugins/            # (stub, WIP) WASM (Extism) multi-language plugin host
└── vfs_ssh/             # (stub, WIP) native SSH/SFTP remote browsing
```

---

## 🧪 Testing

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
```

---

## 📜 Roadmap

See [ROADMAP.md](ROADMAP.md) for priorities, and [DIARY.md](DIARY.md) for the reasoning behind
the architecture and key design decisions.
