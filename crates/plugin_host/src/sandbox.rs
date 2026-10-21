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

//! What a sandboxed plugin (a `.wasm` module) may ask the host for. A module has no access to the
//! machine of its own, so everything it can do arrives as a request over the protocol, and this is
//! the one place that decides which requests are answered. The first cut is read-only: it may
//! look at the folder being browsed and everything below it, talk to the interface (state,
//! commands, pickers, questions), and nothing else. Anything that changes a file, and
//! `run_action` (which can reach a key's delete), is refused with a reason the plugin can log.
//!
//! The match in `Trust::check` has no wildcard arm, so a request added to the protocol later
//! cannot be left unclassified: the build fails until someone decides.
//!
//! A path is judged after resolving symlinks and `..` (`canonicalize`), so a link inside the
//! browsed folder that points out of it is refused. The check happens before the host reads, so a
//! link swapped in by another program between the two could still get through; a sandboxed plugin
//! cannot make links itself, which is the part this module can promise.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::protocol::PluginRequest;

/// How far a plugin is trusted with the host's requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trust {
    /// A program run with the user's permissions: every request is answered, as before.
    Full,
    /// A module in the sandbox: read-only, below the browsed folder. Only a build with the `wasm`
    /// feature starts one.
    #[cfg_attr(not(feature = "wasm"), allow(dead_code))]
    Sandboxed,
}

/// Why a sandboxed plugin's request was not answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The request would change something, or run something that could.
    Changes(&'static str),
    /// The path is not inside the folder being browsed.
    Outside(PathBuf),
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Changes(what) => write!(
                f,
                "a sandboxed plugin may not {what}; it can only read below the browsed folder"
            ),
            Self::Outside(path) => write!(
                f,
                "{} is outside the browsed folder, which is all a sandboxed plugin may read",
                path.display()
            ),
        }
    }
}

impl Trust {
    /// Whether `request` may be answered for a plugin with this trust, given the folder being
    /// browsed.
    ///
    /// # Errors
    ///
    /// [`Refused`] when the plugin is sandboxed and the request changes files or reaches outside
    /// `browsed`.
    pub(crate) fn check(self, request: &PluginRequest, browsed: &Path) -> Result<(), Refused> {
        if self == Self::Full {
            return Ok(());
        }
        match request {
            PluginRequest::GetState
            | PluginRequest::RegisterCommand { .. }
            | PluginRequest::UnregisterCommand { .. }
            | PluginRequest::ListActions
            | PluginRequest::ShowPicker(_)
            | PluginRequest::Ask(_) => Ok(()),
            PluginRequest::ReadDir { path } => within(path, browsed),
            PluginRequest::Walk(walk) => within(&walk.root, browsed),
            PluginRequest::Tally(tally) => within(&tally.root, browsed),
            PluginRequest::ReadFile(read) => within(&read.path, browsed),
            PluginRequest::Stat(stat) => within(&stat.path, browsed),
            PluginRequest::RunAction { .. } => Err(Refused::Changes("run the interface's actions")),
            PluginRequest::Copy { .. } => Err(Refused::Changes("copy files")),
            PluginRequest::Move { .. } => Err(Refused::Changes("move files")),
            PluginRequest::Delete { .. } => Err(Refused::Changes("delete files")),
            PluginRequest::CreateDir { .. } => Err(Refused::Changes("create folders")),
            PluginRequest::CreateFile { .. } => Err(Refused::Changes("create files")),
            PluginRequest::Touch { .. } => Err(Refused::Changes("touch files")),
            PluginRequest::Rename { .. } => Err(Refused::Changes("rename files")),
        }
    }
}

/// Whether `path` is `browsed` or something below it.
fn within(path: &Path, browsed: &Path) -> Result<(), Refused> {
    // `starts_with("")` is true for every relative path, so a folder not yet published must not
    // be read as "anything goes".
    if browsed.as_os_str().is_empty() {
        return Err(Refused::Outside(path.to_path_buf()));
    }
    let inside = match (path.canonicalize(), browsed.canonicalize()) {
        (Ok(path), Ok(browsed)) => path.starts_with(browsed),
        // A path that does not exist (yet), or one on a backend with no local form (an `ssh://`
        // folder), cannot be resolved; it is judged as written, and `..` never passes.
        _ => lexically_within(path, browsed),
    };
    if inside {
        Ok(())
    } else {
        Err(Refused::Outside(path.to_path_buf()))
    }
}

fn lexically_within(path: &Path, browsed: &Path) -> bool {
    !path
        .components()
        .any(|component| component == Component::ParentDir)
        && path.starts_with(browsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{PluginRequest, WalkRequest};
    use file_ops::ConflictPolicy;
    use proptest::prelude::*;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    /// A fresh folder with `browsed/a/` (a file inside), `outside/secret`, and `browsed/link`
    /// pointing at `outside`.
    fn world(label: &str) -> (PathBuf, PathBuf) {
        let base =
            std::env::temp_dir().join(format!("mman-sandbox-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let browsed = base.join("browsed");
        std::fs::create_dir_all(browsed.join("a")).expect("make the browsed folder");
        std::fs::write(browsed.join("a").join("f"), b"x").expect("make a file");
        std::fs::create_dir_all(base.join("outside")).expect("make the outside folder");
        std::fs::write(base.join("outside").join("secret"), b"s").expect("make a secret");
        symlink(base.join("outside"), browsed.join("link")).expect("make the escaping link");
        (base, browsed)
    }

    fn read_dir(path: &Path) -> PluginRequest {
        PluginRequest::ReadDir {
            path: path.to_path_buf(),
        }
    }

    #[test]
    fn a_program_is_trusted_with_everything() {
        let (base, browsed) = world("full");
        let outside = read_dir(&base.join("outside"));
        assert_eq!(Trust::Full.check(&outside, &browsed), Ok(()));
        let delete = PluginRequest::Delete {
            path: base.join("outside"),
        };
        assert_eq!(Trust::Full.check(&delete, &browsed), Ok(()));
    }

    #[test]
    fn a_module_may_read_below_the_browsed_folder() {
        let (_base, browsed) = world("below");
        for path in [
            browsed.clone(),
            browsed.join("a"),
            browsed.join("a").join("f"),
        ] {
            assert_eq!(
                Trust::Sandboxed.check(&read_dir(&path), &browsed),
                Ok(()),
                "{path:?}"
            );
        }
        let walk = PluginRequest::Walk(WalkRequest {
            walk: 1,
            root: browsed.clone(),
            show_hidden: false,
            metadata: false,
        });
        assert_eq!(Trust::Sandboxed.check(&walk, &browsed), Ok(()));
    }

    #[test]
    fn a_module_may_not_read_beside_or_above_the_browsed_folder() {
        let (base, browsed) = world("beside");
        for path in [
            base.clone(),
            base.join("outside"),
            base.join("outside").join("secret"),
        ] {
            assert_eq!(
                Trust::Sandboxed.check(&read_dir(&path), &browsed),
                Err(Refused::Outside(path.clone())),
                "{path:?}"
            );
        }
    }

    #[test]
    fn dot_dot_cannot_climb_out() {
        let (base, browsed) = world("dotdot");
        let climbing = browsed
            .join("a")
            .join("..")
            .join("..")
            .join("outside")
            .join("secret");
        assert!(
            Trust::Sandboxed
                .check(&read_dir(&climbing), &browsed)
                .is_err()
        );
        // Also when the target does not exist, where only the lexical rule can judge it.
        let missing = browsed.join("..").join("nothing-here");
        assert!(
            Trust::Sandboxed
                .check(&read_dir(&missing), &browsed)
                .is_err()
        );
        assert!(base.exists());
    }

    #[test]
    fn a_symlink_inside_the_folder_cannot_lead_out_of_it() {
        let (_base, browsed) = world("link");
        for path in [browsed.join("link"), browsed.join("link").join("secret")] {
            assert_eq!(
                Trust::Sandboxed.check(&read_dir(&path), &browsed),
                Err(Refused::Outside(path.clone())),
                "{path:?}"
            );
        }
    }

    #[test]
    fn nothing_is_readable_before_a_folder_is_published() {
        let request = read_dir(Path::new("/etc/passwd"));
        assert!(Trust::Sandboxed.check(&request, Path::new("")).is_err());
        assert!(
            Trust::Sandboxed
                .check(&read_dir(Path::new("rel")), Path::new(""))
                .is_err()
        );
    }

    #[test]
    fn every_request_that_changes_a_file_is_refused() {
        let (_base, browsed) = world("writes");
        let inside = browsed.join("a").join("f");
        let changing = [
            PluginRequest::Copy {
                src: inside.clone(),
                dst: browsed.join("g"),
                policy: ConflictPolicy::Abort,
            },
            PluginRequest::Move {
                src: inside.clone(),
                dst: browsed.join("g"),
                policy: ConflictPolicy::Abort,
            },
            PluginRequest::Delete {
                path: inside.clone(),
            },
            PluginRequest::CreateDir {
                path: browsed.join("d"),
            },
            PluginRequest::CreateFile {
                path: browsed.join("n"),
            },
            PluginRequest::Touch {
                path: inside.clone(),
            },
            PluginRequest::Rename {
                path: inside,
                new_name: "h".into(),
                policy: ConflictPolicy::Abort,
            },
            PluginRequest::RunAction {
                name: "delete".into(),
            },
        ];
        for request in changing {
            assert!(
                matches!(
                    Trust::Sandboxed.check(&request, &browsed),
                    Err(Refused::Changes(_))
                ),
                "{request:?}"
            );
        }
    }

    #[test]
    fn talking_to_the_interface_is_allowed() {
        let (_base, browsed) = world("talk");
        for request in [
            PluginRequest::GetState,
            PluginRequest::ListActions,
            PluginRequest::RegisterCommand { name: "x".into() },
            PluginRequest::UnregisterCommand { name: "x".into() },
        ] {
            assert_eq!(
                Trust::Sandboxed.check(&request, &browsed),
                Ok(()),
                "{request:?}"
            );
        }
    }

    #[test]
    fn the_reason_names_what_was_refused() {
        assert!(
            Refused::Changes("delete files")
                .to_string()
                .contains("delete files")
        );
        assert!(Refused::Outside("/etc".into()).to_string().contains("/etc"));
    }

    proptest! {
        // Safety, not completeness: whatever mix of folders, links, `..` and missing names is
        // asked for, a path that really resolves outside the folder is never let through, and a
        // missing one is never let through while it contains `..`.
        #[test]
        fn nothing_that_resolves_outside_is_ever_allowed(
            parts in proptest::collection::vec(
                prop_oneof![Just("a"), Just("link"), Just(".."), Just("."), Just("zz"), Just("secret")],
                0..8,
            )
        ) {
            let (base, browsed) = world("prop");
            let mut path = browsed.clone();
            for part in &parts {
                path.push(part);
            }
            let verdict = Trust::Sandboxed.check(&read_dir(&path), &browsed);
            match path.canonicalize() {
                Ok(real) => {
                    let root = browsed.canonicalize().expect("the browsed folder exists");
                    if !real.starts_with(&root) {
                        prop_assert!(verdict.is_err(), "{path:?} resolves to {real:?}");
                    }
                }
                Err(_) => {
                    if parts.contains(&"..") {
                        prop_assert!(verdict.is_err(), "{path:?} is missing and climbs");
                    }
                }
            }
            let _ = std::fs::remove_dir_all(base);
        }
    }
}
