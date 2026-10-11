# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `plugin_api`: an `image` document for `set_preview`, so a previewing plugin can say a file is a
  picture without sending pixels: its source is `original` (the file itself, read by the interface
  through the browsed paths) or `handoff` (a scratch PNG the plugin made, which must be a file
  named `minuteman-preview-image-*` directly in the temporary folder, and which the interface
  decodes and deletes). A handoff naming any other file is refused.
- `mman-preview` takes `--image-hook <json>` beside `--text-hook`: it runs a `[[preview_hook]]` of
  kind `image` and hands over the picture the hook wrote. A hook that fails, times out, writes
  nothing, writes more than 64 MiB or leaves a link falls back to the file's name.
- `plugin_api`: `set_view`, a notification with which a plugin fills a named slot of the interface
  with styled lines (text, a tone, a glyph by name) and the interface draws it. The first two
  slots are `status_entry` and `status_right` on the status bar. Every view is cleaned on arrival:
  control and direction-changing characters are removed and its size is limited. `client::Incoming::Reply`
  now carries the reply's `result`.
- `plugins/git` (`mman-git`), a third bundled plugin: the status bar's git branch, distance from
  upstream, counts, and the state of the selected entry, as before but asked from a plugin. It follows
  `dir_changed` and `selection_changed`, and runs `git status` on a thread of its own.

- `plugin_api`: a third slot, `header_pill`, one line drawn as a pill on the header's right edge.
- `plugins/hud` (`mman-hud`), a fourth bundled plugin: the header's uptime, load average and
  memory, as before but from a plugin that fills `header_pill` every two seconds.

- `plugin_api`: a fourth slot, `entry_badge`: a mark beside each row of the browsed folder. Each
  line of its view names an entry with the new optional `Line::key`, and is bounded one badge at a
  time (2,000 lines, one span of at most 2 characters, a key of at most 255) rather than by the
  per-view totals; a line with no key is refused.
- `mman-git` now also marks changed entries in the list: `+` staged, `~` modified, `?` untracked,
  `!` conflicted (in the user's glyph set), and a folder shows the merged state of what it holds.

- `plugin_api`: what a previewing plugin and the host exchange. The `preview` event tells the
  plugin which file the preview column is about (or `null` for none, or again when that file
  changed on disk); the `read_file` request reads up to 4 MiB of a file from an offset through the
  host's own paths (a folder on another machine or inside an archive previews like the local
  disk; a path with a zero length is only described, and a folder, pipe or device is never
  opened); the `set_preview` notification answers with a finished document: colored text (pieces
  of seven token kinds a line), a binary file's first 64 KiB, or an archive listing. A plugin
  lists `provides = ["preview"]` to be the one. Every document is cleaned on arrival: control and
  direction-changing characters are removed from text and replaced in names, and it is limited to
  1 MiB of text, 64 KiB of bytes and 5,000 archive entries.
- `plugins/preview` (`mman-preview`), a fifth bundled plugin: the preview column's text, hex dump
  and archive listing, as before but made by a plugin. It reads the file through the host with
  `read_file`, colors source with `syntect`, lists zip, tar and tar.gz with every read bounded as
  before, and runs `[[preview_hook]]` tables of kind `text` (passed to it as `--text-hook`). Each
  file is loaded on a thread of its own and dropped if the cursor moves on, so a slow disk or host
  never holds up the next file. Install it with `cargo install --path plugins/preview`.
- `shared::names`: `is_image`, `is_text` and `clean_name`, the file-name rules the interface and
  the preview plugin both use.
- A startup notice, "the preview column is a plugin and mman-preview was not found", when the
  plugin is not installed and the user has not configured one of their own.

- `plugin_api`: what an inspecting plugin and the host exchange. The `inspect` event names the
  entry the Inspect panel is about (and is sent again with `null` when it closes); the `stat`
  request describes one entry through the host's own paths (kind, size, link target, allocated
  bytes, mode, owner and group ids, link count, times, and a `local_path` when the backend is the
  local disk); the `tally` request starts counting what is below a folder and returns at once, the
  count arriving as a `tally_done` event, with `tally_cancel` to stop it; the `set_inspect`
  notification answers with the panel's rows. A plugin lists `provides = ["inspect"]` to be the
  one. Rows are cleaned on arrival: control and direction-changing characters are removed, text
  is cut to 32 characters a label and 1,024 a value, and more than 32 rows are refused.
- `plugins/inspect` (`mman-inspect`), a sixth bundled plugin: the Inspect panel's rows, as before
  but made by a plugin. It describes the entry with `stat`, counts a folder with `tally`, and sends
  its rows at once and again when the count lands; opening another entry or closing the panel
  stops the count. Install it with `cargo install --path plugins/inspect`.
- `shared::format` (`format_size`, `mode_string`, `group_thousands`, `format_time`) and
  `shared::tally` (`tally_dir`), which the interface and the Inspect plugin both use.

- `plugin_api`: what an archiving plugin and the host exchange. The `archive` event hands the
  plugin work under a request number (a compress of one or more jobs, each a list of sources and an
  archive to write, with a format, a level, what to do with a name that is taken and an optional
  password; or an extract of one or more archives into folders); `archive_cancel` stops a request;
  the plugin answers with `archive_progress` (the entry reached and a running total, thinned to
  about 25 a second) and one `archive_done` that says how the request ended (completed, cancelled,
  failed in words, a password needed, a wrong password) and which jobs wrote something. A plugin
  lists `provides = ["archive"]` to be the one. A failure message is cleaned on arrival like every
  text a plugin sends, and a result naming more than 100,000 jobs is refused. `Format`, `Level`,
  `Conflict` and the typed `Password` (wiped on drop, never shown by `Debug`, at most 256
  characters) moved here so the interface and the plugin name the same types.
- `plugins/archive` (`mman-archive`), a seventh bundled plugin: compressing and extracting zip, tar
  and tar.gz, as before but done by a plugin on this machine's disk. Each request runs on a thread
  of its own, so a long one never holds up a cancel, and a request in flight when the host goes
  away is stopped and its result written. An extract with no password reports `password_needed`
  before it writes anything, and with one checks every encrypted archive of the batch first.
  Install it with `cargo install --path plugins/archive`.
- `plugin_api`: what a plugin that fills the deck's rail of devices and the host exchange. The
  `devices` event carries a command (`watch` on or off, `open`, `toggle` or `eject` a device by its
  node); the plugin answers with `set_devices` (every device with its read and write rate, a short
  history and whether an action is under way) and `devices_note` (a message, or a folder to go
  to). Every text is cleaned on arrival, a device with no node or a repeated node is dropped, and
  the list is cut to 256 devices. A plugin lists `provides = ["devices"]` to be the one.
- `plugins/devices` (`mman-devices`), an eighth bundled plugin: the devices behind the deck's rail,
  as before but from a plugin. It looks at the machine once a second while the rail shows, and
  runs mount, unmount and eject through `udisksctl` one at a time. Install it with
  `cargo install --path plugins/devices`.
- `shared::devices::plan`, the steps to open, mount, unmount or eject a device, used by the plugin
  and the wiring diagram alike.
- `shared::create_dir_all`, the `mkdir -p` over a `Vfs` that `file_ops` and the archive plugin both
  use.
- `plugin_api::serve`, the first request that runs from the interface to a plugin: a plugin that
  `provides` `vfs-archive` is sent `vfs` requests (`metadata`, `list`, `scan`, `read_link`, `open`,
  `read`, `close`) for every path inside an archive and answers each with one reply carrying the
  same `id`. A file is read through a handle the plugin keeps (at most 32 open, 256 KiB a read)
  until `close`. `plugin_host::ServeLink` carries the call from a synchronous `Vfs` to the plugin
  and back; it outlives the plugin's process and counts each start as an epoch, so a handle from a
  process that died can never reach the next one, and a request ends by its reply, its process
  ending or its timeout (120 s), whichever comes first.
- `plugin_api::remote`, `RemoteVfs`: a `Vfs` over the `read_file` request, with a window that
  doubles while reads continue, which the preview and archive plugins both use to read a file
  through the host. It moved out of `plugins/preview`.
- `mman-archive` now also serves the inside of archives (zip, tar, tar.gz): the browsing,
  previews, search, disk usage and copying out that already went through the `Vfs` work as before.

### Changed
- The deck's rail of devices no longer scans the machine itself: `mman-devices` sends the list
  and does the mounting. The rail, its keys, the selection, the slide-in of a new device and the
  check of whether carried files fit are unchanged. Without the plugin the rail says
  "mman-devices was not found" and the status line says how to get it; a plugin that stops is
  started again, at most once in three seconds.
- The `devices` crate is now the `shared::devices` module, so a plugin can use it. Nothing about
  what it reads or how changed.
- Which files are pictures, and running `[[preview_hook]]` tables of kind `image`, moved out of
  `tui` into `plugins/preview` (`tui::preview_hook` is removed). The interface still decodes,
  fits and draws the picture. Without `mman-preview` a picture shows only its file name, as text
  files already did; the grid, tile and filmstrip views still draw their thumbnails themselves.
- The preview column's text, hex dump and archive listing moved out of `tui` and `preview` into
  `plugins/preview`. Scrolling (`J`/`K`, the wheel), wrapping, the scrollbar, the colors and the
  typewriter reveal stay in `tui`, which draws the document the plugin sends, so moving through a
  file never waits on the plugin. `preview` now only decodes images; the hex layout (`hex_dump`)
  stays in `tui` because it depends on the pane's width. Without `mman-preview` the column shows
  only the file's name. The trash browser's and the finder's previews go through the plugin too,
  since the interface names the file, not the browser's cursor. A plugin that dies with a
  document half made ends the "loading preview…" instead of leaving it up, and starts again for
  the next file.
- A `[[preview_hook]]` of kind `text` now runs in the preview plugin, and where an extension is
  named by several hooks the first one still decides it, whatever its kind. Hooks of kind `image`
  stay with the interface.
- `scripts/vfs-gate` allows `plugins/preview/src/hook.rs`, whose hook is handed a file on this
  machine's disk.
- The system figures moved out of `tui` (`system_hud.rs`) into `plugins/hud`. `system_hud = true`
  in `config.toml` still turns them on, now by starting the plugin; without `mman-hud` the header
  has no such pill.
- The git status moved out of `tui` (`git_status.rs`) into `plugins/git`. Without `mman-git` the
  status bar has no git segment; `git_status = false` in `config.toml` still turns it off, now by
  not starting the plugin. A plugin that stops no longer leaves its text on screen.
- The fuzzy matcher (`Query`) moved from `browser` to `shared`, and the finder's walk now runs in
  the plugin host, through the same paths the browser reads. `tui`'s finder popup and view are
  now a general picker's, titled by the plugin. A plugin that dies while its picker is open
  closes it.
- While a picker is open, or has just been asked for, the event loop looks every 16 ms instead of
  every 100 ms, so what the plugin sends appears as it arrives.
- `plugins` crate renamed `plugin_host`, and its plain wire data (the event names, `EventKind`,
  `JobOutcome`, `Change`, `Job`, `StateSnapshot`, `LogLevel`, `WireEntry`) moved into a new
  `plugin_api` crate that depends only on `shared`. The host re-exports them under the paths they
  had, so plugins and the protocol are unchanged. The moved types now derive both `Serialize` and
  `Deserialize`.
- `scripts/vfs-gate` also scans the new `plugins/` folder.
- `tui`: the deck view's disk tray is now a rail at the left of the columns, the way a file
  manager's places panel is, instead of a strip of framed cards above them. Devices are grouped
  under Internal, Removable and Network headings; each is two lines, its name over a gauge of how
  full it is with the free space (or "not mounted" and its size, or, while files are carried,
  whether they fit). A bar at the edge of the row marks the folder you are in, a busy disk shows a
  small live sparkline beside its name, and a device plugged in grows in line by line. The rail
  is 26 cells wide and is left out, giving the columns the whole window, when the body is under 70
  cells wide or 6 rows high. The columns, and the gaps between panes, are laid out in what the rail
  leaves.
- `tui`: in the deck view `Tab` takes the keys to the rail of devices and gives them back. With
  them there, the up and down arrows choose a device and `Enter`, `m`, `e` and `p` act as before;
  `Esc` hands the keys back. `h`, `j`, `k` and `l` stay the file browser's in every view, rail
  focused or not, so `l` still enters a folder and `h` still goes up.
- `tui`, `theming`: `y` (yank), `d` (cut), `x` (trash) and `X` (delete for good) now work on the
  entry under the cursor when nothing is marked, and on the marks when there are any. With marks,
  `y` and `d` act at once: mark, go to the folder the files should end up in, press the key, and
  they are copied or moved there with no `p` in between (marks spent, undo still takes the batch
  back). A cut whose marks are all in the folder already says so and keeps them. Without marks,
  `y` and `d` put the cursor entry on the clipboard for `p`. Compress and extract still need marks.
  The right-click menu, the deck's and the wiring diagram's `p` and the `:` commands keep the
  clipboard route, and `:trash` follows the same marks-else-cursor rule.
- `tui`: `space` is a leader with no shell pane open too: `space x` opens the trash browser and
  `space t` the settings popup (which the code had but never reached). With a pane open `space x`
  still closes it.

- `tui`: the wiring diagram now fits all of itself in the window, hardware and unfolded folders
  together, instead of panning across a canvas that a deep path made far bigger than the window.
  It shows fewer siblings of a crowded folder when the window is short (12, 8, 5, then 3, the rest
  being the "+N more" note; the folder on the way to the one you are in and the one chosen are
  always kept, then their nearest siblings, where it used to be the first twelve), squeezes the
  boxes to as little as 10 cells wide to fit the columns (and widens them up to one and a half
  times in a wide window), and becomes the indented tree if the columns still cannot fit. The tree's
  name column now grows to hold the deepest folder name. `+`, `-` and `0` refit as before.
- `tui`: the wiring diagram now sizes itself to the window. It has four zoom levels, from the full
  boxes down to one line of text a node, and by default picks the largest at which the whole
  diagram fits the width and the height, so a filesystem with many unfolded folders no longer runs
  off the bottom. `+`/`-` step the zoom by hand and `0` goes back to fitting. A narrow window can
  now keep the columns at a smaller level before it falls back to the tree.
- `browser`, `tui`: the fuzzy finder (`f`) no longer scores anything on the render thread, so it does
  not stutter on a large tree. An engine on the blocking pool owns the index and the ranking and
  sends back the best 200 results; typing sends it a query and the interface carries on, showing
  the previous results until the new ones arrive (the header says `ranking…` meanwhile). The engine
  works in slices of about 2 ms and notices a new letter between them. Typing another letter now
  narrows from the previous letter's matches instead of rescoring everything, and deleting a letter
  returns to the earlier result at once. A folder's preview is read in the background when it is
  selected, up to 300 entries, instead of on every frame. The selection stays on its entry when
  results reorder under it. The results are the same as before: nothing is skipped or approximated.
  `browser::finder::Ranking` now takes its work in slices (`advance`) and `Query::extends` was added.
- `theming`: `gradient_borders` in `config.toml` is superseded by `[shape] gradient`; `false` still
  turns the gradient off unless `appearance.toml` or the popup sets `gradient`. The
  `Config::gradient_borders` field is removed; read `config.theme.shape.gradient` instead.
- `tui`: the appearance popup (`a`) is laid out like a settings window: five pages down the left
  (Theme, Colors, Shape, Text, Font), switched with `Tab`/`Shift+Tab`, `1`–`5` or a click, and the
  chosen page's rows on the right. The Theme page lists the built-in palettes and the saved themes
  together, with "Save current look" and "Reset to defaults"; choosing a palette now replaces any
  colors changed by hand (it used to keep them) and keeps the shape. `Esc` closes the popup from
  any page. The hint line no longer says changes are session-only, as they have been saved since
  `local.toml` existed.
- `tui`: marking with `v` is now the only way to choose entries. Yank, cut, trash, delete,
  compress and extract — from the keys, the right-click menu and the `:` commands — act on the
  marks and nothing else, where the menu and the commands used to fall back to the row under the
  cursor; and each spends the marks it takes: after `y` or `d` nothing stays marked, and the same
  goes for a confirmed trash or delete and for a compress or extract that starts (declining a
  question keeps them). Messages now read "nothing marked — mark with v first". The status line
  after a cut says "cut X to move" instead of "marked X to move".
- `tui`, `theming`: the startup splash is now off by default (`boot_splash = false`), so the
  browser is there the moment it starts instead of after a 900 ms splash. With it switched on, the
  browser is already drawn underneath, and a key press that dismisses the splash is now also handled
  as usual instead of being swallowed (a mouse click still only dismisses it).
- `browser`: `BrowserState::find_match` uses the same fuzzy ranking as the `/` search.
- `tui`: modified, accessed and created times are shown in the machine's time zone instead of UTC
  (Inspect writes the offset, e.g. `+02:00`; the details column title is now `Modified`). `tui`
  depends on `chrono` directly, already built for the trash crate.
- `tui`: the filmstrip's strip now shows the same bordered tiles as the grid instead of small
  pictures floating in a tall empty frame, and thumbnails are scaled up to fill their tile instead
  of staying at their native size. A frame with no title (a tile) no longer gets two blank cells in
  its top border.
- `shared`, `file_ops`, `preview`: archives are now made, extracted, checked for a password and
  listed through the `Vfs` trait instead of the disk, so they work on any backend. `Vfs` gained
  `create_write` (a seekable writer that only creates new files and never follows a link),
  `create_symlink`, `set_mode` (a backend with no permissions answers `Unsupported`, which is not an
  error for extraction) and `replace` (moves a finished archive over its name in one step). The
  extraction safeguards are unchanged: nothing is written through a link, and folders already
  checked are remembered so a large archive does not recheck them per file. There is no change on
  the local disk. `file_ops::archive::needs_password`, `check_password` and
  `preview::archive::list` and `list_within` now take the filesystem as their first argument.
  `scripts/vfs-gate` no longer allows the two archive files to touch the disk directly; a line
  that only asks an archive entry whether it is a folder is marked `// vfs-gate: archive entry`.
- `shared`, `preview`, `tui`: every feature that reads a browsed path now goes through the `Vfs`
  trait, which gained `metadata`, `symlink_metadata`, `read_link`, `open_read`, `scan_dir` and
  `local_path` and is `Send + Sync`. Preview, Inspect, disk usage, the marked-size pill and the
  background jobs take the filesystem the app was built with, and Inspect leaves out a field a
  backend does not report. There is no change on the local disk. New: an in-memory backend
  (`shared::MemVfs`), a conformance suite for backends, and `scripts/vfs-gate`, which `scripts/check`
  runs to keep code from reaching for the disk directly.
- `theming`, `tui`: default keys are now `d` cut (was `m`), `x` delete to the trash (was `d`) and
  `X` delete permanently (was `D`). `s` no longer opens a shell by default (`Alt+n` still does;
  set `shell = ["s"]` under `[keys]` to restore it). `y`, `d`, `x` and `X` act only on entries
  marked with `v` and report "nothing marked" otherwise, instead of using the entry under the
  cursor; the right-click menu is unchanged.
- `tui`: a program path that needs shell quoting (a space, a parenthesis) is now found when
  opening a file with it; it was reported as "command not found". The status line names the
  program by its file name rather than its full path.
- `tui`: a completed cut-and-paste now drops the marks on the files it moved, and only empties the
  clipboard when it still holds the cut that was just pasted.
- The Inspect panel moved out of `tui` (`inspect.rs` is removed) into `plugins/inspect`. The
  interface keeps the panel's frame, colors and keys and draws the rows the plugin sends. Without
  `mman-inspect`, "Inspect" says how to get it and opens nothing. An entry that cannot be described
  shows an `Error` row in the panel instead of a message on the status line. The panel shows
  "loading…" until the first rows arrive.
- `scripts/vfs-gate` allows `plugins/inspect/src/rows.rs` (this machine's `passwd` and `group`,
  read only for a path the host says is on the local disk) in place of `crates/tui/src/inspect.rs`.

- Compressing and extracting moved out of `file_ops` (`file_ops::archive` is removed, and with it
  the `zip`, `tar`, `flate2` and `zeroize` dependencies) into `plugins/archive`; the code and its
  tests moved unchanged apart from the error type, which is the plugin's own. The interface keeps
  the compress and extract forms, the password and overwrite prompts, the progress gauge, `Esc` and
  `c` to cancel, and the trash: it sends the plugin the work, shows what it reports, and sends the
  originals (or the archive) of the jobs the plugin finished to the trash. The extract password
  question is now the plugin's answer instead of a read of the archive on the render thread, so an
  encrypted archive no longer makes the interface do that I/O. Without `mman-archive`, `:compress`,
  `:extract` and both forms say how to get it and start nothing. `FileOpsError` lost its archive
  variants and gained `Plugin`, the words of a failure a plugin reported.
- **Behavior change, until the plugin can be given files through the host:** compressing or
  extracting where a path is on another machine (`ssh://`) or inside an archive is refused by name
  with a reason. It used to work, because the archive code ran on the interface's own `Vfs`.
- The trash browser, restore, empty and undo stay in the core by decision: the trash is one of the
  file operations, and an undone copy goes to it.
- Browsing inside an archive (`a.zip!/dir/f`) is now answered by `mman-archive` instead of code in
  the interface: `vfs_archive` keeps only the `Vfs` that routes those paths to the plugin, and the
  archive reading (`index`, the codecs and the engine) moved into the plugin. `address.rs` became
  `shared::archive_path`. Archives stay read-only. Without the plugin, opening an archive says how
  to get `mman-archive` and the rest of the file manager is untouched.

### Fixed
- `tui`: the permanent-delete and trash confirmations now take a capital `Y` as well as `y`.
- `tui`: in the wiring diagram a box that was only partly inside the window was left out while the
  lines to it were still drawn, so some boxes showed and others did not. It is now drawn cut off by
  the window's edge.
- `shell_overlay`: the test that runs a fake shell from a script it has just written failed about
  one run in forty with "text file busy", when another test thread forked while the script was open
  for writing. It now tries again on that error alone; the checks on the directory and the exit
  code are unchanged.
- `browser`: going up or into a folder that cannot be listed (a parent with no read permission, or
  the step above a remote machine's root) left the browser pointing at it with the old listing on
  screen. It now stays where it was and reports the error. `:cd` also reports why a path cannot be
  reached, such as a refused connection, instead of "not a directory".
- `tui`: in a folder on another machine, the actions that would run on this one are refused with a
  reason instead of acting on the wrong files: shell panes, `:` shell commands, opening a file
  with a local program, and sending to the trash (a remote machine has none; the permanent delete
  works). `Q` no longer writes an `ssh://` folder for the shell to `cd` into.
- `preview`: the archive round-trip proptest could generate two entries with the same name,
  which a zip archive rejects outright and which made a tar's read-back order for the duplicate
  an untested implementation detail rather than something the test meant to pin down; the
  generated file list is now deduped (first occurrence wins) before building the expected
  listing. The regression case proptest found is checked in under
  `crates/preview/proptest-regressions/`.

### Added
- Structured search (`L`), a second bundled plugin: the program `mman-search`
  (`cargo install --path plugins/search`), started by itself when it is next to `mman` or on
  `PATH`. It lists every entry below the browsed folder that satisfies every word typed: a piece of
  the path, a glob (`*.rs`), `re:` and a regular expression, `size:>1M`, `size:1k..2M`,
  `mtime:<7d`, `mtime:>1w`, `type:f` or `type:d`. A word it cannot read is left out and named in
  the status bar. `/` and `f` are unchanged. New action `locate` and key `locate = ["L"]`; a
  `[[plugin]]` of your own with `provides = ["locate"]` takes its place.
- Plugin protocol: `walk` takes `metadata`, and each entry then carries `size` (files only) and
  `modified` (seconds since the epoch) when the listing has them. Both are optional on the wire.
- The fuzzy finder (`f`) is now a plugin that ships with Minuteman: the program `mman-finder`
  (`cargo install --path plugins/finder`), started by itself when it is next to `mman` or on
  `PATH`. It ranks and the interface draws, so how it looks and behaves is as before; without it
  `f` says how to get it. A `[[plugin]]` of your own with `provides = ["find"]` takes its place.
- Plugin protocol: a plugin can provide one of the interface's actions (`provides` in its
  `[[plugin]]`, the `action` event), open a list the interface draws (`show_picker`,
  `picker_items`, `picker_close`, and the `picker_query` and `picker_closed` events) and have the
  tree below a folder listed through the host's own paths (`walk`, `walk_cancel`, and the
  `walk_batch` and `walk_done` events). `plugin_api` has the shapes and a `client` module with the
  line helpers a Rust plugin needs.
- A `plugins/` folder for the plugins bundled with Minuteman, and a `dependency_direction` test
  that fails if a crate under `crates/` depends on one of them, if `plugin_api` depends on
  anything but `shared`, or if a bundled plugin depends on anything but `plugin_api` and `shared`.
- `portal_backend` (new crate, binary `mman-portal`), `tui`, `shared`: `mman` can be the file
  chooser of a browser's open and save dialogs. `mman-portal` is a backend for the XDG desktop
  portal's `FileChooser` (`OpenFile`, `SaveFile`, `SaveFiles`): when the portal forwards a dialog
  to it, it opens `mman` in a terminal window of class `mman-chooser`, waits for the pick, and
  replies with the chosen `file://` URIs; closing the window or `q` is a cancel, and the portal
  withdrawing the call closes the window. `mman` itself gained `--chooser-file <path>` with
  `--pick <file|files|directory|save>` and `--pick-name <name>`: `Enter` or a double-click on a
  file picks it (with marks, the marks, when picking several), `Q` confirms where the cursor is
  (the folder under it or the one you are in for a folder pick, the marked files for several, a
  name prompt for a save), and `Enter` on a file while saving offers its name. A save asks before
  replacing a file that is there, and refuses a name with a slash. A path on another machine or
  inside an archive is refused, since the program waiting could not open it. The answer file holds
  the paths each ended by a NUL byte (an empty file means no pick), described in
  `shared::chooser`. The terminal is the first of foot, alacritty, kitty, wezterm and xterm found,
  or `MINUTEMAN_PORTAL_TERMINAL` (one of those) or `MINUTEMAN_PORTAL_TERMINAL_CMD` (any command
  that runs its arguments). `crates/portal_backend/data/` holds the `.portal` file, the D-Bus
  service file, a systemd user unit and an example `portals.conf`. The dialog's file-type filters
  and extra choices are not used yet.
- `tui`: an atlas in the `V` cycle (`view = "atlas"` in `config.toml`), after the wiring diagram: a
  zoomable map of the machine's storage that takes the whole window. It starts as one block per
  volume, sized by capacity and filled from the bottom by how full the volume is (red at 90% and
  over, dim when it is not mounted), with a `◆` on the volume holding the folder you are in. `Enter`
  (or a second click) zooms into the chosen block: a volume shows its biggest folders as blocks
  sized by what is inside them, a folder shows its contents, and a file's block opens its folder in
  the browser. `Backspace` or `Esc` zooms back out one level. The arrows (or `h`/`j`/`k`/`l`) move
  to the nearest block that way, `Tab` and `Shift+Tab` walk through them biggest first, `o` opens
  the chosen block's folder in the browser, `a` switches between size on disk and apparent size, and
  `r` looks again. Entries too small to deserve a block of their own are folded into one `+N
  smaller` block. Linux only, like the deck. New `atlas_layout` (a pure treemap on whole terminal
  cells), `atlas` (state and keys) and `atlas_view` (drawing) modules; the folder levels reuse the
  disk usage view's scanner, so a scan runs on the blocking pool and is cancelled when you zoom
  away.
- `devices`, `tui`: a wiring diagram in the `V` cycle (`view = "wiring"` in `config.toml`), after the
  deck. It takes the whole window and shows how the storage is wired: the bus a disk hangs off
  (`PCIe 02:00.0`, `USB 2-3`, `SATA ata1`), the disk, its partitions, anything stacked on them
  (LVM, an encrypted container, RAID) and the filesystem with its mount, a gauge of how full it is
  and, while files are carried, whether they fit. Boxes are drawn in columns joined by lines, with a
  mark moving along the line into any disk that is busy and the bright frame on the node holding the
  folder you are in; in a window too narrow for the columns it is an indented tree. The arrows (or
  `h`/`j`/`k`/`l`) move through it, `Enter` opens a filesystem (mounting it first), `m` mounts or
  unmounts, `e` ejects the whole removable disk with every volume on it unmounted first, and `p`
  sends the carried files. The diagram goes on past the mount point: a filesystem, and then any
  folder, unfolds into its subfolders as more boxes to the right, read in the background (folders
  only; hidden ones only while the browser shows them; a folder with many shows the first twelve and
  "+N more", always including the one on the way to where you are). `Right`/`l` unfolds and then
  goes into the first folder, `Left`/`h` folds and then goes back, a click chooses a node and
  unfolds or folds it, `Enter` opens a folder in the browser, and `p` on a folder sends the carried
  files into it if its filesystem has room. Opening the view unfolds the way down to the folder
  being browsed and chooses it, and the diagram scrolls sideways to keep the chosen box and what is
  under it on screen. New:
  `devices::topology` (`Sys::topology`, `Topology`, `bus_of`) and `mountinfo::parse_opts`, which can
  keep `/boot`.
- `tui`: the deck view (`V`) now says whether what you are carrying will fit. With files yanked or
  cut (else marked), each card shows `✓ fits · 22G left`, `~ tight`, `✗ needs 4.0G more`,
  `✗ a file is over 4.0G` (a FAT32 stick, however empty), `✓ same disk · instant` (a cut within one
  disk) or `mount to check`, and a card that cannot take the files is dimmed. `p` in the deck sends
  them to the chosen device's root through the ordinary paste, yanking the marks first if need be,
  and refuses a device they will not fit on with the reason in the status line.
  `marked_size::measure` also reports the biggest single file.
- `devices`, `tui`: a deck view in the `V` cycle (`view = "deck"` in `config.toml`): the usual
  columns under a strip of cards, one per disk, USB stick or network share, each with its name,
  where it is mounted, a gauge of how full it is, the free space and a sparkline of how busy it is.
  The card holding the folder you are in has the bright frame, and a device plugged in slides in.
  `Tab` takes the keys to the cards: `Left`/`Right` (or `h`/`l`) choose one, `Enter` opens it
  (mounting it first if it is not mounted), `m` mounts or unmounts it, `e` ejects it and `Esc`
  gives the keys back; clicking chooses a card and a second click opens it. Mounting, unmounting
  and ejecting use `udisksctl`; without it the cards are shown and cannot change anything. A device
  that is plugged in but not mounted is listed too. Linux only. New crate `devices`, and
  `BrowserLayout::deck`.
- `browser`, `theming`, `tui`: a fuzzy finder on `f` (`find` under `[keys]`). A panel opens over
  the browser listing every entry below the current directory, ranked as you type, with a preview
  of the highlighted one beside it on a screen 100 columns or wider. Letters in order match
  (`rdme` finds `README.md`) and so does part of a path (`src/main` finds `src/main.rs`); a match on
  the name itself ranks above one spread over the folders. `Down`/`Ctrl-N` and `Up`/`Ctrl-P` move
  through the results, `Ctrl-D`/`Ctrl-U` scroll the preview, a click selects a row and the wheel
  moves, `Enter` closes the panel with the browser's cursor on the entry, and `Esc` or a click
  outside closes it. Every other key goes to the query, so `j` and `k` are letters there. The tree
  is indexed in the background, up to 16 levels and 200,000 entries, and the results fill in
  while it runs; hidden entries are left out unless the browser is showing them. It reads only
  through `Vfs::list_dir`, so it works on a remote machine and inside an archive. New:
  `browser::finder` (`Candidate`, `Ranking`, `walk`, `score`) and `Action::Find`.
- `theming`, `tui`: a configurable border gradient. `gradient_from` and `gradient_to` under
  `[theme]` take a color or `accent`/`focused` (the defaults, which follow `accent_fg` and
  `border_focused_fg`); `[shape] gradient` sets which frames get it (`off`, `focused`, `panes`,
  `all`) and `gradient_flow` which way it runs (`around`, `mirror`, `across`, `down`). The
  appearance popup edits all four (the ends on the Colors page, the rest on the Shape page) and a
  saved theme keeps them. `Theme::resolve` turns the two keywords into colors.
- `theming`, `tui`: a shape for the look. `[shape]` in `appearance.toml` sets `gaps_in` (blank cells
  between panes, 0–4), `gaps_out` (to the left and right screen edges, 0–8, half as many rows at
  the top and bottom), `title_align` (`left`, `center`, `right`), `shadow` (menus, popups and the
  shell box cast a shadow) and `dim_inactive` (the parent and preview columns are drawn dim).
  Out-of-range numbers are clamped and unknown names fall back, and a small terminal gets smaller
  outer gaps. `border_type` accepts `dashed`, `thick-dashed`, `block`, `block-inside` and `none`.
  The shape is part of the theme (`Theme::shape`, `RawTheme::shape`), so a saved theme stores it and
  brings it back; a saved theme from before has the default shape. A `[shape]` alone does not stop
  the colors following the terminal. The appearance popup's Shape page edits all of it live.
- `plugins`, `theming`, `tui`: a plugin can be bound to several keys (`on_key = ["b", "B"]`; a single
  string still works), can declare `:` commands (`commands = ["blame"]`, tried after the built-ins
  so a plugin adds commands but never replaces `cd` or `q`, and only for a line that uses no shell
  syntax), and can be fired by name with `:plugin <name> [args]`. The `key` event now carries
  which `key` fired it, and a new `command` event carries the `command` word and its `args`; both
  fields are additive, so `API_VERSION` stays 1. `PluginManager::spawn` takes a `Triggers` value
  (keys and commands) instead of one optional key, and `PluginSpec.on_key` became `on_keys`.
  `:plugin` with an unknown name says so and lists the running plugins.
- `plugins`, `tui`: a plugin can register a `:` command while it runs, with a `register_command`
  request (`{"name": "blame"}`), and give it up with `unregister_command`, so `:blame` reaches it
  without a `commands` line in config. A name is letters, digits, `-`, `_`, `.` or `+`; a built-in
  command and a word another plugin owns are refused, with the reason in the reply. A registered
  word stays after the process that registered it exits, so it still starts the plugin again;
  `:plugins restart` forgets it until the new process registers again. Config words and registered
  words share one registry (new `plugins::registry`), checked at once by the plugin's reader
  without waiting on the render thread. Additive, so `API_VERSION` stays 1. New
  `PluginManager::reserve_commands`; `tui::command::BUILTIN_WORDS` lists the reserved words and a
  test keeps it in step with `parse`. `dir_stats.py` registers `:state`.
- `plugins`: benchmarks for what a plugin costs, in `crates/plugins/tests/plugin_cost.rs`, ignored
  by default and run with `cargo test --release -p plugins --test plugin_cost -- --ignored
  --nocapture --test-threads=1`. They measure the time to start a plugin, the round trip of
  `list_actions`, `get_state` and `run_action`, and the per-tick cost with no plugin configured. The
  test fixture takes a new `--bench=METHOD:N` flag for them.
- `plugins`, `tui`, `theming`: a plugin can run one of Minuteman's own actions with a `run_action`
  request (`{"name": "paste"}`) and list them with `list_actions`. A name is the word that binds the
  action under `[keys]` (`paste`, `sort`, `undo`, `hidden`, ...); the action does what its key does,
  confirmations included. It runs only while the user is plainly browsing: a popup or view, a prompt,
  a chord, a focused shell or a running job refuses it, and the reply says which. The chord starters
  (`bookmark_jump`, `bookmark_set`, `leader`) are refused and not listed. Additive, so `API_VERSION`
  stays 1. New `theming::Action::{ALL, name, from_name, waits_for_key}`, `plugins::ActionRequest` and
  `PluginManager::declare_actions`; action names are now part of the plugin API. `dir_stats.py` gains
  `:plugin dir-stats run ACTION` and `:plugin dir-stats actions`.
- `plugins`, `tui`: a plugin whose process has ended is started again the next time something is
  meant for it (a key, a `:` command, `:plugin <name>`, or an event for a plugin with no keys or
  commands), where a key bound to a plugin that had exited used to do nothing, silently. Status 0
  counts as finishing and starts again quietly; a plugin that crashes three times in a row, each
  within a minute of starting, is given up on, and each key press or command then says so and how
  to start it again. `:plugins` lists every plugin's state and how often it was restarted, and
  `:plugins restart <name>` starts a fresh process (stopping a live one first), clearing a stopped
  plugin's crashes; any other `plugins ...` line still goes to the shell. A new process gets
  `init` again. New `PluginManager::summaries` and `restart`, `PluginInfo`, `PluginState` and
  `Restart`; `dispatch_key`, `dispatch_command`, `dispatch_named`, `publish_state` and
  `notify_job_finished` now take `&mut self`. A plugin's writer now always sends its queued
  lines, `init` first, before a mailbox event.
- `plugins`, `theming`, `tui`: a plugin can subscribe to events with `events = ["dir_changed",
  "selection_changed", "job_finished"]` in its `[[plugin]]` table (none by default, so existing
  plugins see nothing new). `dir_changed` and `selection_changed` carry `from` and `to`; a burst of
  changes waits in a per-plugin mailbox that holds one of each and reaches the plugin as a single
  event from where the burst began to where it ended, so a slow or stuck plugin never builds a
  queue and costs two slots of memory. `selection_changed` also fires when only the marks change.
  `job_finished` carries the job's `kind` and `outcome` and fires once per job, not per item of a
  batch paste, and not for one paused on a conflict prompt. Additive, so `API_VERSION` stays 1; an
  unknown event name is reported on stderr and skipped. New `EventKind` and `JobOutcome`,
  `PluginManager::notify_job_finished`, `Triggers::events` and `PluginSpec::events`;
  `publish_state` now also posts the events, and the published marks are sorted. `tokio`'s
  `macros` feature is now enabled for `plugins`. `dir_stats.py` reacts to `dir_changed` and
  `job_finished`.
- `plugins`, `tui`: a plugin can send a `get_state` request at any time and get back the browser's
  `cwd`, `cursor`, `marks`, `show_hidden`, `sort`, `sort_reversed` and `view`. The interface
  publishes the state once per tick (only while a plugin is running) and a plugin's reader answers
  from the latest value, so a request never waits on the render thread and can be up to one tick
  old; before the first tick it still names the start directory. Additive, so `API_VERSION` stays
  1. New `StateSnapshot` and `PluginManager::publish_state`/`is_empty`; `App::poll_plugins` now
  takes the browser and the view's name. `dir_stats.py` gained `:plugin dir-stats state`.
- `examples/plugins/dir_stats.py`: a dependency-free Python reference plugin (key, `:` command and
  `:plugin` use, `read_dir` and `create_file` requests, replies matched by `id`), run end to end by
  the `plugins` integration tests with a real `python3` (skipped, with a note, when absent).
- `tui`, `trash`: the trash browser previews the selected item in a pane beside the list (on a
  terminal 100 columns wide or more): text, an image, an archive's listing, a hex dump or a
  folder's contents, read from where the trash keeps the item, so nothing is restored to be looked
  at. `J` and `K` (the browser's preview-scroll keys) scroll it. `TrashEntry` gained `stored_path`.
  The browser's preview drawing moved into `draw_preview_pane`, which both screens now use.
- `tui`, `trash`: a trash browser, opened with `T` (rebindable as `trash_view`). It lists everything
  in the desktop trash, newest first, with its original folder, when it was deleted and its size.
  `v` marks, `r` puts the marked entries back where they were (one whose place is taken stays in
  the trash and says so), `x` deletes the marked ones for good and `E` empties the trash; both ask
  first. It reads and acts on the blocking pool, so the interface never waits. New in `trash`:
  `entries`, `restore_entries`, `purge_entries`, `TrashEntry` and `EntrySize`.
- `tui`, `browser`: `/` is now a fuzzy search. A name matches when it holds the typed letters in
  order (`aernd` finds `aerend`), and matches are ranked: letters side by side, at the start of a
  name or a word, and in the same case score higher, and a name containing the whole text beats any
  that does not. The nearest folder with any match still wins, now by its best match instead of its
  first. While the prompt is open `Down`/`Ctrl-N` and `Up`/`Ctrl-P` step through the best 20
  matches and the status line shows the position (`2/14`). New `browser::search::rank_entries` and
  `KEPT`; `Outcome::Found` carries a ranked list; `Query::score` replaces the substring test.
- `tui`, `browser`: the folder can be ordered. `,` cycles the order (name, size largest first,
  modified newest first, permissions) and `<` reverses it, both rebindable as `sort` and
  `sort_reverse`; in the details view the ordering column's title ends in an arrow and clicking a
  title orders by it (again to flip). Folders stay first and the cursor stays on its entry. The
  order lasts for the session. New `browser::sort` (`Sort`, `SortKey`) and `BrowserState::sort`
  and `set_sort`.
- `tui`: `PageUp`, `PageDown`, `Home` and `End` jump a screenful or to the first or last entry in
  the list, details, grid and filmstrip views.
- `tui`: with the Nerd glyph set the grid and the filmstrip's strip draw a file-type icon on the
  tiles of folders and of files with no picture.
- `tui`: the rename, create, `/` search, `:` command and Open with prompts now edit a line with a
  cursor: Left/Right, Home/End, Delete, `Ctrl`-A/E/B/F/D/W/U/K, `Alt`-B/F and word jumps with
  `Ctrl`/`Alt`+arrows. A `/` search runs again only when its text changes.
- `tui`, `browser`: folders open in the background. Entering, leaving, `:cd`, a bookmark jump, a
  breadcrumb click, a click in either column and the menu's Open no longer freeze the interface
  while a slow folder (another machine, a large archive) loads: the old folder stays on screen, the
  BUSY pill shows, `Esc` abandons the move, and a local folder still opens before the key press
  returns. A folder that cannot be read now leaves the browser where it was and says why; a failed
  step up used to exit the program. `BrowserState` gained `plan_enter`, `plan_leave`, `plan_goto`
  and `apply_move`, with `MovePlan` and `LoadedMove`.
- `tui`, `theming`: the view `V` switches to is remembered between sessions (saved in
  `local.toml`), and `view` in `config.toml` sets the one to start in. `Config` gained `view` and
  `save_view`; `RawLocal` gained `view`.
- `tui`, `theming`: with no `[theme]` set, the default look is now built from the terminal's own
  colors (foreground, background and the 16 ANSI slots, read over OSC 10/11/4), so it matches a
  wallpaper-driven setup on any window manager, desktop or OS. Text colors are lifted to stay
  readable, and panel backgrounds stay transparent. A terminal that answers only the background
  still gets Catppuccin Mocha or Latte; one that answers nothing still gets the neon look. Unix only
  for now. New `theming::TerminalPalette` and `Theme::from_palette`; `Theme::auto` takes the
  palette as its first argument; `tui` depends on `libc` directly.
- `tui`, `theming`: `default` is now a named palette, first in the appearance popup's Theme cycle,
  and what a `[theme]` with fields but no `name` is built on. It is the adaptive look: the
  terminal's own colors, else Catppuccin Mocha or Latte from its background, else neon. The theme
  is resolved again after the terminal has been asked, so this holds however it was configured.
  `Theme::set_terminal_palette` became `Theme::set_terminal_probe` and also records the background.
- `tui`, `theming`: `terminal` is now a named palette, last in the appearance popup's Theme cycle
  and valid as `name = "terminal"` in `appearance.toml`. It is the same terminal-derived look as the
  default, but stays selected across restarts and alongside other `[theme]` fields, which turn the
  automatic default off. The terminal is now asked for its colors at every start rather than only
  when no theme is set. `Config` gained `merged_theme`; `Theme::set_terminal_palette` records the
  result; where the terminal reports nothing, `terminal` is the neon palette.
- `tui`: two more views on `V`. The grid shows framed tiles, a thumbnail over the name each, over
  the whole window like a graphical file manager's icon view: up/down move a row, the left/right
  arrows a tile, the wheel a row, a click selects and a double-click opens, and marks and the
  right-click menu work on tiles. The details view is one flat full-width table of name, size,
  exact modified time (UTC) and permissions with a title row, dropping the right-hand columns first
  on a narrow terminal. `V` now cycles columns, grid, details and filmstrip. New modules
  `tile_view` and `grid_view`; no new dependency, and the release binary grew by about 17 KB.
- `theming`, `tui`: `V` (new `[keys] cycle_view`) switches the browser's view, and the first extra
  view is the filmstrip: the selected file's preview scaled up as large as the window allows, with
  a strip of thumbnails of its neighbours underneath. `j`/`k` and up/down move along the strip,
  a click on a thumbnail selects it, the wheel over the strip moves three entries and over the big
  preview scrolls it; `V` returns to the name list. Thumbnails are decoded and encoded for their
  cell size on the blocking pool into a bounded cache, so the interface never waits on a picture;
  folders and non-images show their type instead. New modules `view_mode`, `thumbnails` and
  `filmstrip_view`; no new dependency, and the release binary grew by about 84 KB.
- `vfs_archive`, `shared`, `browser`, `tui`: archives can be browsed as folders. `enter` on a
  `.zip`, `.jar`, `.tar`, `.tar.gz` or `.tgz` (also `l`, a double-click, or "Open" in the
  right-click menu) opens it; its root is the archive's path plus `!` (`/x/a.zip!`) and `h` steps
  back out onto the archive's row. A new `ArchiveVfs` wraps the application's `Vfs` and serves
  those paths, so previews, search, marks, disk usage and copying files or folders out work unchanged,
  and an archive on a remote machine opens too. Archives are read-only: changing one, running a `:`
  command or a shell in one, trashing from one and opening a file in one with a program are refused
  with a message. The table of contents is cached per archive and reread when the file changes.
  Reading is bounded (200,000 entries, 512 MiB inflated while indexing, 256 MiB per opened entry,
  no zip64 or oversized zip directory), over a limit is an error rather than a short listing, names
  are cleaned of control characters, `..` entries are dropped, and links are followed only inside
  the archive. An `[[open_rule]]` that covers an archive still wins over opening it as a folder.
  `shared::mount` holds the path spelling (`mount_root`, `archive_of_root`), and
  `BrowserState::leave` uses it to put the cursor back on the archive.
- `vfs_ssh`, `tui`, `browser`, `shared`: native remote browsing over SSH, with no mounted
  filesystem. `:cd ssh://[user@]host[:port]/path` opens a folder on another machine; with no path
  (`:cd ssh://host`) it opens the login's home folder, and `sftp://` is accepted as the same
  thing. The header shows the machine as the first segment, and clicking a segment stays on it.
  Listing, previews, marks, mkdir, touch, rename and delete work there, and copying or moving
  between this machine's disk and a remote one (or two remote ones) streams the file through the
  application, so yank and paste work across them. The connection is the system `ssh`'s own
  (`ssh -s sftp`), so `~/.ssh/config` aliases, keys, the agent and `ProxyJump` behave as in a
  terminal. A host not already in `known_hosts` is refused, with the reason shown, so connect once
  with `ssh host` to trust it; there is no password prompt, only keys and the agent. A host or user
  name that could be read as an `ssh` option is refused before `ssh` is run. A dropped connection
  is re-made on the next action, and a read is retried once at once. `Vfs` gained
  `resolve_typed`, the rule that turns a typed location into the folder to open. A remote folder
  is re-read every five seconds instead of every half second.
- `shared`: the conformance suite's `Seed` can declare a store's time resolution and how a seeded
  link's text reads back, which a backend with whole-second times or its own path prefix needs.
  The remote backend is held to the same sixteen scenarios as the disk and memory.
- `tui`, `file_ops`, `theming`: the rest of the archive work. `C` and `E` open the compress and
  extract forms for the marked entries. The compress form has a "Save in" folder, and both forms
  warn as you type when the name or folder they would create is already taken. `:compress` and
  `:extract` ask overwrite / skip / abort instead of failing on an existing destination, and
  `:compress -p name.zip` seals a zip with a password asked for in a masked prompt. A zip entry
  records its file's modified time (UTC), and extracting a zip whose end-of-central-directory
  record claims too many entries or too large a directory is refused before it is read into
  memory.
- `file_ops`, `tui`: encrypted zip archives. The compress form has Password and Confirm password
  rows (zip only) that seal the archive with AES-256; the legacy ZipCrypto scheme is never written.
  Extracting an encrypted zip, through `:extract` or the extract form, asks for the password in a
  masked prompt before writing anything and asks again after a wrong one. File names stay visible
  in an encrypted zip (the form says so); only contents are sealed. The password is wiped from
  memory when dropped and is never shown or logged.
- `file_ops`, `trash`, `tui`, `theming`: undo history. `z` takes back the newest create, rename,
  move, copy or trash and `Z` does it again (`undo` and `redo` under `[keys]`); a batch such as a
  paste of several marked files, or a `:mkdir a b c`, is one step, and the last 100 are kept for
  the session. A trash is undone by restoring the item from the desktop trash, a move by moving
  it back, a copy by sending the copy to the trash, and a creation by removing it only while it is
  still empty. A step whose files have changed since (something in the way, an emptied trash) is
  refused with a message and stays on the stack. Permanent deletes, overwrites, compress/extract
  and shell commands are not recorded.
- `tui`, `theming`: opening files by association. `enter`, `l`, `→` and a double-click on a file
  open it with the first matching `[[open_rule]]` (new in `config.toml`: `match` takes extensions
  or MIME patterns), else the program the desktop has for its type, else `xdg-open`. The
  right-click "Open with" submenu now lists matching rules, `[[open_with]]` entries and every
  installed program for the file's type (from `.desktop` files, `mimeinfo.cache` and
  `mimeapps.list`), and ends with "Other…", which asks for a command. `Terminal=true` programs
  take over the terminal. New modules `mime_type`, `desktop_entry` and `associations`; new
  `terminal` field on `[[open_with]]`; new `open::program_word`.
- `tui`: mouse stage 3, drag and drop. Dragging a row (or the marked set, when the pressed row is
  marked) onto a folder in the middle or left column moves it there, `Ctrl` held at the drop copies,
  and blank space in the left column means the folder above. Dropping on a mini-shell pane types
  the quoted paths into it. A label follows the pointer, the target folder is drawn inverted,
  holding at the middle column's top or bottom border scrolls it, and `Esc` cancels. A drop uses the
  same background paste as `p`, leaves the yank/cut clipboard alone, and refuses to move a folder
  into itself. New module `browser_drag`, `App::begin_drop`, and
  `overlay_view::render_drag_ghost`.
- `tui`, `browser`, `theming`: mouse stage 2. `Ctrl`-click on a row toggles its mark; a range
  click (`Shift` or `Alt`, chosen by the new `mouse_range_modifier` option, default `"either"`)
  marks every entry from the last one clicked to this one, keeping earlier marks; clicking a
  segment of the header's path jumps to that directory; a middle-click opens an entry (a folder in
  place, a file with its default program). A modified click never pairs into a double-click.
  `BrowserState` gained `toggle_mark_at`, `mark_range_to` and `anchor_here`; `hud` gained `crumbs`
  and `crumb_rects`, and `breadcrumb` keeps its signature.
- `tui`: an extract form on the right-click menu. "Extract…" appears on a zip, tar or tar.gz and
  opens a form for the destination folder (named after the archive by default; emptied, it means
  the browsed directory itself), what to do with files that already exist (stop, skip them or
  replace them) and whether to send the archive to the trash once it is extracted. With several
  archives marked each goes to a folder named after it. With "stop" a folder that already exists
  is refused inside the form before anything is written. `:extract` behaves as before.
- `tui`, `file_ops`: a compress form on the right-click menu. "Compress…" on a file or folder (with
  a count when several are marked) opens a form for the archive's name, format (zip, tar.gz, tar),
  level (fast, normal, smallest), whether to make one archive per item, and whether to send the
  originals to the trash once their archive is complete. A name that is empty, reserved or already
  taken is reported inside the form. Typing an extension such as `.tar.gz` in the name picks the
  format too. `file_ops::archive` gained `Level` and `compress_with_level`; `compress` is
  unchanged. Passwords are not supported yet.
- `file_ops`, `tui`: compress and extract archives. `:extract` unpacks every marked archive (or the
  selected one) into a directory named after it, and `:compress <name>` packs the marked entries
  (or the selection) into `<name>`, with `.zip`, `.tar` or `.tar.gz` choosing the format. Both run
  in the background with progress and cancel with `Esc`. Extraction refuses entries that would
  land outside the destination (absolute paths, `..`, symlinked directories, control characters
  in names), skips links that could point outside it, clamps permissions, and stops at 1,000,000
  entries or 32 GiB written. A new archive is built beside its target and moved into place only
  when complete. An existing destination is an error. `rar`, `7z`, `xz`, `zstd` and `bzip2` are
  not supported.
- `tui`: saved custom themes can now be reselected, renamed and deleted from the appearance
  popup, through a new "Saved themes" category. `Enter` or a click applies a theme's snapshot as
  the live look, `r` renames it (an empty name or one already in use is refused), and `d` deletes
  it after a `y`/`n` confirmation. Renaming the active theme keeps tracking it; deleting it leaves
  the live look alone and stops tracking a saved theme. The popup's label column is also wider, so
  category labels and long theme names are no longer cut off.
- `theming`, `tui`: the appearance popup is now a full editor. It opens on five categories (Theme &
  colors, Border & separator, Glyphs, Text styles, Font) instead of one flat list, and "Theme &
  colors" covers all 22 `Theme` color fields rather than six. A color row's `Tab` mode is now a
  real two-axis picker, a saturation/value gradient square plus a hue strip, settable by click or
  drag, in addition to typed hex. "Text styles" edits each `[style]` element's modifiers with a
  live preview, and "Font" edits the family and size; both persist to `local.toml`'s new `[style]`
  and `[font]` tables. `theming` gains `Hsv::to_rgb`, `Mods::names`, `Styles::overlay_raw`/
  `by_index`/`with_index` and `RawStyles::set_by_index`, and `RawStyles`/`RawFont` now serialize.
- `theming`, `tui`: Ranger-style directory bookmarks. `` ` `` then a letter or digit jumps to the
  directory saved under that register; `B` then a letter or digit saves the browsed directory
  there (`m` was already `cut`, so bookmark-set defaults to `B` instead of Ranger's own `m`). Both
  are one-shot chords, shown in the status bar's mode pill (`BOOKMARK`/`SET BOOKMARK`) while
  pending, with `Esc` to cancel and any other key both ending the chord and still dispatching
  normally. Registers persist to `local.toml`'s new `[bookmarks]` table, the same way saved custom
  themes already do. A jump to an empty register reports it in the status line instead of erroring.
- `tui`: a "caps lock" header pill, shown for as long as Caps Lock is on and hidden the moment
  it's off. `App` now tracks the state itself (`caps_lock`/`set_caps_lock`), updated from every
  key event's `KeyEventState` — including under a modal view or prompt — so the pill stays
  correct rather than only being read where `with_caps_lock_applied` already flips a letter's
  case. Static, not blinking, matching the taste call already on record against a pulsing effect.
- `theming`, `tui`: the rest of the UI overhaul's cinematic layer — a boot splash, gradient
  borders/titles, a typewriter preview reveal, and an optional system HUD; the pulsing selection
  raised alongside these last cycle is dropped from scope for good (built, verified, and cut on
  taste alone before that commit — see `DIARY.md`). A brief splash (`[config] boot_splash`,
  default on) shows the app name over the browser for under a second on startup, dismissed by the
  first keypress or click. The focused browser pane's border and title now paint as a gradient
  between the theme's accent and focused-border colors (`gradient_borders`, default on) via a new
  `tui::gradient`, falling back to the existing flat look wherever the two colors can't be
  blended. The text preview streams in from the top, line by line, the first time a file is
  selected (`typewriter_preview`, default on; not once it's been scrolled, and not on a `reload`
  of the same file). A new header segment (`system_hud`, default off) shows session uptime, load
  average and memory used/total, read straight from `/proc` — no new dependency; a `sysinfo`-based
  cross-platform version was measured (+276 KiB, 8 new transitive crates including `rayon`) and
  set aside for if this ever needs to run somewhere without a `/proc`.
- `tui`: the start of the UI overhaul's cinematic layer — an animation tick and an animated
  shell-pane focus transition. A shell pane's border now fades between its dim and lit colors over
  180ms when focus moves to or away from it, instead of switching instantly. A new `tui::anim`
  module holds the pure transition math, and a new `style::blend_rgb` interpolates two resolved
  colors when both are truecolor RGB — any theme built from named ANSI colors, or running without
  a truecolor terminal, keeps the previous instant switch rather than guessing an in-between shade.
- `theming`, `tui`: external previewers for files the built-in preview can't otherwise show (a
  PDF, a video, ...). A new `[[preview_hook]]` config table (mirroring `[[open_with]]`'s style)
  names a command per extension and a `kind` (`image` or `text`); the command runs under `sh -c`
  with `{in}`/`{out}` substituted, off the render thread, killed after a timeout (default 10s) —
  the same spawn/poll/timeout/kill shape `git_status` already uses for `git`. An `image` result
  decodes through the same path a real image file uses and renders in the existing image
  pipeline; a `text` result reads back bounded and renders through the existing text pipeline,
  syntax-highlighted the same as any other text file. A missing program, a non-zero exit, a
  timeout, or unreadable output all fall back to quietly showing just the file's name — never an
  error, never a hex dump of a format that was never meant to be read as bytes. See
  `config.example.toml` for a `pdftoppm`/`ffmpegthumbnailer` example.
- `preview`, `theming`, `tui`: syntax highlighting in the text preview. New `preview::highlight`
  tokenizes a previewed file with `syntect` (parsing only, no theme/HTML machinery) and classifies
  each token by its raw scope name into a small, curated set (keyword/string/comment/number/
  function/type/plain) rather than one theme field per possible scope. Every bundled theme
  (default, Dracula, Catppuccin Mocha/Latte, Nord) gained six `syntax_*_fg` colors, using each
  theme's own published mapping where one exists. Tokenizing runs off the render thread alongside
  the file read itself, the same treatment already given image decoding and archive listing, so a
  slow-to-parse file can't stall input; `preview_view::draw_text` turns the tokens into styled
  spans, falling back to the ordinary preview text color for plain text and for any language
  `syntect` doesn't recognize.
- `trash` (new crate wiring), `file_ops`, `theming`, `tui`: a built-in trash bin, integrated with
  the real desktop trash (freedesktop.org trash on Linux, the same one Nautilus/Dolphin/Explorer/
  Finder use) rather than a private minuteman-only folder. `d` now sends the marked-or-selected
  entries to the trash — `Enter` or `y` confirms, since it's reversible — and a new `D` (Shift+d)
  keeps the previous permanent, trash-bypassing delete (`y` only), the same convention a desktop
  file manager's Delete vs. Shift+Delete uses. `:trash` at the `:` prompt does the same as `d`;
  anything typed after "trash" defers to the shell instead. New `trash::send` wraps the `trash`
  crate (renamed `os_trash` locally, since the workspace already has a same-named crate scaffolded
  for this), and `file_ops::trash` exposes it alongside `copy`/`mv`/`delete`/`rename` — deliberately
  not `Vfs`-based like the rest of `file_ops`, since the desktop trash has no remote-backend
  equivalent (a non-local `Vfs` should fall back to `delete` instead, not currently reachable since
  every call site is still `LocalVfs`). See `config.example.toml`'s `delete`/`delete_permanently`.
- `theming`, `tui`: a color picker and saved custom themes in the appearance popup. Any color row
  can now be edited as HSV sliders instead of typed hex — `Tab` mid-edit toggles between the two,
  `Up`/`Down` pick H/S/V and `Left`/`Right` nudge the selected channel, and every color row shows a
  live swatch of its own value. A new "Save theme" row saves the live look under a name; if it
  still matches the custom theme it started from it says so and does nothing, if it's drifted it
  asks whether to update that theme or save a new one, and if nothing's active yet it just asks for
  a name. Saved themes are full color snapshots (`theming::RawTheme::from_theme`), stored in
  `local.toml`'s new `[[custom_themes]]` array alongside a new `active_custom_theme` field, so they
  survive restarts and keep looking the same regardless of later `config.toml`/`appearance.toml`
  edits. New `theming::color` module (`Hsv`, `hex_to_hsv`) and `theming::CustomTheme`.
- `plugins` (new crate), `theming`, `tui`: a plugin system. A `[[plugin]]` entry in `config.toml`
  (`name`, `command`, `args`, an optional `on_key`) spawns an external process at startup that
  speaks a small, versioned, line-delimited JSON-RPC protocol over its own stdin/stdout — any
  language that can read a line and print one works, with no compile step and no per-language
  host bindings. Each plugin gets an `init` event at spawn and a `key` event when its `on_key` is
  pressed (whichever key that is stays otherwise unbound — the built-in keymap is untouched), both
  carrying the current directory and selection (marks, or the entry under the cursor). A plugin
  calls back into `read_dir`, `copy`, `mv`, `delete`, `create_dir`, `create_file`, `touch` and
  `rename` — the same `file_ops` orchestration the built-in keys use — and a `log` notification
  surfaces as the status bar message. Deliberately unsandboxed for this first stage (a plugin runs
  with Minuteman's own OS permissions); the WASM/Extism host already on the Low Priority roadmap
  is the sandboxed tier meant to sit alongside this, not replace it. See `config.example.toml`.
- `theming`: two new built-in palettes, `catppuccin` (Mocha) and `nord`, plus `catppuccin-latte`
  (Catppuccin's light flavor, reachable by name but not in the appearance popup's Theme cycle).
  The Theme row (`neon`/`classic`/`dracula`/`catppuccin`/`nord`) later moved from the settings
  popup to the appearance popup — see below.
- `tui`/`theming`: the default look now adapts to the terminal's own background instead of always
  being the neon palette. When `[theme]` is left out of both config files entirely, Minuteman asks
  the terminal for its background color (OSC 11, bundled into the existing graphics-capability
  probe in `ImagePreview::new`) and picks Catppuccin Mocha or Latte to match it, falling back to
  `neon` if the terminal never answers. Setting anything under `[theme]` — a `name`, or even one
  field — opts back out and pins the look exactly as configured; `Config::theme_is_customized`
  carries that distinction. See `appearance.example.toml`.
- `tui`: an appearance popup, `a` (new `[keys] appearance`, `Action::Appearance`) or
  "Appearance…" on blank space's right-click menu. Theme cycles the base palette; six colors
  (Accent, Focused border, Selection, Directory, Status bar, Danger) are edited as free text — a
  name or hex, live-previewed as you type, `Enter` to confirm, `Esc` to cancel; Border style,
  Separator and Glyphs cycle through their fixed choices; Reset to defaults clears every change
  this popup made this session. Unlike the settings popup, it is genuinely mouse-driven: a click
  on a row acts on it exactly like `Enter` would, via new `appearance_popup::hit`/`click_row`. New
  `theming::Theme::overlay_raw` layers these live edits onto whichever theme is otherwise active
  (`RawTheme`'s own `From` impl is now built on it too).
- `theming`/`tui`: the settings and appearance popups' changes are saved. Every commit from
  either — a settings-popup toggle, a color/border/separator/glyph/Theme edit, Reset to defaults —
  is written at once to a new, program-owned `~/.config/minuteman/local.toml`, layered above both
  `config.toml` and `appearance.toml` (so it wins over them, without either ever being touched or
  risking their comments) and read back on the next launch. It stays sparse — only ever the
  fields a popup actually changed — via new `Serialize` impls on `RawTheme`/`RawUi`/`RawPanels`
  and new `RawPanels::overlay`/`PanelsConfig::overlay_raw`, so a later hand-edit to
  `appearance.toml` is never masked by a stale save. New `Config::save_local`,
  `Config::local_theme`/`local_ui`/`local_panels` (the unmerged `local.toml` layer, for seeding a
  popup's live state), and `RawLocal`.
- `tui`: the settings popup's Theme row moved to the appearance popup (now its first row) — it
  belongs with the rest of the look it picks a base for, and having it in two popups risked them
  drifting on what "the theme" currently is. The settings popup now cycles Columns, HUD and
  Command bar only.

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
