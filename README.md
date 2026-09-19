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
- 🔹 **Ranger-style marks** – `v` toggles a mark on the current entry; `d` (delete) acts on every
  marked entry when any are marked, falling back to the single selection otherwise.
- 🔹 **Tiling mini-shells with a single leader key** – `s` opens a real `$SHELL` in a movable,
  resizable box; `space` is the one leader for everything else (see below). While you type, every
  key goes to the shell — `Tab` completion included — except `Esc`.

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

### Quitting into the current directory (`Q`)

A program can't change its parent shell's working directory, so `Q` needs a one-line shell
hook, the same mechanism as ranger's `--choosedir` wrapper. Add this to your rc file
(`~/.zshrc`, `~/.bashrc`, or `~/.config/fish/config.fish`):

```bash
eval "$(minuteman init zsh)"   # or: bash; fish: minuteman init fish | source
```

That defines `mm`. Run `mm` instead of `minuteman`: `Q` quits and leaves your shell in the
directory you were browsing, while `q` (and `:q`) quit and leave it where you started. Under the
hood `mm` runs `minuteman --cwd-file <tmpfile>`; `Q` writes the directory there and the wrapper
`cd`s to it. A directory literally named `init` must be passed as `./init`.

### Default keybindings

| Key         | Action                                       |
|-------------|-----------------------------------------------|
| `j`         | move down                                     |
| `k`         | move up                                       |
| `l` / Enter | enter directory                               |
| `h`         | leave directory                               |
| `q`         | quit                                          |
| `Q`         | quit and `cd` your shell to the directory you were in (needs the `mm` wrapper above) |
| `y`         | yank (copy) selection                         |
| `m`         | cut (move) selection                          |
| `p`         | paste                                         |
| `d`         | delete — marked entries if any are marked, else the selection (confirm `y`/N) |
| `r`         | rename selection                              |
| `n`         | create — trailing `/` makes a directory       |
| `s`         | shell — drop into `$SHELL` in the current dir |
| `/`         | search — jump to the first matching entry as you type |
| `:`         | command — `:q`/`:quit` to exit, `:cd <path>` to jump to a directory |
| `v`         | toggle mark on the selection (Ranger-style: queue files for the next bulk action) |
| `space`     | leader — commands for the mini-shell panes (below); inert with no shell open |

### Mini-shell keys

`s` opens a shell in the current directory. There are two modes:

- **Typing:** every key, `Space` and `Tab` included, goes to the shell. `Esc` leaves typing.
- **Browsing:** the shell stays visible and the file browser works normally. `space` is the leader:

| After `space` | Action                                              |
|---------------|-----------------------------------------------------|
| `space`       | go back to typing in the shell                      |
| `h` `j` `k` `l` / arrows | focus the pane in that direction         |
| `\|`          | split side by side                                  |
| `-`           | split stacked                                       |
| `x`           | close the focused pane                              |
| `r` / `m`     | resize / move mode (`hjkl` to nudge, `Esc` to leave) |
| `t`           | flip the focused split between side by side and stacked |

Pressing `space` lists these in the status bar. Clicking a pane focuses it and starts typing;
dragging a divider, the box's title bar, or its right border resizes and moves things.

While a prompt is active (rename/create/delete-confirm/conflict/search/command), `Enter` submits
and `Esc` cancels; the keybindings above are not resolved until the prompt closes. `Esc` while
searching also restores the selection you had before the search started. While a paste or delete
is running in the background, every key except `Esc` (cancel — copy/move only; delete can't be
cancelled mid-flight) is ignored until it finishes, including `q`. Marks persist as you navigate
directories until toggled off or consumed by a delete, and a marked entry is shown with a `*`
prefix in the current pane.

---

## ⚙️ Configuration

Minuteman reads `~/.config/minuteman/config.toml` if present. Any key you don't specify keeps
its default. Copy [`config.example.toml`](config.example.toml) to that path as a starting point —
it documents every field with its built-in default.

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
select = ["v"]
leader = ["space"]

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
