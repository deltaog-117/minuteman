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

//! How a plugin's `command` is started: as an ordinary process, or, when it names a `.wasm`
//! module, inside the sandbox (see `sandbox` and, with the `wasm` feature, `wasm`). Deciding this
//! once, here, means everything after it matches on the answer and can't mix the two up.

use std::path::Path;

/// What a build without the `wasm` feature says to someone who configured a module.
#[cfg(not(feature = "wasm"))]
pub(crate) const NO_WASM_SUPPORT: &str = "this build of mman has no WebAssembly support; \
     rebuild with `cargo install --path crates/tui --features wasm`";

/// What a plugin's configured `command` and `args` ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Launch<'a> {
    /// A program run with the user's own permissions.
    Process {
        command: &'a str,
        args: &'a [String],
    },
    /// A WebAssembly module run with no access but the protocol. It is given no arguments, so any
    /// `args` configured for it are not passed on.
    Wasm {
        module: &'a Path,
        ignored_args: bool,
    },
}

impl<'a> Launch<'a> {
    /// A command whose extension is `wasm` (any case) is a module; anything else is a program.
    pub(crate) fn of(command: &'a str, args: &'a [String]) -> Self {
        let path = Path::new(command);
        let is_module = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("wasm"));
        if is_module {
            Self::Wasm {
                module: path,
                ignored_args: !args.is_empty(),
            }
        } else {
            Self::Process { command, args }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_wasm_extension_in_any_case_is_a_module() {
        for command in [
            "finder.wasm",
            "/opt/p/Finder.WASM",
            "./x.Wasm",
            "~/a b/c.wasm",
        ] {
            assert!(
                matches!(Launch::of(command, &[]), Launch::Wasm { .. }),
                "{command}"
            );
        }
    }

    #[test]
    fn programs_and_near_misses_are_processes() {
        for command in [
            "mman-finder",
            "sh",
            "wasm",
            "a.wasm.sh",
            "a.wasmx",
            "dir.wasm/run",
            "",
        ] {
            assert!(
                matches!(Launch::of(command, &[]), Launch::Process { .. }),
                "{command}"
            );
        }
    }

    #[test]
    fn arguments_given_to_a_module_are_reported_as_ignored() {
        let args = vec!["--fast".to_string()];
        assert!(matches!(
            Launch::of("p.wasm", &args),
            Launch::Wasm {
                ignored_args: true,
                ..
            }
        ));
        assert!(matches!(
            Launch::of("p.wasm", &[]),
            Launch::Wasm {
                ignored_args: false,
                ..
            }
        ));
    }

    proptest! {
        #[test]
        fn exactly_the_commands_ending_in_dot_wasm_are_modules(stem in "[a-z]{1,8}", tail in "[a-z]{0,4}") {
            let module = format!("{stem}.wasm");
            let other = format!("{stem}.{tail}x");
            let is_module = matches!(Launch::of(&module, &[]), Launch::Wasm { .. });
            let is_process = matches!(Launch::of(&other, &[]), Launch::Process { .. });
            prop_assert!(is_module);
            prop_assert!(is_process);
        }
    }
}
