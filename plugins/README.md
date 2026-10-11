# Bundled plugins

One crate per plugin that ships with Minuteman and is enabled on a fresh install. Each one talks to
the program through `plugin_api` only; nothing under `crates/` may depend on anything here (the
`dependency_direction` test in `crates/plugin_host` fails if it does), and a plugin may depend on
`plugin_api` and `shared` but never on `plugin_host`.

A plugin is added to the workspace `members` in the root `Cargo.toml` when its crate is created.
Installed, they live in `<prefix>/share/minuteman/plugins/<name>/`; a user plugin of the same name
in `~/.config/minuteman/plugins/<name>/` takes its place.

A plugin's package name can differ from its folder: `plugins/preview` is the package `previewer`,
because `preview` is already the image-decoding crate in `crates/`. The program it installs is
still `mman-preview`.
