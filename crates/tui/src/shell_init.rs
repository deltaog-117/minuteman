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

//! The shell-side half of "quit into the directory minuteman was browsing" (`Q`).
//!
//! A process can't change its parent shell's working directory, so `Q` only *records* the
//! directory (see `cli::write_cwd_file`); these wrappers, printed by `minuteman init <shell>`,
//! are what actually `cd` afterward. Same mechanism as ranger's `--choosedir` wrapper.

/// A shell `minuteman init` can emit a wrapper for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "fish" => Some(Self::Fish),
            _ => None,
        }
    }

    /// The wrapper function to `eval`/`source` from the shell's rc file. Named `mm` so the real
    /// `minuteman` binary stays directly runnable (without cd-on-quit) under its own name.
    pub fn wrapper(self) -> &'static str {
        match self {
            Self::Bash | Self::Zsh => POSIX_WRAPPER,
            Self::Fish => FISH_WRAPPER,
        }
    }
}

// `cd` only when the file is non-empty — `q`, `:q` and a crash never write it — and the
// directory still exists, so a stale or racing delete can't make the wrapper fail loudly.
const POSIX_WRAPPER: &str = r#"mm() {
  local tmp dir rc
  tmp="$(mktemp "${TMPDIR:-/tmp}/minuteman-cwd.XXXXXX")" || return
  command minuteman --cwd-file "$tmp" "$@"
  rc=$?
  if [ -s "$tmp" ]; then
    dir="$(cat -- "$tmp")"
    if [ -d "$dir" ] && [ "$dir" != "$PWD" ]; then
      builtin cd -- "$dir"
    fi
  fi
  rm -f -- "$tmp"
  return $rc
}
"#;

const FISH_WRAPPER: &str = r#"function mm
    set -l tmp (mktemp -t minuteman-cwd.XXXXXX); or return
    command minuteman --cwd-file $tmp $argv
    set -l rc $status
    if test -s $tmp
        set -l dir (cat $tmp | string collect)
        if test -d "$dir"; and test "$dir" != "$PWD"
            builtin cd -- $dir
        end
    end
    rm -f $tmp
    return $rc
end
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_supported_shells() {
        assert_eq!(Shell::parse("zsh"), Some(Shell::Zsh));
        assert_eq!(Shell::parse("bash"), Some(Shell::Bash));
        assert_eq!(Shell::parse("fish"), Some(Shell::Fish));
        assert_eq!(Shell::parse("tcsh"), None);
        assert_eq!(Shell::parse("ZSH"), None);
    }

    #[test]
    fn every_wrapper_passes_the_cwd_file_flag_and_defines_mm() {
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
            let w = shell.wrapper();
            assert!(
                w.contains("--cwd-file"),
                "{shell:?} wrapper lacks --cwd-file"
            );
            assert!(
                w.starts_with("mm()") || w.starts_with("function mm"),
                "{shell:?} wrapper doesn't define mm"
            );
        }
    }
}
