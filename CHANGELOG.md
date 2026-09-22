# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `theming`: a `[panels]` table in `config.toml` — `columns` (`"three"`, the default parent |
  current | preview, or `"two"` to remove the parent column and give its width to
  `current`/`preview`), `show_hud` (the header row) and `show_command_bar` (the status bar's idle
  chrome only; a prompt, a busy/leader/resize/move mode or a transient message always still shows
  it). An unrecognised `columns` value falls back to `"three"`. See `config.example.toml`.
- `tui`: a settings popup, `space` then `t` with no shell pane open (previously a no-op there).
  `j`/`k` moves the cursor, `h`/`l`/`enter` cycles the row under it — Columns, Theme
  (`neon`/`classic`/`dracula`), HUD, Command bar — and `Esc`/`q` closes it. Changes apply at once
  but are session-only: nothing is written back to `config.toml`/`appearance.toml` yet.
- `scripts/check`: format check, clippy and the whole workspace's tests, in one command.
- `tui`: a disk usage view. `u` (new `[keys] disk_usage`, `Action::DiskUsage`), or "Disk usage" in
  the right-click menu of a folder or of empty space, covers the screen with what is taking the
  space in the browsed folder, biggest first: each entry with its size, its share of the folder
  and a bar, and a totals line (`784K │ 13 entries │ on disk │ scanning 1,204`). `enter` (or `l`,
  or the arrow) opens a folder to look inside it, `h` goes back up (and above the folder you
  started in, with the cursor on the one just left), `a` switches between the size on disk and the
  apparent size, `r` scans again, `PageUp`/`PageDown`/`Home`/`End` move, the wheel and a click
  move and select, and `q`, `Esc` or `u` close the view (not the program). Rows arrive as the
  scan runs; the cursor stays on the biggest row until you move it, then on the row you chose.
  The size is the space allocated on disk by default, as `du` shows; a file with several hard
  links is counted once; the scan stays on the filesystem it started on (a folder on another one
  is listed as such and not entered) and never follows a symlink, so `/proc`, network mounts and
  link loops are safe. A folder that could not be fully read is marked `!`. Only the folder shown
  is scanned, on the blocking pool and cancelled when the view closes or moves on, and at most
  10,000,000 entries are looked at (past that the sizes say they are lower bounds); a folder's
  20,000 biggest entries get a row each and the rest are folded into one `N smaller entries` row.
  New modules `disk_usage` (the scan and the view's state, with no terminal in it) and
  `disk_usage_view` (layout, hit-testing and drawing). On a warm cache scanning `/usr` (580,370
  entries) takes about as long as `du` does (2.9 s against 2.9 to 3.0 s).
- `theming`: `disk_usage` under `[keys]`, defaulting to `u`. See `config.example.toml`.
- `tui`: `App::begin_disk_usage`, and `MenuCommand::DiskUsage` in the context menu.
- `tui`: the preview column scrolls. `J` and `K` (new `[keys] preview_down` and `preview_up`,
  `Action::PreviewDown` and `PreviewUp`) move it half a screen, and the mouse wheel over the column
  moves it three rows, as it does in the file columns. It works on text, hex dumps and archive
  listings; a scrollbar appears over the frame's right edge when the content is longer than the
  pane, and the position resets when the selection changes but survives the file changing on disk.
  The position is clamped when drawn, so scrolling past either end just stops.
- `tui` and `preview`: a hex view for binary files. Anything that is not an image, an archive or
  text shows its first 64 KiB as offset, hex bytes and an ASCII column, with as many bytes to a row
  (16, 8 or 4) as the pane is wide, and a last line saying `first 64K of 200K shown` when the file
  is longer. Only the rows on screen are formatted. New `preview::hex`.
- `preview` and `tui`: archive listings. `.zip`, `.jar`, `.tar`, `.tar.gz` and `.tgz` show a
  summary (`zip │ 15 entries │ 1.4K unpacked`), then each entry with its unpacked size, folders
  ending in `/`, and a note when entries were left out. Nothing is extracted, and every read is
  bounded: at most 5,000 entries are kept, a `.tar.gz` is inflated for at most 256 MiB (a cut-off
  listing says so rather than passing for a whole one), a plain `.tar` is skipped through by
  seeking, and a zip whose central directory claims to be over 8 MiB, or uses zip64, is shown as
  bytes instead of being opened. Entry names are cleaned of control and direction-changing
  characters before they reach the screen. New `preview::archive`; new dependencies `zip` (with no
  compression codecs, since only the directory is read), `tar` and `flate2` (already built for
  `image`).
- `preview`: `preview::load`, which decides what to show for a file (text, bytes, an archive, or
  neither) and reads it, and `looks_like_text`, which tells text from binary by content.
- `tui`: `preview_view`, which draws the three kinds of content and their scrollbar, with its row
  contents as plain functions.
- `theming`: `preview_down` and `preview_up` under `[keys]`, defaulting to `J` and `K`. See
  `config.example.toml`.
- `tui`: the status bar shows the repository the browsed directory is in. On the right, before
  the position, a segment reads `⎇ main ↑2 ↓1 +3 ~2 ?1` — the branch (or `@0123456` when detached),
  how far it is ahead of and behind its upstream, and the counts of staged, modified and untracked
  files, with conflicts as `!n`; counts of zero are left out, so a clean branch is just its name.
  On the left, after the type, the selected entry's own state (`modified`, `staged`, `untracked`,
  `staged+modified`, `conflict`) appears when it is not clean, and a folder shows the state of what
  it holds. Both segments are dropped on a narrow bar before any file detail is, and only show in
  the normal mode. The answer comes from `git status --porcelain=v2 --branch -z` run on the
  blocking pool with `--no-optional-locks` (so it never takes git's index lock and cannot make a
  `git commit` elsewhere fail), refreshed three seconds after the last run finished, cancelled when
  you leave the repository and killed after ten seconds. Whether a directory is in a repository is
  decided by looking for `.git`, so browsing elsewhere never starts a process; a missing `git`
  just means no segment. New module `git_status` (a pure parser, `Repo`, `GitStatus`).
- `tui`: the `◆ N marked` pill in the header gains the marked entries' total size, `◆ 3 marked │
  1.4 GiB`, or `at least …` when the walk hit its 500,000-entry limit. The sum runs on the blocking
  pool and starts again whenever the marks change; the previous total stays up until the new one
  lands. A marked folder counts everything below it, once, even if something inside it is marked
  too. New module `marked_size`.
- `theming`: top-level `git_status` in `config.toml` (default `true`); `false` never runs `git`.
- `tui`: glyph-set entries for the git segment (`branch`, `ahead`, `behind`, `staged`, `modified`,
  `untracked`, `conflicted`); the ASCII set uses `^`/`v` and no branch symbol. `minuteman glyphs`
  prints them.
- `tui`: a right-click context menu. Right-clicking a file or folder selects it and opens a menu
  at the pointer with Open, Open with ▸, Cut, Copy, Paste into folder (folders only), Rename,
  Delete, Mark/Unmark, Copy path and Inspect; right-clicking empty space offers New file or
  folder, Paste, Show/Hide hidden files, Refresh, Copy path and Inspect this folder. Rows follow
  the pointer, a submenu opens beside its row, the menu is pulled back inside the screen, and
  `↑`/`↓`/`j`/`k`, `→`/`l`, `←`/`h`, `Enter` and `Esc` drive it from the keyboard. Every item
  calls the same method as its key, so Cut, Copy and Delete still act on the marked entries when
  the clicked one is marked. New modules `context_menu` (contents, geometry, hit-testing; no
  terminal) and `overlay_view` (drawing).
- `tui`: an Inspect panel (`inspect`) showing name, location, type, size and on-disk size,
  permissions, owner and group, link count, and modified/accessed/created times in UTC. For a
  folder it counts files, subfolders and total size on the blocking pool, shows `counting…` until
  it lands, stops at 500,000 entries, and cancels when the panel closes. `Esc`, `Enter` or a
  click closes it.
- `tui`: Open and Open with. Open runs the desktop's default program (`xdg-open`, or `open` on
  macOS); Open with lists the `[[open_with]]` entries from `config.toml`, or `$VISUAL`/`$EDITOR`
  when there are none. A program named in `interactive_commands` takes over the terminal like a
  `:` command; any other is started detached. A missing program is reported instead of failing
  silently, and the file's path is shell-quoted whatever it is called (`open`).
- `tui`: Copy path sends the path to the terminal's clipboard with OSC 52 (`osc52`).
- `tui`: `App::begin_paste_into` pastes into a chosen folder, and `App::begin_inspect` and
  `App::open_with` back the menu.
- `theming`: `[[open_with]]` tables in `config.toml` (`name`, `command`, with `{}` standing for the
  path), exposed as `Config::open_with`. See `config.example.toml`.
- `shell_overlay`: `spawn_detached`, which starts a command under `sh -c` in its own process group
  with no terminal attached and reaps it in the background.
- `tui`: a `:` command can take over the whole terminal, so `:nvim ROADMAP.md` opens the editor.
  A command whose program name is in `interactive_commands` (editors, pagers, `htop`, `mpv`,
  `ssh`, `tmux`, `fzf` by default), or any command prefixed with `!` (`:!python3`), suspends the
  interface, runs under `sh -c` in the browsed directory on the real terminal with the keyboard
  attached, and brings the browser back repainted when it exits, with the exit status on the
  status line and the listing re-read. `Ctrl-C` interrupts the program, not Minuteman.
- `theming`: top-level `interactive_commands` in `config.toml` sets which program names get the
  terminal (an empty list turns the name check off; `!` still works).
- `shell_overlay`: `run_foreground`, which runs a command line under `sh -c` with the caller's
  stdio inherited.
- `tui`: `c` cancels everything pending in one press, from any directory: the yanked or cut
  clipboard, every mark, and a running copy, move or command. It also works while an operation is
  running. A running delete cannot be interrupted, and the status line says so. `theming` gains
  `Action::Cancel` and `[keys] cancel` (default `c`).
- `tui`: the arrow keys browse. Up and down move the selection, right opens the selected
  directory and left goes up. Key names `up`, `down`, `left` and `right` are now accepted in
  `[keys]`.
- `browser`: `search`, a breadth-first, cancellable search below a directory for the nearest name
  containing a query, using only `Vfs::list_dir`. `BrowserState::reveal` opens a path's directory
  with the cursor on it, and `BrowserState::clear_marks` forgets every mark.
- `tui`: `search_job`, which runs that search on the blocking pool, one job per keystroke.
- `tui`: `.` shows or hides dot-prefixed files and directories in every column, keeping the
  cursor on the entry it was on. The parent column keeps listing the directory you are in even
  when that directory is itself hidden.
- `theming`: top-level `show_hidden` in `config.toml` (default `false`) sets whether hidden
  entries are listed at startup, and `[keys] hidden` (default `.`) rebinds the toggle. See
  `config.example.toml`.
- `tui`: the `:` prompt runs commands without a mini-shell. `:mkdir [-p] <name>...` and
  `:touch <name>...` are built in and go through `Vfs`; `:cd` and `:q` are unchanged. Names may be
  quoted (`:mkdir 'two words'`). Any other command, and any built-in given shell syntax it can't
  honour (`*`, `|`, `&&`, `$VAR`, `~`, `mkdir -m 700`), runs under `sh -c` in the browsed
  directory on the blocking pool. Its output (up to eight lines) lands on the status line, a
  non-zero exit shows as `exit N: ...`, and `Esc` kills a command that is still running. The
  header pill counts the seconds a running command has taken.
- `tui`: the file lists refresh on their own. About twice a second the browsed directory and its
  parent are re-listed off-thread and compared with what is on screen, so a file made, removed,
  renamed or written to by a mini-shell, a `:` command or another program shows up without
  leaving the directory. The selected text or image file's preview is re-read when that file
  changes.
- `shell_overlay`: `run_command`, which runs one command line under `sh -c` with stdin closed,
  stdout and stderr merged in order, output capped at 64 KiB, and cancellation by flag.
- `shared`: `Vfs::touch`, which creates an empty file or sets an existing file's modified time to
  now. `file_ops`: `touch` and `create_directory_all` (`mkdir -p`).
- `browser`: `BrowserState::with_show_hidden`, `show_hidden`, `toggle_hidden` and
  `apply_listing`.
- `tui`: the executable is now installed as `mman`, so `cargo install --path crates/tui` puts a
  short command on the `PATH`. See *Changed* and *Removed*.
- `tui`: mouse support in the file browser. A click on a row in the middle column selects it, a
  double-click on a directory opens it (on a file it only selects), and a click on a row in the
  left column goes up a directory and selects that entry. The wheel over the left or middle
  column moves the selection three entries at a time. Clicking the browser gives the keyboard
  back from a mini-shell. The shell box keeps priority over the pointer, and nothing is handled
  while a prompt is open.
- `theming`: top-level `browser_mouse` in `config.toml` (default `true`) turns the browser's mouse
  handling off; the mini-shell box's own mouse gestures are unaffected. See `config.example.toml`.
- `tui`: `proptest` as a dev-dependency, for the property tests of the new mouse hit-testing.
- `tui`: tapping `Alt` on its own switches between typing in the mini-shell and using the file
  browser. It relies on the kitty keyboard protocol, which Minuteman enables at startup only if
  the terminal supports it and disables on exit; elsewhere the tap does nothing.
- `theming`: top-level `alt_tap` in `config.toml` (default `true`) turns that protocol off. See
  `config.example.toml`.
- `tui`: an `Alt` layer for the mini-shell box, active in every mode. `Alt+h/j/k/l` moves the
  box, `Alt+a`/`Alt+d` grow its left/top edge, `Alt+f`/`Alt+s` shrink it horizontally/vertically,
  `Alt+z/x/c/v` focuses the pane on that side, `Alt+n` splits the focused pane (or opens the first shell), `Alt+t`/`Alt+b`
  snap the box to the top/bottom centre, `Alt+m` closes the pane under the pointer and `Alt+q`
  closes every shell. `Alt`+left-drag moves the box from anywhere on it; `Alt`+right-drag
  resizes it from the bottom-right corner.
- `theming`: `appearance.toml` (in `~/.config/minuteman/`) holds the whole look in one file:
  `[theme]` colors, `[ui]` glyphs, and two new tables. `minuteman init-appearance` prints the
  fully commented default (`appearance.example.toml`).
- `theming`: `[style]` sets text attributes — `bold`, `italic`, `dim`, `underline`, `reverse`,
  `strikethrough` — for 22 elements: the seven entry kinds, executables, the selection, marks,
  pane titles, the size/age columns, the breadcrumb, header pills, the status bar's mode pill,
  name, segments, hints and messages. A list replaces the element's default.
- `theming`: `[font]` (`family`, `size`), which `minuteman init-terminal` now puts in the kitty,
  alacritty and wezterm snippets it prints. Minuteman can't apply a font itself.
- `tui`: executables (files with an execute bit) are styled on top of their kind.
- `theming`: `[ui] glyphs = "unicode" | "nerd" | "ascii"` chooses the symbols the interface draws
  with. `nerd` adds file-type icons (Font Awesome codepoints, stable across Nerd Fonts v2/v3) in
  each kind's color and Powerline arrows; `ascii` uses plain ASCII for every frame and symbol.
- `tui`: `minuteman glyphs` prints a sample of each glyph set, to see what the terminal's font
  can draw.
- `tui`: `minuteman init-terminal <kitty|alacritty|wezterm>` prints a font (JetBrainsMono Nerd
  Font Mono) and neon 16-color palette snippet matching the theme; the kitty one also offers a
  `symbol_map` fallback to Symbols Nerd Font. Nothing is written to config files.
- `tui`: header row with a breadcrumb path (`~` for home, `…` for elided middle segments) and
  pills for marked entries, the clipboard (`⧉ N yanked` / `✂ N cut`), and a gauge for a running
  copy/move/delete.
- `tui`: right-aligned size and relative-age columns in the current pane (`14K`, `2h`), dropping
  age and then size when the pane is too narrow to keep names readable.
- `tui`: scrollbar over the current pane's right border when the list overflows.
- `tui`: powerline-style status bar: a colored mode pill (`NORMAL`, `SHELL`, `LEADER`, `RESIZE`,
  `MOVE`, `BUSY`, or the prompt's label), the selection's name, permissions, size or item count,
  and type, the position (`3/49`), and key hints drawn from the user's own bindings that change
  with the mode. Text prompts show a cursor.
- `theming`: `[theme]` fields `bar_bg`, `danger_fg` and `separator` (`"flat"` or `"arrow"`, the
  latter needing a Powerline/Nerd Font), and `KeyMap::keys_for`.
- `shared`: `DirEntryInfo` gained `size`, `modified` and `mode`, filled from the `stat` the local
  listing already made per entry.
- `tui`: new neon look. Rounded frames; the active pane (the current directory's column, or the
  focused shell) gets a bright border and an accent-colored bold title while the others dim; the
  selected row gets an accent stripe (`▌`) over a background highlight and keeps its own color;
  marked rows show an accent `*`. Entries are colored by kind (directory, source, config, doc,
  archive, media) from their extension, dotfiles counting as config.
- `theming`: `#rrggbb` / `#rgb` hex colors in `[theme]`. Hex is drawn in true color when
  `COLORTERM` is `truecolor`/`24bit` and quantized to the nearest xterm-256 color otherwise.
- `theming`: new `[theme]` fields `border_focused_fg`, `accent_fg`, `source_fg`, `config_fg`,
  `doc_fg`, `archive_fg`, `media_fg` and `border_type` (`rounded`/`plain`/`double`/`thick`);
  `selection_fg = "keep"` leaves the selected entry in its file-type color. New built-in
  palettes `neon` and `classic`.
- Cargo workspace scaffold: 10 feature-first crates (`shared`, `vfs_ssh`, `browser`,
  `file_ops`, `preview`, `shell_overlay`, `trash`, `plugins`, `theming`, `tui`).
- `shared`: `Vfs` trait and a synchronous `LocalVfs` implementation over `std::fs`, with
  directories sorted before files.
- `browser`: miller-column navigation state (`BrowserState`) — enter/leave a directory, move
  selection up/down, restores the previous selection when leaving a directory (like Ranger).
- `theming`: `Config` loading from `~/.config/minuteman/config.toml` (TOML), with an
  action-to-keys keybinding schema and per-field fallback to built-in defaults when the file is
  missing, partial, or fails to parse.
- `tui`: the `minuteman` binary — a 3-pane (parent / current / selection preview) ratatui TUI
  over the local filesystem, with vim-style navigation (`j`/`k`/`h`/`l`/Enter/`q`) resolved
  through the config-driven keymap. Terminal state (raw mode, alternate screen) is always
  restored on exit via an RAII guard.
- `shared`: `Vfs` trait extended with `exists`, `create_dir`, `create_file`, `copy_file`,
  `rename`, `remove_file`, `remove_dir_all`; `LocalVfs` implements all of them over `std::fs`,
  never silently truncating an existing file or overwriting an existing copy/rename destination.
  New `VfsError::NotFound` / `VfsError::AlreadyExists` variants.
- `file_ops`: `copy`/`mv`/`delete`/`create_directory`/`create_new_file`/`rename` orchestration on
  top of `Vfs` — recursive directory copy, a guard against a destination nested inside its own
  source, and a guard against a no-op same-path operation.
- `file_ops`: `ConflictPolicy` (`Abort`/`Skip`/`Overwrite`) and `Outcome` (`Completed`/`Skipped`),
  threaded through `copy`/`mv`/`rename` so callers can resolve an existing-destination conflict
  instead of just erroring.
- `theming`: six new keybindable actions — `Yank`, `Cut`, `Paste`, `Delete`, `Rename`, `Create` —
  with defaults `y`/`m`/`p`/`d`/`r`/`n`.
- `browser`: `BrowserState::reload` — re-lists the current/parent directories after a filesystem
  mutation made outside `BrowserState` (copy/move/delete/create/rename).
- `tui`: new `app` module (`App`, `Clipboard`, `Prompt`) driving `file_ops` from the keyboard —
  yank/cut mark a clipboard entry, paste copies or moves it into the current directory, delete
  asks for `y`/N confirmation (permanent — no trash yet), rename opens a prompt pre-filled with
  the current name, and create prompts for a name (trailing `/` makes a directory). A conflicting
  paste/rename surfaces an overwrite/skip/abort prompt. A new bottom status/prompt bar in the TUI
  shows the active prompt or the last operation's result.
- `file_ops`: `copy_with_progress`/`mv_with_progress` — the same copy/move logic with a
  `&mut dyn FnMut(&Path) -> ControlFlow<()>` callback fired once per file/directory processed;
  returning `ControlFlow::Break(())` aborts with the new `FileOpsError::Cancelled`. `copy`/`mv`
  are now thin wrappers over these with a no-op callback (unchanged signatures/behavior).
- `file_ops`: `mv`/`mv_with_progress` now fall back to copy + delete when `Vfs::rename` fails
  with `ErrorKind::CrossesDevices` (moving across filesystems), instead of just erroring.
- `tui`: paste and delete now run on a `tokio::runtime::Handle::spawn_blocking` thread pool
  instead of inline, so a large copy/move/delete no longer freezes the render loop. The event
  loop switched from a blocking `event::read()` to a 100ms `event::poll` so it keeps redrawing
  progress even without a keypress. Copy/move show live "N done (name)" progress and are
  cancellable with `Esc`; delete shows an indeterminate "deleting…" status (no per-file hook to
  report through). While an operation is in flight, all other actions — including quit — are
  blocked except `Esc`, so the process can't exit mid-write.
- `shell_overlay`: new crate — `spawn_shell(cwd: &Path) -> io::Result<ExitStatus>` spawns
  `$SHELL` (falling back to `/bin/sh`) inheriting stdio directly, blocking until it exits.
- `theming`: new `Shell` keybindable action, default `s`.
- `tui`: `s` suspends raw mode/the alternate screen, drops into a real interactive shell in the
  browser's current directory, then resumes and forces a full redraw via `Terminal::resize`
  (not `Terminal::clear`, which depends on a cursor-position query that can hang — see
  `DIARY.md`). `TerminalGuard` gained `suspend`/`resume` methods for this.
- `theming`: `Theme` grew from 2 fields to 7 — `selection_bg`/`fg`, `border_fg`, `title_fg`,
  `dir_fg`, `file_fg`, `status_fg`. New `RawTheme` (`name: Option<String>` plus every color field
  optional) resolves via `Theme::named` (built-in `"default"`/`"dracula"` palettes, unrecognized
  names fall back to `"default"`) as a base, with individually specified fields overriding it.
- `tui`: every pane's border and title, each directory/file entry, and the status bar are now
  themed (previously only the selection highlight was). New `themed_block`/`entry_item` helpers
  in `main.rs`.
- `preview`: `is_image(path) -> bool` (extension whitelist: png/jpg/jpeg/gif/bmp/ico/tiff/tif/
  webp) and `load_image(path) -> Option<DynamicImage>` (returns `None` rather than erroring on any
  I/O or decode failure) — pure, terminal-agnostic building blocks for image preview.
- `tui`: new `image_preview` module (`ImagePreview`, `PreviewStatus`) — renders the selected
  image inline via `ratatui-image` (Kitty/iTerm2/Sixel, falling back to Unicode halfblocks).
  Decoding and resize/encoding both run on `tokio::runtime::Handle::spawn_blocking` via
  `ratatui-image`'s `ThreadProtocol`, so a large or slow-to-encode image never blocks the render
  loop. The preview pane shows "loading preview…" while decoding and "preview failed" if the
  file doesn't actually decode as an image despite its extension.
- `browser`: `BrowserState::select_index` (clamped jump to an arbitrary entry), `find_match`
  (case-insensitive substring search over the current directory's entries, always from the top),
  and `goto` (jump directly to an arbitrary directory, resolving a relative path against the
  current one) — backing the new search/command bar.
- `theming`: two new keybindable actions — `Search` (default `/`) and `Command` (default `:`).
- `tui`: `Prompt` gained `SearchInput`/`CommandInput` variants — `/` opens an incremental
  filename search that jumps the selection to the first case-insensitive match as you type and
  restores the original selection on `Esc`; `:` opens a command prompt supporting `:q`/`:quit`
  (exit the app) and `:cd <path>` (jump to a directory). `handle_prompt_key` now returns
  `ControlFlow<()>` so a `:q`/`:quit` command can signal the app to exit.
- `preview`: `is_text(path) -> bool` (extension whitelist covering common source/config/doc
  formats, plus a filename whitelist for extensionless files like `Makefile`/`.gitignore`) and
  `load_text(path) -> Option<String>` (returns `None` for a file over 1 MiB, containing a null
  byte, or not valid UTF-8) — pure, terminal-agnostic building blocks for text/code preview,
  mirroring `is_image`/`load_image`'s shape.
- `tui`: new `text_preview` module (`TextPreview`, `PreviewStatus`) — reads the selected text/code
  file off `tokio::runtime::Handle::spawn_blocking` and renders it, word-wrapped, in the preview
  pane, the same never-block-the-render-loop treatment `image_preview` gives image decoding.
  Shows "loading preview…" while reading and "preview failed" for anything over the size cap or
  not valid UTF-8 despite a text-like name.
- `tui`: `ImagePreview` and `TextPreview` are now constructed and driven together via a new
  `Previews` struct, keeping `run`/`draw`'s argument counts down now that there are two preview
  pipelines instead of one.
- `shell_overlay`: new `PopupShell` type — spawns `$SHELL` on its own `portable-pty` pty and
  parses its output into a `vt100::Parser` screen buffer on a background thread. New
  `write_input`/`resize`/`with_screen`/`try_wait` methods; `try_wait` returns a local
  `ExitOutcome` rather than a `portable_pty` type, keeping that dependency contained to this
  crate.
- `tui`: new `popup_shell` module — sizes a centered popup (80%×70% of the frame), encodes
  `KeyEvent`s into the raw escape sequences a real terminal sends (arrows, function keys,
  `Ctrl`/`Alt` combos), and renders the `vt100` screen buffer cell-by-cell as styled `Span`s
  (color/bold/italic/underline/inverse), including cursor positioning.
- `tui`: `s` now opens the popup shell instead of suspending to a full-screen shell — every
  keystroke forwards straight to the popup's pty while it's open, `Event::Resize` resizes the
  pty to match, and the popup closes itself (with a "shell exited" status) the moment the child
  exits. The old full-screen `spawn_shell` path is unchanged in `shell_overlay`, just no longer
  bound to any keybinding.

- `shell_overlay`: `PopupShell::close` — kills and reaps the child shell immediately, for a
  caller-initiated close rather than waiting for it to exit on its own.
- `tui`: `Esc` now closes the popup shell (kills the child, restores the underlying UI, sets a
  "shell closed" status) instead of being forwarded to it as input.
- `theming`: two new keybindable actions — `Select` (default `v`, Ranger-style toggle-mark) and
  `Leader` (default `space`, an inert placeholder reserved for future chorded commands).
- `browser`: `BrowserState` gained a mark set (`toggle_mark`/`is_marked`/`marked_paths`/
  `prune_marks`) — marks persist across directory navigation until toggled off or consumed by a
  bulk action.
- `tui`: `v` toggles a mark on the current entry (shown with a `* ` prefix in the current pane);
  `d` (delete) now acts on every marked entry when any are marked, falling back to the single
  cursor selection otherwise, with a pluralized confirmation prompt and per-item progress.
  `space` is bound to the new inert `Leader` action and never blocks or delays any other key.
- Added `config.example.toml` at the repo root — every `[keys]`/`[theme]` field spelled out with
  its built-in default, to copy to `~/.config/minuteman/config.toml` as a starting point. A new
  `theming` test parses it and asserts it resolves to exactly `RawConfig::default()`, so it can't
  silently drift out of sync with a future default change.

- `theming`: two new keybindable actions — `ShellFocus` (default `tab`) and `ShellMove` (default
  `g`).
- `tui`: while the popup shell is open, `tab` toggles whether keystrokes go to the shell or drive
  the browser underneath it, so the shell can stay open and visible while you keep browsing.
  `g` (only reachable while the popup is open and unfocused) enters move mode — `h`/`j`/`k`/`l`
  or arrow keys reposition the popup, `enter`/`esc` confirms. `popup_shell::popup_area` now takes
  an `(i32, i32)` offset, clamped so the popup can never be nudged off-screen.

- `tui`: new `shell_layout` module — a binary split-pane tree (`ShellPanes`) replacing the single
  floating popup shell with tmux-style tiled panes. The tree/geometry logic (`split_rect`,
  `Divider`, tree search/mutation) is generic over the leaf payload so it's unit-testable without
  spawning real shells; `ShellPanes` specializes it to `shell_overlay::PopupShell` and owns
  spawning, resizing, rendering, and reaping exited panes.
- `theming`: `Action::ShellMove` replaced with three new actions — `ShellSplitHorizontal`
  (default `%`), `ShellSplitVertical` (default `"`), and `ShellPaneNext` (default `o`), matching
  tmux's own default split bindings for muscle memory.
- `tui`: `s` opens the first shell pane tiled into the frame (docked below the status bar, not
  floating); `%`/`"` split the focused pane side by side or stacked, spawning a new shell in the
  current directory and focusing it; `o` cycles keyboard focus to the next pane. `Esc` now closes
  just the focused pane, promoting its sibling to fill the freed space, and only exits shell mode
  entirely once it's the last pane open.
- `tui`: mouse support — `EnableMouseCapture` is now on for the whole session. Dragging a
  divider (the shared border between two sibling panes) live-resizes their split ratio; clicking
  inside a pane focuses it (and refocuses keyboard input to the shell if it was browsing).
  Dragging a pane to reposition it and reordering panes are explicitly out of scope (see
  `ROADMAP.md` and `DIARY.md`) — panes are tiled, not floating, so there's no independent
  position to drag.
- `tui`: the focused pane's border is now highlighted with the theme's `selection_bg` color, so
  which pane keystrokes route to is visible at a glance once more than one is open.
- `tui`: `shell_area` now returns a centered 80%-width/70%-height box within the browser region
  instead of the whole thing, so shell panes stay contained to a mini floating area (matching the
  original single popup's sizing) instead of tiling across the entire screen. `draw` now calls
  `shell_area` directly rather than duplicating its layout math.
- `tui`: the shell box itself can now be dragged around the screen by its title bar, like a
  floating window — `shell_area` takes a `(dx, dy)` offset from centered, clamped so the box can
  never be dragged off screen. A mouse-down on the box's top border (where the "shell" title
  renders) starts the drag; `Drag` events accumulate into the offset, `Up` ends it. The offset
  resets to `(0, 0)` on every new `s` spawn. This moves the whole box, not individual panes inside
  it, which stay tiled exactly as before (see `ROADMAP.md`/`DIARY.md` for why per-pane dragging is
  still out of scope). New `ShellView` struct bundles the pane tree with its offset for `draw`,
  keeping its argument count from growing.
- `tui`: batch file operations — `y`/`m` now yank/cut every currently marked path (falling back
  to the cursor entry if nothing's marked), and `p` pastes the whole batch one item at a time,
  re-spawning the next item itself as each one finishes so a multi-item paste stays one
  continuous background operation. A conflict on any item still pauses the batch on the existing
  overwrite/skip/abort prompt and resumes afterward — including `Skip`, which now actually
  continues a batch past a conflicting item instead of ending the whole paste. The status line
  shows an `[i/N]` progress hint for batches of more than one item.
- `tui`: keyboard resize/move for shell panes. `space` (`Action::Leader`) now starts a chord
  while any shell pane is open — `r` enters a resize mode where `hjkl`/arrows nudge the focused
  pane's nearest divider by 5% per press, falling back to growing/shrinking the box's own
  width/height by 2 cells per press when there's no divider along that axis (a lone pane, or a
  tree only ever split the other way); `m` enters a move mode where the same keys nudge the
  whole box's offset by 2 cells per press. `Esc` leaves either mode; any other key leaves it too
  but is still dispatched normally afterward. New `shell_layout::ShellPanes::resize_focused`
  (now returning whether it actually adjusted a divider) and `shell_layout::NudgeDir`; `shell_area`
  gained a `size_adjust: (i32, i32)` parameter alongside its existing `offset`.
- `tui`: the shell box's own right border is now mouse-draggable, resizing its width directly
  (checked before the title-bar drag hit-test, so the shared top-right corner prefers resize).
  `space t` — a third, one-shot branch of the leader chord — flips the focused pane's split
  between side-by-side and stacked, keeping the same two panes and their ratio. New
  `shell_layout::ShellPanes::toggle_focused_orientation`.
- `theming`: new `QuitToCwd` keybindable action (`quit_to_cwd`), default `Q`. Single-character
  key names in `[keys]` are now case-sensitive (`"Q"` is Shift+q, distinct from `"q"`); named keys
  such as `"Enter"`/`"space"` still match case-insensitively.
- `tui`: `Q` quits and records the directory being browsed into the file named by the new
  `--cwd-file <path>` option; `q` and `:q` never write it. Without `--cwd-file`, `Q` is a plain
  quit.
- `tui`: new `minuteman init <bash|zsh|fish>` subcommand prints an `mm` shell wrapper that runs
  minuteman with `--cwd-file` and `cd`s the parent shell afterward, since a child process cannot
  change its parent's directory. Unknown flags and extra positional arguments are now rejected
  with exit code 2 instead of being taken as the start directory.

### Changed
- `tui`: a file with no known extension that turns out to be text (a `.service` file, a `notes`
  file) now shows its text, and a binary one shows a hex dump; both used to show only the file
  name. A file named like text but holding binary bytes (a `.txt` that is not) shows a hex dump
  instead of `preview failed`. A text-named file over 1 MiB still says `preview failed`, and a
  socket, device or named pipe still shows only its name (a named pipe is never opened, since that
  would wait for a writer forever).
- `tui`: the wheel over the preview column, which did nothing, now scrolls it.
- `tui`: ratatui is built with its `unstable-rendered-line-info` feature, for
  `Paragraph::line_count`, which the scrolling text preview needs to know how many rows wrapped
  text takes.
- `tui`: double-clicking a file in the middle column opens it with the desktop's default program;
  it used to do nothing. Double-clicking a directory still opens the directory.
- `tui`: `/` search now looks below the current directory as well as in it. The nearest match
  wins, so a name in the directory you are in is still found first; otherwise the search walks
  subdirectories one level at a time on the blocking pool (hidden entries only when shown, at
  most 16 levels and 200,000 entries), opens the directory of the first hit with the cursor on
  it, and says `searching…` or `no match` in front of the query. `Esc`, or deleting the query,
  returns to the directory and row the search started from. `Enter` keeps the cursor where it
  landed, and stops a walk still in progress.
- `theming`: the default `move_down`, `move_up`, `enter` and `leave` bindings now include the
  arrow keys. Naming a key in `config.toml` still replaces that action's whole list, so a config
  that sets `move_down = ["j"]` drops the arrow unless it is listed.
- `tui`: `parse` in `command` takes the list of interactive program names, and
  `Prompt::SearchInput`'s `origin` is now a `SearchOrigin` (directory and index).
- `tui`: `run` takes its read-only inputs as one `Session` struct.
- `tui`: `Esc` typed into a mini-shell now goes to the program running in it instead of leaving
  typing mode. Full-screen programs such as `vim` and `fzf` use `Esc` themselves, and the old
  behaviour dropped keyboard focus from under them. To leave typing, tap `Alt`, click the
  browser, or press `Alt+m` to close the pane. The status bar hint for shell mode now reads
  `alt browse`. `Esc` still cancels prompts, running commands, and the resize/move chord.
- `tui`: hidden files are now hidden by default. Before, every file was listed. Set
  `show_hidden = true` in `config.toml` to keep the old behaviour.
- `tui`: a command typed at `:` that is not a built-in used to answer `unknown command`; it now
  runs in `sh`. `sh -c` is not an interactive shell, so aliases and shell functions are not
  available, and a `cd` inside the command does not move the browser (use `:cd`).
- `browser`: `BrowserState::reload` now keeps the cursor on the same entry by path, falling back
  to the same index only when that entry is gone. Before, a file appearing above the cursor
  moved the selection.
- `tui`: text and image previews drop the result of a read or decode that a newer one has
  superseded, not only one for a path that is no longer selected.
- `tui`: the executable Cargo builds is `mman` instead of `minuteman`, and `mman init <shell>`
  prints a wrapper function named `mman` (it was `mm`) that calls the binary through
  `command mman`. The project, the crate layout and `~/.config/minuteman/` keep their names.
  The README and `config.example.toml` say `mman` wherever they tell you to run a command.
- `tui`: the middle column's scroll position is now kept between frames instead of being
  recomputed from the top each frame, so the list only scrolls when the selection reaches an
  edge of the view. Needed to map a click back to the entry it hit.
- `tui`: `Alt`-prefixed letters are no longer forwarded to a shell in the mini-shell box:
  `Alt+b/f/d/t` (readline word motions) and `Alt+h/j/k/l` (tmux navigation) now drive the box
  instead. `Alt+q` no longer quits from browse mode. The `space` leader is unchanged.
- `tui`: directories and executables are now bold by default (like Ranger). Every bold in the
  interface was hardcoded before and is now a `[style]` default.
- `theming`: a `[theme]` or `[ui]` table in `config.toml` is still honored, but values in
  `appearance.toml` override it field by field. `config.example.toml` no longer lists them.
- `theming`: `[theme] separator` now defaults to `"auto"` (Powerline arrows exactly when the
  glyph set is `nerd`) instead of `"flat"`; `"flat"` and `"arrow"` still force a style. With the
  default `unicode` glyph set nothing looks different.
- `tui`: the header prefix, breadcrumb separator, selection stripe, scrollbar, gauge, status-bar
  dividers, prompt cursor and pane frames are drawn from the active glyph set instead of being
  hardcoded.
- `tui`: the current pane's frame is titled with the directory's own name; the full path lives
  in the new header. Rows in that pane now start with a two-cell gutter (selection stripe, mark).
- `tui`: `Esc`/`space`/resize/typing states are shown by the status bar's mode pill and hints
  instead of one-off messages ("resize mode — ...", "shell focused — ..."). Status messages now
  clear after five seconds instead of staying until replaced.
- `theming`: the default theme is now `neon` (true color). The previous 16-color look is
  `name = "classic"`; `name = "default"` and unknown names resolve to `neon`.
- `tui`: the mini-shell frame now uses the shared pane frame, so a focused shell is outlined in
  `border_focused_fg` (previously `selection_bg`).
- `tui`: `space` is now the single leader key for all mini-shell commands: `space space` goes
  back to typing in the shell, `space h`/`j`/`k`/`l` (or arrows) moves focus between panes,
  `space |` and `space -` split side by side / stacked, `space x` closes the focused pane, next to
  the existing `space r`/`m`/`t`. Pressing `space` lists them in the status bar.
- `tui`: while typing in a shell, `Esc` now leaves typing mode instead of closing the pane (close
  with `space x`), and every other key — `Space`, `Tab`, `o`, `%`, `"` included — is sent to the
  shell.
- `tui`: `Clipboard.path: PathBuf` is now `Clipboard.paths: Vec<PathBuf>`.

### Fixed
- `tui`: cancelling a multi-item paste no longer lets the batch carry on if the cancel lands just
  as an item finishes. Each item has its own cancel flag, so the request was forgotten and the
  next item started.
- `tui`: with Caps Lock on, `Q` (quit and `cd`) was read as `q`, and letters typed into a
  mini-shell came out lowercase. In a terminal running the keyboard protocol, Caps Lock is
  reported as a flag beside an unchanged lowercase letter. It now flips the letter's case, as in
  a terminal without the protocol: Caps Lock+`q` is `Q`, Caps Lock+Shift+`q` is `q`. `Alt`
  commands ignore Caps Lock.
- `tui`: `Q` (quit and `cd`) did nothing different from `q` in a terminal that reports Shift+q as
  a lowercase `q` with the Shift flag, which the keyboard protocol allows. Shift plus a lowercase
  letter is now read as the capital before any key is looked up. This also makes capitals typed
  into a mini-shell and the `Alt+Shift` check behave under the same reports.

### Removed
- `tui`: the `minuteman` executable and the `mm` shell wrapper, replaced by `mman` (see
  *Changed*). Anything that ran `minuteman ...` or `mm` needs `mman`.
- `theming`: `Action::ShellFocus`/`ShellSplitHorizontal`/`ShellSplitVertical`/`ShellPaneNext` and
  their `shell_focus`/`shell_split_horizontal`/`shell_split_vertical`/`shell_pane_next` config keys
  (`Tab`, `%`, `"`, `o`), replaced by the `space` leader commands above. Old config files that
  still set them keep loading; the keys are ignored.
- `tui`: `TerminalGuard::suspend`/`resume`, now dead code after the popup shell replaced their
  only caller.
- `tui`: `popup_shell::popup_area` and `theming::Action::ShellMove`/`RawKeyMap::shell_move` —
  floating-popup positioning is gone now that shell panes are tiled by `shell_layout::ShellPanes`
  instead.

## [0.1.0] - 2026-09-16

### Added
- Initial release: bare-bones, runnable local filesystem browser establishing the workspace
  architecture as a north star for the rest of `ROADMAP.md`.
