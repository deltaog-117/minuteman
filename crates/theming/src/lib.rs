//! Config loading, keybinding resolution, and theme colors — read from
//! `~/.config/minuteman/config.toml`, with built-in defaults when absent or invalid.

pub mod config;
pub mod keymap;
pub mod theme;

pub use config::Config;
pub use keymap::{Action, KeyMap};
pub use theme::Theme;
