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

//! Where the plugin menu's "Found" list comes from: the browsed folder itself and each folder
//! directly inside it that holds a `plugin.toml`. One level only, so a folder full of repositories
//! cannot make opening the list slow, and a folder whose manifest is not accepted is passed over
//! (choosing it with the folder picker says why).

use std::fs;
use std::path::{Path, PathBuf};

use theming::DropIn;
use theming::dropin::MANIFEST;

/// The most subfolders of one folder looked into.
const MAX_FOLDERS: usize = 500;

/// A plugin that could be added, and the reason it cannot be yet, if there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub found: DropIn,
    /// Something already has its name, a key or a `:` word. The review card shows it and refuses
    /// the add, so the user learns before the plugin is started.
    pub issue: Option<String>,
}

/// The plugin folders at `root`: `root` itself if it has a manifest, then each folder inside it
/// that has one, in name order, without those in `skip` (folders the menu already lists).
pub fn find(root: &Path, skip: &[PathBuf]) -> Vec<DropIn> {
    let mut folders: Vec<PathBuf> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .take(MAX_FOLDERS)
        .map(|entry| entry.path())
        .filter(|path| path.join(MANIFEST).is_file())
        .collect();
    folders.sort();
    if root.join(MANIFEST).is_file() {
        folders.insert(0, root.to_path_buf());
    }
    folders
        .into_iter()
        .filter(|dir| !skip.contains(dir))
        .filter_map(|dir| DropIn::load(&dir).ok())
        .collect()
}

/// What `Enter` means on a row while the browser is choosing a plugin folder.
#[derive(Debug, PartialEq, Eq)]
pub enum Choice {
    /// A folder with no manifest: go into it like any folder.
    Enter,
    /// Not something to choose, and not something to open either: a file.
    NotAFolder,
    /// A folder with an accepted manifest.
    Chosen(DropIn),
    /// A folder whose manifest was not accepted, and why.
    Refused(String),
}

/// Decides what `Enter` does on the row at `path` (`is_dir` as the listing says).
pub fn choose(path: &Path, is_dir: bool) -> Choice {
    if !is_dir {
        return Choice::NotAFolder;
    }
    if !path.join(MANIFEST).is_file() {
        return Choice::Enter;
    }
    match DropIn::load(path) {
        Ok(found) => Choice::Chosen(found),
        Err(why) => Choice::Refused(format!("{}: {why}", path.display())),
    }
}

/// Decides what `Q` chooses while the browser is choosing a plugin folder: the folder under the
/// cursor, or else the one being browsed. Unlike `Enter` it never goes into a folder, so a folder
/// with no manifest is refused with the reason.
pub fn choose_here(cursor: Option<(&Path, bool)>, dir: &Path) -> Choice {
    let target = match cursor {
        Some((path, true)) => path,
        _ => dir,
    };
    match DropIn::load(target) {
        Ok(found) => Choice::Chosen(found),
        Err(why) => Choice::Refused(format!("{}: {why}", target.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A fresh empty folder under the system temp dir, deleted when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "minuteman-plugin-source-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).expect("create scratch folder");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn plugin(dir: &Path, name: &str) {
        fs::create_dir_all(dir).expect("create plugin folder");
        fs::write(
            dir.join(MANIFEST),
            format!("name = \"{name}\"\ncommand = \"true\"\n"),
        )
        .expect("write manifest");
    }

    fn names(found: &[DropIn]) -> Vec<&str> {
        found.iter().map(|f| f.spec.name.as_str()).collect()
    }

    #[test]
    fn the_folder_itself_comes_first_then_its_subfolders_in_name_order() {
        let tmp = Scratch::new();
        plugin(tmp.path(), "here");
        plugin(&tmp.path().join("zeta"), "zeta");
        plugin(&tmp.path().join("alpha"), "alpha");
        fs::create_dir(tmp.path().join("plain")).expect("create plain folder");
        assert_eq!(
            names(&find(tmp.path(), &[])),
            ["here", "alpha", "zeta"],
            "a folder without a manifest is not a plugin"
        );
    }

    #[test]
    fn only_one_level_down_is_looked_at() {
        let tmp = Scratch::new();
        plugin(&tmp.path().join("a").join("deep"), "deep");
        assert!(find(tmp.path(), &[]).is_empty());
    }

    #[test]
    fn a_listed_folder_and_a_bad_manifest_are_left_out() {
        let tmp = Scratch::new();
        plugin(&tmp.path().join("kept"), "kept");
        plugin(&tmp.path().join("listed"), "listed");
        fs::create_dir(tmp.path().join("broken")).expect("create broken folder");
        fs::write(tmp.path().join("broken").join(MANIFEST), "name = [").expect("write manifest");
        let skip = [tmp.path().join("listed")];
        assert_eq!(names(&find(tmp.path(), &skip)), ["kept"]);
    }

    #[test]
    fn enter_chooses_a_plugin_folder_enters_any_other_and_refuses_a_file() {
        let tmp = Scratch::new();
        plugin(&tmp.path().join("good"), "good");
        fs::create_dir(tmp.path().join("plain")).expect("create plain folder");
        fs::create_dir(tmp.path().join("bad")).expect("create bad folder");
        fs::write(tmp.path().join("bad").join(MANIFEST), "name = [").expect("write manifest");
        assert!(matches!(
            choose(&tmp.path().join("good"), true),
            Choice::Chosen(found) if found.spec.name == "good"
        ));
        assert_eq!(choose(&tmp.path().join("plain"), true), Choice::Enter);
        assert!(matches!(
            choose(&tmp.path().join("bad"), true),
            Choice::Refused(why) if why.contains("bad")
        ));
        assert_eq!(choose(&tmp.path().join("file"), false), Choice::NotAFolder);
    }

    #[test]
    fn q_chooses_the_folder_under_the_cursor_or_else_the_one_browsed() {
        let tmp = Scratch::new();
        plugin(&tmp.path().join("good"), "good");
        fs::create_dir(tmp.path().join("plain")).expect("create plain folder");
        let good = tmp.path().join("good");
        let plain = tmp.path().join("plain");
        assert!(matches!(
            choose_here(Some((&good, true)), tmp.path()),
            Choice::Chosen(found) if found.spec.name == "good"
        ));
        assert!(matches!(
            choose_here(Some((&plain, true)), tmp.path()),
            Choice::Refused(why) if why.contains("plain")
        ));
        let file = good.join("notes");
        assert!(matches!(
            choose_here(Some((&file, false)), &good),
            Choice::Chosen(found) if found.spec.name == "good"
        ));
        assert!(matches!(choose_here(None, &plain), Choice::Refused(_)));
    }

    #[test]
    fn a_missing_folder_finds_nothing() {
        assert!(find(Path::new("/nonexistent/minuteman-test"), &[]).is_empty());
    }

    proptest! {
        #[test]
        fn every_plugin_folder_is_found_exactly_once(
            folders in proptest::collection::btree_set("[a-z]{1,8}", 0..8),
        ) {
            let tmp = Scratch::new();
            for name in &folders {
                plugin(&tmp.path().join(name), name);
            }
            let found = find(tmp.path(), &[]);
            let got: Vec<&str> = names(&found);
            let want: Vec<&str> = folders.iter().map(String::as_str).collect();
            prop_assert_eq!(got, want);
        }
    }
}
