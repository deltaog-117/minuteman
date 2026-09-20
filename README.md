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
- 🔹 **The whole look in one file** – `appearance.toml` holds colors, glyphs, per-element text
  styles (bold, italic, dim, underline, ...) and your font; `minuteman init-appearance` prints a
  commented starting point. Directories and executables are bold by default, like Ranger's.
- 🔹 **Glyph sets that match your font** – `[ui] glyphs = "nerd"` adds file-type icons and
  Powerline arrows (with a Nerd Font); `"unicode"` (default) needs nothing special; `"ascii"` is
  for a Linux console. `minuteman glyphs` previews them, and
  `minuteman init-terminal kitty|alacritty|wezterm` prints a matching font + neon color config.
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

#### The `Alt` layer

Holding `Alt` drives the shell box directly, in every mode — typing, browsing, or mid-chord — and
the shell inside never sees these keys. That means `Alt+b`, `Alt+f`, `Alt+d`, `Alt+t` (readline's
word motions) and `Alt+hjkl` (tmux-style navigation) no longer reach a shell running in the box;
use the `space` leader if you need to keep them.

| Keys | Action |
|------|--------|
| `Alt`+`h` `j` `k` `l` | move the whole box left / down / up / right |
| `Alt`+`a` `s` `d` `f`  | grow the box's left / bottom / top / right edge outward |
| `Alt`+`z` `x` `c` `v`  | focus the pane to the left / below / above / right |
| `Alt`+`Shift`+`s`      | new shell: splits the focused pane (opens the first one if none) |
| `Alt`+`t` / `Alt`+`b`  | snap the box to the top / bottom centre of the screen |
| `Alt`+`e`              | close the pane under the pointer (the focused one if it isn't over any) |
| `Alt`+`q`              | close every shell |
| `Alt`+left-drag        | move the box, grabbing it anywhere |
| `Alt`+right-drag       | resize the box from its bottom-right corner (also shrinks it) |

Some window managers grab `Alt`+drag before the terminal sees it; if the mouse gestures do
nothing, change that binding in the window manager.

While a prompt is active (rename/create/delete-confirm/conflict/search/command), `Enter` submits
and `Esc` cancels; the keybindings above are not resolved until the prompt closes. `Esc` while
searching also restores the selection you had before the search started. While a paste or delete
is running in the background, every key except `Esc` (cancel — copy/move only; delete can't be
cancelled mid-flight) is ignored until it finishes, including `q`. Marks persist as you navigate
directories until toggled off or consumed by a delete, and a marked entry is shown with a `*`
prefix in the current pane.

---

## ⚙️ Configuration

Two files in `~/.config/minuteman/`, each optional and each falling back per field — a partial
file, a missing file, even one that fails to parse never stops Minuteman from starting:

- **`config.toml`** — keybindings. Copy [`config.example.toml`](config.example.toml) as a starting
  point.
- **`appearance.toml`** — the whole look: colors, glyphs, text styles and the font. Print a fully
  commented starting point with `minuteman init-appearance > ~/.config/minuteman/appearance.toml`
  (it's [`appearance.example.toml`](appearance.example.toml)).

```toml
# config.toml
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
```

## 🎨 Appearance

`appearance.toml` has four tables. Everything left out keeps its default.

```toml
[theme]   # colors: a built-in palette ("neon", "classic", "dracula") plus per-field overrides
name = "neon"
border_focused_fg = "#00f0ff"   # the active pane; see appearance.example.toml for every field
dir_fg = "#00d9ff"

[ui]      # glyphs
glyphs = "unicode"             # "unicode" | "nerd" (icons + Powerline arrows) | "ascii"

[style]   # bold, italic, dim, underline, reverse, strikethrough — per element
dir = ["bold"]                 # directories are bold, like Ranger's
executable = ["bold"]          # files with an execute bit
doc = ["italic"]               # e.g. make documents italic
title_focused = ["bold", "underline"]

[font]    # used by `minuteman init-terminal` — see below
family = "JetBrainsMono Nerd Font Mono"
size = 12.0
```

**Colors** are a basic name (`black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`,
`white`, `gray`/`grey`) or hex (`#rrggbb`, `#rgb`). Hex is drawn in true color when the terminal
sets `COLORTERM=truecolor` and mapped to the nearest 256-color otherwise; an unrecognised color
resets to the terminal's default.

**Text styles.** Each `[style]` element takes a list of `"bold"`, `"italic"`, `"dim"`,
`"underline"`, `"reverse"` and `"strikethrough"`. A list *replaces* that element's default, so
`dir = []` turns directory bold off and `dir = ["bold", "italic"]` adds italic. The elements are
the seven entry kinds (`dir`, `source`, `config`, `doc`, `archive`, `media`, `other`),
`executable`, `selection`, `mark`, `title`, `title_focused`, `columns`, `breadcrumb`,
`breadcrumb_current`, `pill`, `mode`, `status_name`, `status`, `hint_key`, `hint_label` and
`message`; the example file says what each covers.

**A `[theme]` or `[ui]` left in `config.toml`** (where they used to live) still works, but
anything `appearance.toml` sets wins, field by field.

### Fonts and glyphs

The typeface itself is your terminal's — Minuteman can't change it, only choose which symbols it
draws and how heavy or slanted its text is. To set up a font and icons:

1. Install a Nerd Font — the **Mono** variant, so icons stay one cell wide (for example
   *JetBrainsMono Nerd Font Mono* from [nerdfonts.com](https://www.nerdfonts.com), or your
   distribution's `ttf-jetbrains-mono-nerd`-style package).
2. Put your font in `[font]` in `appearance.toml`, then run
   `minuteman init-terminal <kitty|alacritty|wezterm>` and paste the snippet into that terminal's
   config. It sets that font and a 16-color palette matching the neon theme; nothing is written
   for you. Prefer to keep your font? kitty's `symbol_map` line (in the snippet) borrows just the
   icons from *Symbols Nerd Font Mono*.
3. Run `minuteman glyphs` — if the `[nerd]` rows show boxes or `?`, the font isn't set up yet.
4. Set `glyphs = "nerd"` under `[ui]` in `appearance.toml`.

Without a Nerd Font, leave it on `unicode` (the default), or use `ascii` on a bare console. The
subcommands `init`, `init-terminal`, `init-appearance` and `glyphs` are only recognised as the
first argument; browse a directory with one of those names as `./init`.

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
