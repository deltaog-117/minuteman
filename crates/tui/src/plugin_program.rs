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

//! The program file behind a bundled plugin, and the rules for deleting it from the plugin menu.
//!
//! Deleting is the one destructive thing the menu does, so it is narrow: only a plain file (a
//! link is left for the user to remove by hand) that the running user owns, so a program the
//! system or a package manager put in `/usr/bin` is never touched. The question names the file,
//! its size and the command that installs it again.

use std::ffi::OsStr;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use crate::hud::format_size;

/// A bundled plugin's program, checked and ready to be deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub path: PathBuf,
    pub size: u64,
    /// The command that puts it back.
    pub reinstall: String,
}

impl Program {
    /// The lines the confirmation shows: what is deleted, how big it is, how to get it back and
    /// which keys answer.
    pub fn question(&self, name: &str) -> Vec<String> {
        vec![
            format!("Delete the program of plugin {name}?"),
            format!("{} ({})", self.path.display(), format_size(self.size)),
            format!("To get it back: {}", self.reinstall),
            "y deletes it, n or Esc keeps it".to_string(),
        ]
    }

    /// Deletes the file.
    pub fn remove(&self) -> std::io::Result<()> {
        std::fs::remove_file(&self.path)
    }
}

/// What the decision depends on, read from the file system apart from the decision itself.
#[derive(Debug, Clone, Copy)]
struct Facts {
    regular: bool,
    owner: u32,
    size: u64,
}

/// Why a program may not be deleted, or `Ok` with its size when it may.
fn judge(facts: Facts, me: u32) -> Result<u64, &'static str> {
    if !facts.regular {
        return Err("its program is not a plain file (a link, say), so remove it by hand");
    }
    if facts.owner != me {
        return Err("its program belongs to another user, so it is not deleted from here");
    }
    Ok(facts.size)
}

/// The program `command` names, checked for deletion. `name` is the bundled plugin it belongs to.
pub fn inspect(name: &str, command: &str) -> Result<Program, String> {
    inspect_as(
        name,
        command,
        std::env::var_os("PATH").as_deref(),
        current_user(),
    )
}

fn inspect_as(
    name: &str,
    command: &str,
    search_path: Option<&OsStr>,
    me: u32,
) -> Result<Program, String> {
    let path = resolve(command, search_path)
        .ok_or_else(|| format!("{command} was not found, so there is nothing to delete"))?;
    let meta = std::fs::symlink_metadata(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let facts = Facts {
        regular: meta.file_type().is_file(),
        owner: meta.uid(),
        size: meta.len(),
    };
    let size = judge(facts, me).map_err(|why| format!("{}: {why}", path.display()))?;
    Ok(Program {
        path,
        size,
        reinstall: format!("cargo install --path plugins/{name}"),
    })
}

/// Where `command` is: itself when it holds a `/`, otherwise the first folder of `search_path`
/// that has a file of that name.
fn resolve(command: &str, search_path: Option<&OsStr>) -> Option<PathBuf> {
    if command.contains('/') {
        return Some(PathBuf::from(command));
    }
    std::env::split_paths(search_path?)
        .map(|dir| dir.join(command))
        .find(|candidate| candidate.is_file())
}

/// The user this program runs as.
fn current_user() -> u32 {
    // SAFETY: `geteuid` takes no arguments, cannot fail and touches no memory.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::path::Path;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mman-program-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_plain_file_of_mine_may_go() {
        let facts = Facts {
            regular: true,
            owner: 1000,
            size: 42,
        };
        assert_eq!(judge(facts, 1000), Ok(42));
    }

    #[test]
    fn a_file_of_another_user_or_a_link_may_not() {
        let mine = Facts {
            regular: true,
            owner: 0,
            size: 1,
        };
        assert!(judge(mine, 1000).is_err());
        let link = Facts {
            regular: false,
            owner: 1000,
            size: 1,
        };
        assert!(judge(link, 1000).is_err());
    }

    #[test]
    fn a_name_is_searched_for_on_the_path_and_a_slash_is_taken_as_it_is() {
        let dir = scratch("resolve");
        std::fs::write(dir.join("mman-x"), b"x").unwrap();
        let paths = std::env::join_paths([Path::new("/nonexistent"), dir.as_path()]).unwrap();
        assert_eq!(resolve("mman-x", Some(&paths)), Some(dir.join("mman-x")));
        assert_eq!(resolve("mman-y", Some(&paths)), None);
        assert_eq!(resolve("mman-x", None), None);
        assert_eq!(
            resolve("/opt/bin/mman-x", None),
            Some("/opt/bin/mman-x".into())
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn inspecting_and_removing_a_file_of_mine_names_it_and_deletes_only_it() {
        let dir = scratch("remove");
        let file = dir.join("mman-finder");
        let other = dir.join("mman-git");
        std::fs::write(&file, vec![0u8; 2048]).unwrap();
        std::fs::write(&other, b"keep").unwrap();
        let me = current_user();
        let program = inspect_as("finder", file.to_str().unwrap(), None, me).unwrap();
        assert_eq!(program.size, 2048);
        assert_eq!(program.reinstall, "cargo install --path plugins/finder");
        let lines = program.question("finder").join("\n");
        assert!(lines.contains(file.to_str().unwrap()));
        assert!(lines.contains("cargo install --path plugins/finder"));
        program.remove().unwrap();
        assert!(!file.exists() && other.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_owned_by_someone_else_is_refused_and_stays() {
        let dir = scratch("refuse");
        let file = dir.join("mman-hud");
        std::fs::write(&file, b"x").unwrap();
        let someone_else = current_user().wrapping_add(1);
        let refused = inspect_as("hud", file.to_str().unwrap(), None, someone_else);
        assert!(refused.is_err());
        assert!(file.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_link_is_refused_even_when_it_points_at_my_file() {
        let dir = scratch("link");
        let target = dir.join("real");
        let link = dir.join("mman-ssh");
        std::fs::write(&target, b"x").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let refused = inspect_as("ssh", link.to_str().unwrap(), None, current_user());
        assert!(refused.is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    proptest! {
        #[test]
        fn only_a_regular_file_of_the_running_user_is_judged_fit(
            regular in any::<bool>(), owner in 0u32..4, me in 0u32..4, size in any::<u64>(),
        ) {
            let verdict = judge(Facts { regular, owner, size }, me);
            prop_assert_eq!(verdict.is_ok(), regular && owner == me);
        }
    }
}
