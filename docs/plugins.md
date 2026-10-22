# Writing a plugin: quick start

A plugin is a folder holding a `plugin.toml` and a program. Minuteman starts the program and talks
to it on its standard input and output, one JSON object per line, so a plugin can be written in
any language. Plugins of your own live in `~/.config/minuteman/plugins/<name>/`.

## Make one

```sh
mman plugin new hello                # a shell script
mman plugin new hello --lang python  # or a Python script
```

This writes `~/.config/minuteman/plugins/hello/` with a `plugin.toml` and a short program that
answers `:hello` with a line on the status bar. The name is 1 to 64 letters, digits, `-` and `_`,
starting with a letter or digit. A folder that already exists is never touched: the command says so
and exits 1. Rust is not offered yet.

## Check it

```sh
mman plugin check hello
```

`check` takes the name of a plugin the menu lists, or a folder holding a `plugin.toml`. It starts
the plugin for half a second and prints one line, for example `hello starts and keeps running`. It
exits 1 when something is wrong: the manifest is refused, the program ended at start, closed its
output, or logged an error. A key or `:` word already taken by the interface or another plugin is
reported here as well.

## Switch it on

A new plugin is off. Press `space`, then `p` to open the plugin menu, switch `hello` on, and run
`:hello` from the `:` line. The status bar says `hello from hello`. The menu remembers the choice
in `local.toml`.

## The manifest

```toml
name = "hello"
description = "A plugin made with `mman plugin new`."
command = "./plugin.sh"
api_version = 1

commands = ["hello"]          # run from the `:` line as `:hello`
# on_key = ""                 # run from a key; pick one that is free
# events = ["dir_changed"]    # also hear about dir_changed, selection_changed, job_finished
```

`api_version` must match what this build speaks; a plugin written for another version is refused
with the reason (`written for plugin API 99; this build speaks 1`).

## The conversation

Each line the plugin reads on stdin is an event, `{"jsonrpc":"2.0","method":...,"params":{...}}`,
and `params.name` says which one (`command` when its `:` word was run). A line the plugin prints
with `"method":"log"` shows on the status bar:

```json
{"jsonrpc":"2.0","method":"log","params":{"level":"info","message":"hello from hello"}}
```

The generated program in `plugin.sh` or `plugin.py` is the shortest working example.

## Add one from elsewhere

```sh
mman plugin add ~/path/to/folder
```

`add` runs the same checks as `plugin check` (manifest, API version, name, keys and `:` words
against the interface and the other plugins) and only if they pass copies the folder into
`~/.config/minuteman/plugins/`. It is a copy, not a link, and only from a local folder, never a
URL. A link or special file in the folder is refused, `.git` is left behind, and a folder over
5,000 files, 64 MiB or 16 levels deep is refused. Nothing is left half-copied. The added plugin is
off until you switch it on in the menu.

## Where the menu looks

In the plugin menu, `a` lists the plugins found in the folder you are browsing. To have it look in
other places too, name them in `config.toml`:

```toml
plugin_dirs = ["~/code/mman-plugins"]
```

Each folder is read like the browsed one: itself, and the folders directly inside it that hold a
`plugin.toml`. `~` is your home; an entry that is not an absolute path or not a folder is ignored.
It is read at startup.

## Write one in Rust, for the sandbox

A plugin can also be a WebAssembly module, which the program runs in a sandbox: no folders,
environment or network of its own, read-only access to the folder being browsed and below it.
`examples/plugins/wasm_stats/` is a complete one in Rust: it counts the files in the browsed folder
on `:wstats`, and `:wtouch NAME` shows the sandbox refusing a write.

```sh
rustup target add wasm32-wasip1
cd examples/plugins/wasm_stats
cargo build --release --target wasm32-wasip1
mkdir -p ~/.config/minuteman/plugins/wasm-stats
cp plugin.toml target/wasm32-wasip1/release/wasm_stats.wasm ~/.config/minuteman/plugins/wasm-stats/
mman plugin check wasm-stats
```

Switch it on in the plugin menu as above. Your `mman` must be built with the `wasm` feature
(`cargo install --path crates/tui --features wasm`); one built without it says so when the plugin
starts. The module reads and writes the same lines as any plugin, and ends with an error line if
the host's `api_version` in `init` is not the one it was written for.
