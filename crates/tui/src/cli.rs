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

//! Command-line parsing: `minuteman [--cwd-file <path>] [start_dir]` and
//! `minuteman init <bash|zsh|fish>`.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::{fs, io};

use crate::shell_init::Shell;

/// What the process was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Print the shell wrapper for `Shell` and exit, without starting the TUI.
    Init(Shell),
    Run(RunArgs),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RunArgs {
    pub start_dir: Option<PathBuf>,
    /// Where `Q` records the directory being browsed. `None` makes `Q` behave like `q`.
    pub cwd_file: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CliError {
    MissingValue(&'static str),
    UnknownFlag(String),
    UnknownShell(String),
    MissingShell,
    ExtraArgument(PathBuf),
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
            Self::ExtraArgument(path) => {
                write!(f, "unexpected extra argument: {}", path.display())
            }
        }
    }
}

impl std::error::Error for CliError {}

/// Parses everything after the program name. `init` as the *first* argument is the subcommand
/// (browse a directory literally named `init` as `./init`).
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, CliError> {
    let mut args = args.into_iter();
    let mut run = RunArgs::default();
    let mut first = true;

    while let Some(arg) = args.next() {
        if std::mem::take(&mut first) && arg == "init" {
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

    Ok(Command::Run(run))
}

/// Records `dir` as raw bytes with no trailing newline, so the wrapper reads back exactly the
/// path — including one that isn't valid UTF-8.
pub fn write_cwd_file(file: &Path, dir: &Path) -> io::Result<()> {
    fs::write(file, dir.as_os_str().as_encoded_bytes())
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
            }))
        );
    }

    #[test]
    fn cwd_file_accepts_both_spellings_in_any_position() {
        let expected = Ok(Command::Run(RunArgs {
            start_dir: Some("/tmp".into()),
            cwd_file: Some("/x/out".into()),
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
    fn init_selects_a_shell() {
        assert_eq!(parse_strs(&["init", "zsh"]), Ok(Command::Init(Shell::Zsh)));
        assert_eq!(parse_strs(&["init"]), Err(CliError::MissingShell));
        assert_eq!(
            parse_strs(&["init", "tcsh"]),
            Err(CliError::UnknownShell("tcsh".into()))
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
}
