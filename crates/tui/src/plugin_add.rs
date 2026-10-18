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

//! `mman plugin add <folder>`: puts a plugin folder with a `plugin.toml` into the drop-in directory,
//! where the plugin menu lists it (switched off) at the next start. The folder is held to what
//! `mman plugin check` holds a manifest to (its API version, its name, its keys and `:` words
//! against the interface and the other plugins) *before* anything is copied, and then it is
//! copied, not linked, so the plugin keeps working if the original moves. Only a local folder: a
//! plugin fetched from the network would run with the user's permissions and nothing here could
//! vouch for it.
//!
//! The copy is bounded, because the folder is somebody else's: it follows no symbolic link, takes
//! no special file, leaves `.git` behind, and stops at 5,000 files, 64 MiB or 16 levels. It is made
//! under a temporary name and renamed into place, so the menu never lists half a plugin.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use theming::{Config, DropIn};

use crate::plugin_check;
use crate::plugin_new::{NameError, PluginName};
use crate::plugin_roster;

const MAX_FILES: usize = 5_000;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DEPTH: usize = 16;

/// Why a plugin was not added.
#[derive(Debug)]
pub enum AddError {
    NoConfigDir,
    /// The folder's manifest is missing or not accepted.
    Manifest(String),
    /// A version, name, key or `:` word the plugin cannot have.
    Refused(String),
    BadName(NameError),
    /// The folder is already inside the plugins folder.
    AlreadyThere(PathBuf),
    Exists(PathBuf),
    TooMuch(&'static str),
    NotCopyable(PathBuf),
    Io(PathBuf, io::Error),
}

impl fmt::Display for AddError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoConfigDir => write!(f, "could not find the config folder to add the plugin to"),
            Self::Manifest(why) | Self::Refused(why) => write!(f, "{why}"),
            Self::BadName(e) => write!(f, "{e}"),
            Self::AlreadyThere(dir) => {
                write!(f, "{} is already in the plugins folder", dir.display())
            }
            Self::Exists(dir) => write!(f, "{} already exists; nothing was changed", dir.display()),
            Self::TooMuch(what) => {
                write!(f, "the folder holds more than {what}; nothing was added")
            }
            Self::NotCopyable(path) => write!(
                f,
                "{} is a link or a special file, which is not copied; nothing was added",
                path.display()
            ),
            Self::Io(path, e) => write!(f, "{}: {e}", path.display()),
        }
    }
}

impl std::error::Error for AddError {}

/// Checks the plugin in `src` against `config` and copies it into the plugins folder `root`.
/// Returns its name and where it now is.
///
/// # Errors
///
/// An [`AddError`] naming why the manifest, the name, the keys, the folder or a write was not
/// acceptable; in every case `root` is left as it was.
pub fn add(src: &Path, root: &Path, config: &Config) -> Result<(String, PathBuf), AddError> {
    let found =
        DropIn::load(src).map_err(|e| AddError::Manifest(format!("{}: {e}", src.display())))?;
    let name = found.spec.name.clone();
    if let Some(why) = plugin_roster::api_mismatch(found.api_version) {
        return Err(AddError::Refused(format!("{name}: {why}")));
    }
    if let (Ok(from), Ok(inside)) = (src.canonicalize(), root.canonicalize())
        && from.starts_with(&inside)
    {
        return Err(AddError::AlreadyThere(from));
    }
    let roster = plugin_check::roster(config);
    if let Some(why) = plugin_roster::conflict(&roster, &found.spec, &config.keys) {
        return Err(AddError::Refused(format!("{name}: {why}")));
    }
    let dir = install(&found, root)?;
    Ok((name, dir))
}

/// Copies the plugin folder into `<root>/<name>/`.
fn install(found: &DropIn, root: &Path) -> Result<PathBuf, AddError> {
    let name = PluginName::try_new(&found.spec.name).map_err(AddError::BadName)?;
    let dest = root.join(name.as_str());
    if dest.exists() {
        return Err(AddError::Exists(dest));
    }
    fs::create_dir_all(root).map_err(|e| AddError::Io(root.to_path_buf(), e))?;
    let staging = root.join(format!(".adding-{}-{}", name.as_str(), std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    let mut budget = Budget::default();
    let result = copy_dir(&found.dir, &staging, 0, &mut budget)
        .and_then(|()| fs::rename(&staging, &dest).map_err(|e| AddError::Io(dest.clone(), e)));
    match result {
        Ok(()) => Ok(dest),
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

#[derive(Default)]
struct Budget {
    files: usize,
    bytes: u64,
}

fn copy_dir(from: &Path, to: &Path, depth: usize, budget: &mut Budget) -> Result<(), AddError> {
    if depth > MAX_DEPTH {
        return Err(AddError::TooMuch("16 levels"));
    }
    fs::create_dir(to).map_err(|e| AddError::Io(to.to_path_buf(), e))?;
    let entries = fs::read_dir(from).map_err(|e| AddError::Io(from.to_path_buf(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| AddError::Io(from.to_path_buf(), e))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|e| AddError::Io(path.clone(), e))?;
        if kind.is_dir() {
            if entry.file_name() != ".git" {
                copy_dir(&path, &to.join(entry.file_name()), depth + 1, budget)?;
            }
        } else if kind.is_file() {
            budget.files += 1;
            if budget.files > MAX_FILES {
                return Err(AddError::TooMuch("5,000 files"));
            }
            let size = entry
                .metadata()
                .map_err(|e| AddError::Io(path.clone(), e))?
                .len();
            budget.bytes = budget.bytes.saturating_add(size);
            if budget.bytes > MAX_BYTES {
                return Err(AddError::TooMuch("64 MiB"));
            }
            // `fs::copy` keeps the permission bits, so a program stays executable.
            fs::copy(&path, to.join(entry.file_name())).map_err(|e| AddError::Io(path, e))?;
        } else {
            return Err(AddError::NotCopyable(path));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_new::{Lang, create};
    use proptest::prelude::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A folder of its own under the system's temporary one, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "minuteman-plugin-add-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).expect("create scratch folder");
            Self(dir)
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }

        /// A plugin folder named `folder` whose manifest is `manifest`.
        fn plugin(&self, folder: &str, manifest: &str) -> PathBuf {
            let dir = self.path(folder);
            fs::create_dir_all(&dir).expect("create plugin folder");
            fs::write(dir.join("plugin.toml"), manifest).expect("write manifest");
            dir
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn config() -> Config {
        Config::from_sources(None, None, None)
    }

    #[test]
    fn a_plugin_made_by_new_is_added_whole_and_stays_executable() {
        let scratch = Scratch::new();
        let name = PluginName::try_new("fresh").unwrap();
        let src = create(&scratch.path("work"), &name, Lang::Sh).unwrap();
        fs::create_dir(src.join("lib")).unwrap();
        fs::write(src.join("lib/data.txt"), "data").unwrap();
        fs::create_dir(src.join(".git")).unwrap();
        fs::write(src.join(".git/config"), "x").unwrap();

        let root = scratch.path("plugins");
        let (added, dir) = add(&src, &root, &config()).expect("added");
        assert_eq!(added, "fresh");
        assert_eq!(dir, root.join("fresh"));
        assert_eq!(
            fs::read_to_string(dir.join("lib/data.txt")).unwrap(),
            "data"
        );
        assert!(!dir.join(".git").exists(), ".git stays behind");
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dir.join("plugin.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
        let loaded = DropIn::load(&dir).expect("the copy loads on its own");
        assert!(loaded.spec.command.starts_with(&*dir.to_string_lossy()));
        assert!(src.join("plugin.sh").is_file(), "the original is kept");
        let leftovers: Vec<_> = fs::read_dir(&root).unwrap().flatten().collect();
        assert_eq!(leftovers.len(), 1, "no staging folder is left");
    }

    #[test]
    fn a_bad_manifest_a_foreign_api_version_and_a_bad_name_add_nothing() {
        let scratch = Scratch::new();
        let root = scratch.path("plugins");
        let bad = scratch.plugin("bad", "command = ");
        assert!(matches!(
            add(&bad, &root, &config()),
            Err(AddError::Manifest(_))
        ));
        let old = scratch.plugin("old", "command = 'sh'\napi_version = 999");
        let why = add(&old, &root, &config()).unwrap_err().to_string();
        assert!(why.contains("written for plugin API 999"), "{why}");
        let odd = scratch.plugin("odd", "name = '..'\ncommand = 'sh'");
        assert!(matches!(
            add(&odd, &root, &config()),
            Err(AddError::BadName(_))
        ));
        assert!(!root.exists() || fs::read_dir(&root).unwrap().next().is_none());
    }

    #[test]
    fn a_key_the_interface_binds_is_refused_before_copying() {
        let scratch = Scratch::new();
        let root = scratch.path("plugins");
        let greedy = scratch.plugin("greedy", "command = 'sh'\non_key = 'j'");
        let why = add(&greedy, &root, &config()).unwrap_err().to_string();
        assert!(why.contains("greedy: key j is already bound"), "{why}");
        assert!(!root.exists());
    }

    #[test]
    fn a_plugin_already_there_or_of_a_taken_name_is_refused_and_untouched() {
        let scratch = Scratch::new();
        let root = scratch.path("plugins");
        let src = scratch.plugin("one", "command = 'sh'");
        let (_, dir) = add(&src, &root, &config()).unwrap();
        fs::write(dir.join("mine.txt"), "mine").unwrap();
        assert!(matches!(
            add(&src, &root, &config()),
            Err(AddError::Exists(_))
        ));
        assert!(matches!(
            add(&dir, &root, &config()),
            Err(AddError::AlreadyThere(_))
        ));
        assert_eq!(fs::read_to_string(dir.join("mine.txt")).unwrap(), "mine");
    }

    #[test]
    fn a_link_is_refused_and_the_staging_folder_removed() {
        let scratch = Scratch::new();
        let root = scratch.path("plugins");
        let src = scratch.plugin("linky", "command = 'sh'");
        std::os::unix::fs::symlink("/etc/passwd", src.join("secret")).unwrap();
        assert!(matches!(
            add(&src, &root, &config()),
            Err(AddError::NotCopyable(_))
        ));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }

    #[test]
    fn a_folder_over_the_limits_is_refused() {
        let scratch = Scratch::new();
        let root = scratch.path("plugins");
        let src = scratch.plugin("deep", "command = 'sh'");
        let mut deep = src.clone();
        for level in 0..=MAX_DEPTH {
            deep = deep.join(format!("d{level}"));
        }
        fs::create_dir_all(&deep).unwrap();
        assert!(matches!(
            add(&src, &root, &config()),
            Err(AddError::TooMuch(_))
        ));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }

    proptest! {
        /// Whatever files a folder holds, the copy has the same names and contents.
        #[test]
        fn the_copy_has_every_file_with_its_contents(
            files in proptest::collection::btree_map("[a-z]{1,8}", proptest::collection::vec(any::<u8>(), 0..64), 0..8)
        ) {
            let scratch = Scratch::new();
            let src = scratch.plugin("p", "command = 'sh'");
            for (file, bytes) in &files {
                fs::write(src.join(format!("f-{file}")), bytes).unwrap();
            }
            let (_, dir) = add(&src, &scratch.path("plugins"), &config()).unwrap();
            for (file, bytes) in &files {
                prop_assert_eq!(&fs::read(dir.join(format!("f-{file}"))).unwrap(), bytes);
            }
            prop_assert_eq!(fs::read_dir(&dir).unwrap().count(), files.len() + 1);
        }
    }
}
