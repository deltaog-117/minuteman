# Minuteman

**Fast as its namesake militia: a Ranger-inspired terminal file manager in Rust, ready at a
moment's notice.**

---

## 💡 About

Minuteman is a terminal file manager for Linux, inspired by [Ranger](https://github.com/ranger/ranger)
but built in Rust to fix the thing that makes Ranger painful in practice: it gets slow fast once
you're moving many files or large ones. It's named after the American Minutemen militia, prized
for being combat-ready at a moment's notice — the same idea applied to file operations.

This is early — v0.1.0 is a bare-bones, runnable local filesystem browser that establishes the
project's architecture. It is **not yet** a daily-driver replacement for Ranger or Yazi. See
[ROADMAP.md](ROADMAP.md) for the full plan (async bulk file ops, image preview, an interactive
shell overlay, native SSH/SFTP browsing, and a stable multi-language sandboxed plugin system).

---

## ✨ Features (v0.1.0)

- 🔹 **3-pane miller-column browser** – Ranger-style parent / current / selection-preview layout.
- 🔹 **Vim-style navigation** – `j`/`k`/`h`/`l`/Enter/`q`.
- 🔹 **Core file operations** – yank/cut/paste, permanent delete (with confirmation), rename, and
  create file/directory, all driven from the keyboard with an overwrite/skip/abort prompt on
  conflicts.
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

While a prompt is active (rename/create/delete-confirm/conflict), `Enter` submits and `Esc`
cancels; the keybindings above are not resolved until the prompt closes.

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

[theme]
selection_bg = "blue"
selection_fg = "white"
```

---

## 📁 Project Structure

Feature-first Cargo workspace — each crate owns one capability and depends only on `shared`:

```
crates/
├── shared/          # Vfs trait + local filesystem implementation (read-only infra)
├── browser/          # miller-column navigation state
├── theming/           # config/keybinding/theme loading
├── tui/                # binary crate — render loop, input dispatch, wiring
├── file_ops/           # copy/move/delete/create/rename orchestration on top of Vfs
├── preview/            # (stub, WIP) text + image preview
├── shell_overlay/      # (stub, WIP) interactive $SHELL overlay
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
