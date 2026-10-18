// Minuteman - a fast, Ranger-inspired terminal file manager
// Copyright (C) 2026  Davi Oliveira Gonçalves
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Command-line parsing: `minuteman [--cwd-file <path>] [--chooser-file <path> [--pick <kind>]
//! [--pick-name <name>]] [start_dir]`, `minuteman init
//! <bash|zsh|fish>`, `minuteman init-terminal <kitty|alacritty|wezterm>`, `minuteman init-appearance`,
//! `minuteman glyphs`, and `minuteman plugin <check <name or folder> | new <name> [--lang sh|python] |
//! add <folder>>`.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::{fs, io};

use shared::chooser::PickKind;

use crate::plugin_new::{Lang, NameError, PluginName};
use crate::shell_init::Shell;
use crate::terminal_init::Terminal;

/// What the process was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Print the shell wrapper for `Shell` and exit, without starting the TUI.
    Init(Shell),
    /// Print a font + color snippet for `Terminal` and exit.
    InitTerminal(Terminal),
    /// Print the commented default `appearance.toml` and exit.
    InitAppearance,
    /// Print a sample of every glyph set, to see which ones the terminal's font can draw.
    Glyphs,
    /// Start one plugin for half a second and say whether it looks healthy, then exit.
    PluginCheck(String),
    /// Write a working plugin to start from into the drop-in folder.
    PluginNew(PluginName, Lang),
    /// Copy a plugin folder into the drop-in folder.
    PluginAdd(PathBuf),
    Run(RunArgs),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RunArgs {
    pub start_dir: Option<PathBuf>,
    /// Where `Q` records the directory being browsed. `None` makes `Q` behave like `q`.
    pub cwd_file: Option<PathBuf>,
    /// Set when another program asked for a pick (see [`ChooserArgs`]).
    pub chooser: Option<ChooserArgs>,
}

/// A request to pick something and write the answer to `file` (see `shared::chooser`).
#[derive(Debug, PartialEq, Eq)]
pub struct ChooserArgs {
    pub file: PathBuf,
    pub kind: PickKind,
    /// The file name to offer when saving.
    pub suggested_name: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CliError {
    MissingValue(&'static str),
    UnknownFlag(String),
    UnknownShell(String),
    MissingShell,
    UnknownTerminal(String),
    MissingTerminal,
    ExtraArgument(PathBuf),
    UnknownPick(String),
    /// `--pick` or `--pick-name` given without anywhere to write the answer.
    PickWithoutChooserFile(&'static str),
    /// `plugin` without `check`, `new` or `add`, or without the one thing they take.
    PluginUsage,
    BadPluginName(NameError),
    UnknownLang(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingValue(flag) => write!(f, "{flag} needs a value"),
            Self::UnknownFlag(flag) => write!(f, "unknown option: {flag}"),
            Self::UnknownShell(name) => {
                write!(
                    f,
                    "unsupported shell '{name}' (expected bash, zsh, or fish)"
                )
            }
            Self::MissingShell => write!(f, "init needs a shell: bash, zsh, or fish"),
            Self::UnknownTerminal(name) => write!(
                f,
                "unsupported terminal '{name}' (expected kitty, alacritty, or wezterm)"
            ),
            Self::MissingTerminal => {
                write!(
                    f,
                    "init-terminal needs a terminal: kitty, alacritty, or wezterm"
                )
            }
            Self::ExtraArgument(path) => {
                write!(f, "unexpected extra argument: {}", path.display())
            }
            Self::UnknownPick(name) => write!(
                f,
                "unsupported pick '{name}' (expected file, files, directory, or save)"
            ),
            Self::PickWithoutChooserFile(flag) => write!(f, "{flag} needs --chooser-file"),
            Self::PluginUsage => write!(
                f,
                "usage: plugin check <plugin name or folder> | plugin new <name> [--lang sh|python] | plugin add <folder>"
            ),
            Self::BadPluginName(e) => write!(f, "{e}"),
            Self::UnknownLang(name) => {
                write!(f, "unsupported language '{name}' (expected sh or python)")
            }
        }
    }
}

impl std::error::Error for CliError {}

/// Parses everything after the program name. `init`, `init-terminal`, `plugin` and `glyphs` as the
/// *first* argument are subcommands (browse a directory literally named one of those as
/// `./init`).
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, CliError> {
    let mut args = args.into_iter();
    let mut run = RunArgs::default();
    let mut first = true;
    let mut chooser_file: Option<PathBuf> = None;
    let mut pick: Option<PickKind> = None;
    let mut pick_name: Option<String> = None;

    while let Some(arg) = args.next() {
        let is_first = std::mem::take(&mut first);
        if is_first && arg == "init-appearance" {
            return match args.next() {
                None => Ok(Command::InitAppearance),
                Some(extra) => Err(CliError::ExtraArgument(PathBuf::from(extra))),
            };
        }
        if is_first && arg == "glyphs" {
            return match args.next() {
                None => Ok(Command::Glyphs),
                Some(extra) => Err(CliError::ExtraArgument(PathBuf::from(extra))),
            };
        }
        if is_first && arg == "plugin" {
            return parse_plugin(args);
        }
        if is_first && arg == "init-terminal" {
            let name = args.next().ok_or(CliError::MissingTerminal)?;
            let name = name.to_string_lossy();
            return Terminal::parse(&name)
                .map(Command::InitTerminal)
                .ok_or_else(|| CliError::UnknownTerminal(name.into_owned()));
        }
        if is_first && arg == "init" {
            let name = args.next().ok_or(CliError::MissingShell)?;
            let name = name.to_string_lossy();
            return Shell::parse(&name)
                .map(Command::Init)
                .ok_or_else(|| CliError::UnknownShell(name.into_owned()));
        }

        if arg == "--cwd-file" {
            let value = args.next().ok_or(CliError::MissingValue("--cwd-file"))?;
            run.cwd_file = Some(PathBuf::from(value));
        } else if let Some(value) = arg.to_str().and_then(|a| a.strip_prefix("--cwd-file=")) {
            run.cwd_file = Some(PathBuf::from(value));
        } else if arg == "--chooser-file" {
            let value = args
                .next()
                .ok_or(CliError::MissingValue("--chooser-file"))?;
            chooser_file = Some(PathBuf::from(value));
        } else if let Some(value) = arg.to_str().and_then(|a| a.strip_prefix("--chooser-file=")) {
            chooser_file = Some(PathBuf::from(value));
        } else if arg == "--pick" {
            let value = args.next().ok_or(CliError::MissingValue("--pick"))?;
            pick = Some(parse_pick(&value.to_string_lossy())?);
        } else if let Some(value) = arg.to_str().and_then(|a| a.strip_prefix("--pick=")) {
            pick = Some(parse_pick(value)?);
        } else if arg == "--pick-name" {
            let value = args.next().ok_or(CliError::MissingValue("--pick-name"))?;
            pick_name = Some(value.to_string_lossy().into_owned());
        } else if let Some(value) = arg.to_str().and_then(|a| a.strip_prefix("--pick-name=")) {
            pick_name = Some(value.to_owned());
        } else if arg
            .to_str()
            .is_some_and(|a| a.starts_with('-') && a.len() > 1)
        {
            return Err(CliError::UnknownFlag(arg.to_string_lossy().into_owned()));
        } else if run.start_dir.is_some() {
            return Err(CliError::ExtraArgument(PathBuf::from(arg)));
        } else {
            run.start_dir = Some(PathBuf::from(arg));
        }
    }

    run.chooser = match chooser_file {
        Some(file) => Some(ChooserArgs {
            file,
            kind: pick.unwrap_or_default(),
            suggested_name: pick_name,
        }),
        None if pick.is_some() => return Err(CliError::PickWithoutChooserFile("--pick")),
        None if pick_name.is_some() => return Err(CliError::PickWithoutChooserFile("--pick-name")),
        None => None,
    };

    Ok(Command::Run(run))
}

/// The words after `plugin`.
fn parse_plugin(mut args: impl Iterator<Item = OsString>) -> Result<Command, CliError> {
    let word = args.next().ok_or(CliError::PluginUsage)?;
    if word == "check" {
        let target = args.next().ok_or(CliError::PluginUsage)?;
        return match args.next() {
            None => Ok(Command::PluginCheck(target.to_string_lossy().into_owned())),
            Some(extra) => Err(CliError::ExtraArgument(PathBuf::from(extra))),
        };
    }
    if word == "add" {
        let folder = args.next().ok_or(CliError::PluginUsage)?;
        return match args.next() {
            None => Ok(Command::PluginAdd(PathBuf::from(folder))),
            Some(extra) => Err(CliError::ExtraArgument(PathBuf::from(extra))),
        };
    }
    if word != "new" {
        return Err(CliError::PluginUsage);
    }
    let mut name: Option<PluginName> = None;
    let mut lang = Lang::default();
    while let Some(arg) = args.next() {
        let arg = arg.to_string_lossy().into_owned();
        let value = if arg == "--lang" {
            let value = args.next().ok_or(CliError::MissingValue("--lang"))?;
            Some(value.to_string_lossy().into_owned())
        } else {
            arg.strip_prefix("--lang=").map(str::to_owned)
        };
        if let Some(value) = value {
            lang = Lang::parse(&value).ok_or(CliError::UnknownLang(value))?;
        } else if arg.starts_with('-') && arg.len() > 1 {
            return Err(CliError::UnknownFlag(arg));
        } else if name.is_some() {
            return Err(CliError::ExtraArgument(PathBuf::from(arg)));
        } else {
            name = Some(PluginName::try_new(&arg).map_err(CliError::BadPluginName)?);
        }
    }
    Ok(Command::PluginNew(name.ok_or(CliError::PluginUsage)?, lang))
}

fn parse_pick(name: &str) -> Result<PickKind, CliError> {
    PickKind::parse(name).ok_or_else(|| CliError::UnknownPick(name.to_owned()))
}

/// Records `dir` as raw bytes with no trailing newline, so the wrapper reads back exactly the
/// path — including one that isn't valid UTF-8.
pub fn write_cwd_file(file: &Path, dir: &Path) -> io::Result<()> {
    fs::write(file, dir.as_os_str().as_encoded_bytes())
}

/// Writes what `--chooser-file` was asked to hold: the picked paths, or nothing for a cancel.
pub fn write_chooser_file(file: &Path, answer: &[u8]) -> io::Result<()> {
    fs::write(file, answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<Command, CliError> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn no_arguments_runs_with_defaults() {
        assert_eq!(parse_strs(&[]), Ok(Command::Run(RunArgs::default())));
    }

    #[test]
    fn positional_argument_is_the_start_dir() {
        assert_eq!(
            parse_strs(&["/tmp"]),
            Ok(Command::Run(RunArgs {
                start_dir: Some("/tmp".into()),
                cwd_file: None,
                chooser: None,
            }))
        );
    }

    #[test]
    fn cwd_file_accepts_both_spellings_in_any_position() {
        let expected = Ok(Command::Run(RunArgs {
            start_dir: Some("/tmp".into()),
            cwd_file: Some("/x/out".into()),
            chooser: None,
        }));
        assert_eq!(parse_strs(&["--cwd-file", "/x/out", "/tmp"]), expected);
        assert_eq!(parse_strs(&["/tmp", "--cwd-file=/x/out"]), expected);
    }

    #[test]
    fn cwd_file_without_a_value_is_an_error() {
        assert_eq!(
            parse_strs(&["--cwd-file"]),
            Err(CliError::MissingValue("--cwd-file"))
        );
    }

    #[test]
    fn unknown_flags_and_extra_positionals_are_errors() {
        assert_eq!(
            parse_strs(&["--nope"]),
            Err(CliError::UnknownFlag("--nope".into()))
        );
        assert_eq!(
            parse_strs(&["/a", "/b"]),
            Err(CliError::ExtraArgument("/b".into()))
        );
    }

    #[test]
    fn chooser_file_defaults_to_picking_one_file() {
        assert_eq!(
            parse_strs(&["--chooser-file", "/x/out"]),
            Ok(Command::Run(RunArgs {
                chooser: Some(ChooserArgs {
                    file: "/x/out".into(),
                    kind: PickKind::File,
                    suggested_name: None,
                }),
                ..RunArgs::default()
            }))
        );
    }

    #[test]
    fn pick_and_pick_name_accept_both_spellings() {
        let expected = Ok(Command::Run(RunArgs {
            start_dir: Some("/tmp".into()),
            chooser: Some(ChooserArgs {
                file: "/x/out".into(),
                kind: PickKind::Save,
                suggested_name: Some("a b.pdf".into()),
            }),
            ..RunArgs::default()
        }));
        assert_eq!(
            parse_strs(&[
                "--chooser-file=/x/out",
                "--pick",
                "save",
                "--pick-name",
                "a b.pdf",
                "/tmp",
            ]),
            expected
        );
        assert_eq!(
            parse_strs(&[
                "/tmp",
                "--chooser-file",
                "/x/out",
                "--pick=save",
                "--pick-name=a b.pdf",
            ]),
            expected
        );
    }

    #[test]
    fn pick_needs_somewhere_to_write_the_answer() {
        assert_eq!(
            parse_strs(&["--pick", "files"]),
            Err(CliError::PickWithoutChooserFile("--pick"))
        );
        assert_eq!(
            parse_strs(&["--pick-name", "x"]),
            Err(CliError::PickWithoutChooserFile("--pick-name"))
        );
    }

    #[test]
    fn an_unknown_pick_and_missing_values_are_errors() {
        assert_eq!(
            parse_strs(&["--chooser-file", "/x", "--pick", "folder"]),
            Err(CliError::UnknownPick("folder".into()))
        );
        assert_eq!(
            parse_strs(&["--chooser-file"]),
            Err(CliError::MissingValue("--chooser-file"))
        );
        assert_eq!(
            parse_strs(&["--chooser-file", "/x", "--pick"]),
            Err(CliError::MissingValue("--pick"))
        );
    }

    #[test]
    fn init_selects_a_shell() {
        assert_eq!(parse_strs(&["init", "zsh"]), Ok(Command::Init(Shell::Zsh)));
        assert_eq!(parse_strs(&["init"]), Err(CliError::MissingShell));
        assert_eq!(
            parse_strs(&["init", "tcsh"]),
            Err(CliError::UnknownShell("tcsh".into()))
        );
    }

    #[test]
    fn init_terminal_selects_a_terminal() {
        assert_eq!(
            parse_strs(&["init-terminal", "kitty"]),
            Ok(Command::InitTerminal(Terminal::Kitty))
        );
        assert_eq!(
            parse_strs(&["init-terminal"]),
            Err(CliError::MissingTerminal)
        );
        assert_eq!(
            parse_strs(&["init-terminal", "xterm"]),
            Err(CliError::UnknownTerminal("xterm".into()))
        );
    }

    #[test]
    fn init_appearance_is_a_bare_subcommand() {
        assert_eq!(
            parse_strs(&["init-appearance"]),
            Ok(Command::InitAppearance)
        );
        assert_eq!(
            parse_strs(&["init-appearance", "x"]),
            Err(CliError::ExtraArgument("x".into()))
        );
    }

    #[test]
    fn glyphs_is_a_bare_subcommand() {
        assert_eq!(parse_strs(&["glyphs"]), Ok(Command::Glyphs));
        assert_eq!(
            parse_strs(&["glyphs", "nerd"]),
            Err(CliError::ExtraArgument("nerd".into()))
        );
    }

    #[test]
    fn plugin_check_takes_exactly_one_target() {
        assert_eq!(
            parse_strs(&["plugin", "check", "git"]),
            Ok(Command::PluginCheck("git".into()))
        );
        assert_eq!(parse_strs(&["plugin"]), Err(CliError::PluginUsage));
        assert_eq!(parse_strs(&["plugin", "check"]), Err(CliError::PluginUsage));
        assert_eq!(
            parse_strs(&["plugin", "remove", "x"]),
            Err(CliError::PluginUsage)
        );
        assert_eq!(
            parse_strs(&["plugin", "check", "a", "b"]),
            Err(CliError::ExtraArgument("b".into()))
        );
    }

    #[test]
    fn init_is_only_a_subcommand_in_first_position() {
        assert_eq!(
            parse_strs(&["/tmp", "init"]),
            Err(CliError::ExtraArgument("init".into()))
        );
    }

    #[test]
    fn cwd_file_round_trips_the_exact_path() {
        let file = std::env::temp_dir().join(format!("minuteman-cwd-test-{}", std::process::id()));
        let dir = Path::new("/some/dir with spaces");
        write_cwd_file(&file, dir).expect("temp dir is writable");
        let read = fs::read(&file).expect("just written");
        let _ = fs::remove_file(&file);
        assert_eq!(read, dir.as_os_str().as_encoded_bytes());
    }
    #[test]
    fn plugin_new_takes_a_name_and_an_optional_language() {
        let name = PluginName::try_new("hello").unwrap();
        assert_eq!(
            parse_strs(&["plugin", "new", "hello"]),
            Ok(Command::PluginNew(name.clone(), Lang::Sh))
        );
        for spelling in [&["--lang", "python"][..], &["--lang=python"]] {
            let mut args = vec!["plugin", "new", "hello"];
            args.extend_from_slice(spelling);
            assert_eq!(
                parse_strs(&args),
                Ok(Command::PluginNew(name.clone(), Lang::Python))
            );
        }
        assert_eq!(
            parse_strs(&["plugin", "new", "--lang", "sh", "hello"]),
            Ok(Command::PluginNew(name, Lang::Sh))
        );
    }

    #[test]
    fn plugin_new_refuses_what_cannot_be_a_name_a_language_or_an_argument() {
        assert_eq!(parse_strs(&["plugin", "new"]), Err(CliError::PluginUsage));
        assert_eq!(
            parse_strs(&["plugin", "new", "../x"]),
            Err(CliError::BadPluginName(NameError::BadStart('.')))
        );
        assert_eq!(
            parse_strs(&["plugin", "new", "x", "--lang", "rust"]),
            Err(CliError::UnknownLang("rust".into()))
        );
        assert_eq!(
            parse_strs(&["plugin", "new", "x", "--lang"]),
            Err(CliError::MissingValue("--lang"))
        );
        assert_eq!(
            parse_strs(&["plugin", "new", "x", "--bogus"]),
            Err(CliError::UnknownFlag("--bogus".into()))
        );
        assert_eq!(
            parse_strs(&["plugin", "new", "x", "y"]),
            Err(CliError::ExtraArgument("y".into()))
        );
    }

    #[test]
    fn plugin_add_takes_exactly_one_folder() {
        assert_eq!(
            parse_strs(&["plugin", "add", "./p"]),
            Ok(Command::PluginAdd("./p".into()))
        );
        assert_eq!(parse_strs(&["plugin", "add"]), Err(CliError::PluginUsage));
        assert_eq!(
            parse_strs(&["plugin", "add", "a", "b"]),
            Err(CliError::ExtraArgument("b".into()))
        );
        assert_eq!(
            parse_strs(&["plugin", "remove"]),
            Err(CliError::PluginUsage)
        );
    }
}
