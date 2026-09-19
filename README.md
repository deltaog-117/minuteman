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
- 🔹 **HUD layout** – a header with a breadcrumb path and pills for marks, the clipboard and
  running jobs; size and age columns; a scrollbar; and a powerline-style status bar with a colored
  mode pill, the selection's permissions/size/type, your position in the list, and key hints that
  follow your own bindings and change with the mode.
- 🔹 **Neon, themeable UI** – a cyberpunk true-color palette by default: rounded frames, the
  active pane glowing cyan against dim indigo neighbours, a magenta stripe on the selected row,
  and entries colored by kind (directories, source, config, docs, archives, media). Hex colors
  drop to the nearest of 256 colors on terminals without truecolor. `[theme] name = "classic"`
  restores the old plain look, `"dracula"` swaps the palette, and every color still overrides.
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
# Built-in palettes: "neon" (default), "classic", "dracula". Any field below overrides one color.
name = "neon"
selection_bg = "#2b1a4f"
selection_fg = "keep"          # "keep" = leave each entry in its own file-type color
border_fg = "#3d4270"          # every pane but the active one
border_focused_fg = "#00f0ff"  # the active pane (or focused shell)
title_fg = "#8a8fd6"
accent_fg = "#ff2bd6"          # active title, selection stripe, marks
dir_fg = "#00d9ff"
source_fg = "#39ff88"
config_fg = "#ffd60a"
doc_fg = "#b69cff"
archive_fg = "#ff7a3d"
media_fg = "#ff5cf0"
file_fg = "#c8ccff"
status_fg = "#7a80b8"
border_type = "rounded"        # or "plain", "double", "thick"
bar_bg = "#1a1f3d"             # status-bar segments
danger_fg = "#ff3860"          # delete / overwrite prompts
separator = "flat"             # "arrow" = Powerline arrows (needs a Powerline/Nerd Font)
```

Colors are a basic name (`black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`,
`gray`/`grey`) or a hex value (`#rrggbb` or `#rgb`). Hex is drawn in true color when the terminal
sets `COLORTERM=truecolor` and mapped to the nearest 256-color otherwise. An unrecognised color
resets to the terminal's default.

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
