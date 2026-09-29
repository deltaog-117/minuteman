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

//! Which installed programs open which kind of file, worked out the way the desktop does it:
//! the user's `mimeapps.list` says what they chose (default, added, removed), each data
//! directory's `mimeinfo.cache` says what is installed, and the `.desktop` files say how to run
//! it. Everything is read in-process, so the menu costs no spawned program and works in a
//! terminal-only session.
//!
//! Precedence, for a type and then each type it is a kind of: the user's default, their added
//! associations, then whatever the installed programs registered; a removed association is
//! never offered. A program that is not actually installed is skipped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use theming::OpenWith;

use crate::desktop_entry::{self, DesktopEntry};
use crate::mime_type::{MimeDb, XdgDirs};
use crate::open;

/// The most `-` → `/` substitutions tried when looking for a desktop file ID's file.
const MAX_ID_DASHES: usize = 8;

#[derive(Debug, Default)]
pub struct Associations {
    xdg: XdgDirs,
    mime: MimeDb,
    defaults: HashMap<String, Vec<String>>,
    added: HashMap<String, Vec<String>>,
    removed: HashMap<String, Vec<String>>,
    cache: HashMap<String, Vec<String>>,
}

/// `key=a;b;c` lines of one `[group]` as `(key, [a, b, c])`.
fn group_lists<'a>(text: &'a str, group: &str) -> Vec<(&'a str, Vec<&'a str>)> {
    let mut inside = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            inside = name == group;
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            out.push((
                key.trim(),
                value
                    .split(';')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .collect(),
            ));
        }
    }
    out
}

fn append(map: &mut HashMap<String, Vec<String>>, text: &str, group: &str) {
    for (mime, ids) in group_lists(text, group) {
        map.entry(mime.to_string())
            .or_default()
            .extend(ids.into_iter().map(str::to_string));
    }
}

impl Associations {
    /// Reads the databases the environment points at.
    pub fn from_env() -> Self {
        Self::load(XdgDirs::from_env())
    }

    /// Reads `mimeapps.list` (highest priority first, desktop-specific before general in each
    /// directory), every `mimeinfo.cache` and the MIME database from `xdg`. Anything missing or
    /// unreadable contributes nothing.
    pub fn load(xdg: XdgDirs) -> Self {
        let mut this = Self {
            mime: MimeDb::load(&xdg),
            ..Self::default()
        };
        let mut lists: Vec<PathBuf> = Vec::new();
        let dirs = std::iter::once(xdg.config_home.clone())
            .chain(xdg.config_dirs.iter().cloned())
            .chain(xdg.all_data_dirs().map(|d| d.join("applications")));
        for dir in dirs {
            for desktop in &xdg.desktops {
                lists.push(dir.join(format!("{desktop}-mimeapps.list")));
            }
            lists.push(dir.join("mimeapps.list"));
        }
        for path in lists {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            append(&mut this.defaults, &text, "Default Applications");
            append(&mut this.added, &text, "Added Associations");
            append(&mut this.removed, &text, "Removed Associations");
        }
        for data in xdg.all_data_dirs() {
            if let Ok(text) = std::fs::read_to_string(data.join("applications/mimeinfo.cache")) {
                append(&mut this.cache, &text, "MIME Cache");
            }
        }
        this.xdg = xdg;
        this
    }

    /// The type of the file at `path`.
    pub fn mime_of(&self, path: &Path) -> String {
        self.mime.mime_of_file(path)
    }

    /// Desktop file IDs that may open `mime`, best first, without repeats.
    fn ids_for(&self, mime: &str) -> Vec<String> {
        let mut ids: Vec<String> = Vec::new();
        for kind in self.mime.lineage(mime) {
            let removed = self.removed.get(&kind);
            for list in [&self.defaults, &self.added, &self.cache] {
                for id in list.get(&kind).into_iter().flatten() {
                    if removed.is_some_and(|r| r.contains(id)) || ids.contains(id) {
                        continue;
                    }
                    ids.push(id.clone());
                }
            }
        }
        ids
    }

    /// The `.desktop` file an ID names, in the highest-priority data directory that has it. An ID
    /// stands for a path with `/` written as `-`, so `kde-foo.desktop` may be `kde/foo.desktop`.
    fn entry(&self, id: &str) -> Option<DesktopEntry> {
        let dashes: Vec<usize> = id.match_indices('-').map(|(i, _)| i).collect();
        let mut names = vec![id.to_string()];
        names.extend(dashes.iter().take(MAX_ID_DASHES).map(|&i| {
            let mut name = id.to_string();
            name.replace_range(i..=i, "/");
            name
        }));
        for data in self.xdg.all_data_dirs() {
            for name in &names {
                let path = data.join("applications").join(name);
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let entry = desktop_entry::parse(id, &path, &text, &self.xdg.locales)?;
                let program = entry
                    .try_exec
                    .clone()
                    .or_else(|| desktop_entry::split_exec(&entry.exec).into_iter().next())?;
                return open::program_exists(&program).then_some(entry);
            }
        }
        None
    }

    /// The program the desktop would open `mime` with: the first preferred one that is installed.
    /// A menu-hidden program (`NoDisplay`) can still be the default.
    pub fn default_for(&self, mime: &str) -> Option<DesktopEntry> {
        self.ids_for(mime).iter().find_map(|id| self.entry(id))
    }

    /// Every installed program that can open `mime`, preferred first, for a menu: ones marked
    /// `NoDisplay` are left out.
    pub fn apps_for(&self, mime: &str) -> Vec<DesktopEntry> {
        self.ids_for(mime)
            .iter()
            .filter_map(|id| self.entry(id))
            .filter(|entry| !entry.no_display)
            .collect()
    }
}

/// An entry as an "Open with" choice.
pub fn choice(entry: &DesktopEntry) -> OpenWith {
    OpenWith {
        name: entry.name.clone(),
        command: desktop_entry::command_line(entry),
        terminal: entry.terminal,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use proptest::prelude::*;

    use super::*;

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// A scratch XDG layout: `data/` (user), `sys/` (system) and `config/`.
    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "mm-assoc-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(root.join("data/applications")).unwrap();
            std::fs::create_dir_all(root.join("sys/applications")).unwrap();
            std::fs::create_dir_all(root.join("sys/mime")).unwrap();
            std::fs::create_dir_all(root.join("config")).unwrap();
            Self { root }
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }

        /// A launchable app whose program (`sh`) exists everywhere the tests run.
        fn app(&self, rel: &str, name: &str, extra: &str) {
            self.write(
                rel,
                &format!("[Desktop Entry]\nType=Application\nName={name}\nExec=sh %f\n{extra}\n"),
            );
        }

        fn xdg(&self, desktops: &[&str]) -> XdgDirs {
            XdgDirs {
                data_home: self.root.join("data"),
                data_dirs: vec![self.root.join("sys")],
                config_home: self.root.join("config"),
                config_dirs: Vec::new(),
                desktops: desktops.iter().map(|d| (*d).to_string()).collect(),
                locales: Vec::new(),
            }
        }

        fn load(&self) -> Associations {
            Associations::load(self.xdg(&[]))
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn names(apps: &[DesktopEntry]) -> Vec<&str> {
        apps.iter().map(|a| a.name.as_str()).collect()
    }

    #[test]
    fn the_desktop_default_comes_first_then_added_then_installed() {
        let f = Fixture::new();
        for (id, name) in [("a", "A"), ("b", "B"), ("c", "C"), ("d", "D")] {
            f.app(&format!("sys/applications/{id}.desktop"), name, "");
        }
        f.write(
            "sys/applications/mimeinfo.cache",
            "[MIME Cache]\ntext/plain=a.desktop;b.desktop;c.desktop;\n",
        );
        f.write(
            "config/mimeapps.list",
            "[Default Applications]\ntext/plain=c.desktop;\n\
             [Added Associations]\ntext/plain=d.desktop;\n",
        );
        let apps = f.load().apps_for("text/plain");
        assert_eq!(names(&apps), ["C", "D", "A", "B"]);
        assert_eq!(f.load().default_for("text/plain").unwrap().name, "C");
    }

    #[test]
    fn a_removed_association_is_never_offered_and_repeats_collapse() {
        let f = Fixture::new();
        for (id, name) in [("a", "A"), ("b", "B")] {
            f.app(&format!("sys/applications/{id}.desktop"), name, "");
        }
        f.write(
            "sys/applications/mimeinfo.cache",
            "[MIME Cache]\ntext/plain=a.desktop;b.desktop;a.desktop;\n",
        );
        f.write(
            "config/mimeapps.list",
            "[Removed Associations]\ntext/plain=a.desktop;\n",
        );
        assert_eq!(names(&f.load().apps_for("text/plain")), ["B"]);
    }

    #[test]
    fn parents_and_aliases_count_but_come_after_the_exact_type() {
        let f = Fixture::new();
        f.app("sys/applications/md.desktop", "Markdown", "");
        f.app("sys/applications/ed.desktop", "Editor", "");
        f.write("sys/mime/subclasses", "text/markdown text/plain\n");
        f.write("sys/mime/aliases", "text/x-markdown text/markdown\n");
        f.write(
            "sys/applications/mimeinfo.cache",
            "[MIME Cache]\ntext/plain=ed.desktop;\ntext/x-markdown=md.desktop;\n",
        );
        assert_eq!(
            names(&f.load().apps_for("text/markdown")),
            ["Markdown", "Editor"]
        );
    }

    #[test]
    fn menu_hidden_apps_are_not_listed_but_can_still_be_the_default() {
        let f = Fixture::new();
        f.app("sys/applications/quiet.desktop", "Quiet", "NoDisplay=true");
        f.app("sys/applications/loud.desktop", "Loud", "");
        f.write(
            "sys/applications/mimeinfo.cache",
            "[MIME Cache]\nimage/png=quiet.desktop;loud.desktop;\n",
        );
        let assoc = f.load();
        assert_eq!(names(&assoc.apps_for("image/png")), ["Loud"]);
        assert_eq!(assoc.default_for("image/png").unwrap().name, "Quiet");
    }

    #[test]
    fn a_program_that_is_not_installed_is_skipped() {
        let f = Fixture::new();
        f.write(
            "sys/applications/ghost.desktop",
            "[Desktop Entry]\nType=Application\nName=Ghost\nExec=minuteman-no-such-program %f\n",
        );
        f.write(
            "sys/applications/gated.desktop",
            "[Desktop Entry]\nType=Application\nName=Gated\nExec=sh %f\nTryExec=minuteman-no-such-program\n",
        );
        f.app("sys/applications/real.desktop", "Real", "");
        f.write(
            "sys/applications/mimeinfo.cache",
            "[MIME Cache]\nimage/png=ghost.desktop;gated.desktop;real.desktop;\n",
        );
        let assoc = f.load();
        assert_eq!(names(&assoc.apps_for("image/png")), ["Real"]);
        assert_eq!(assoc.default_for("image/png").unwrap().name, "Real");
    }

    #[test]
    fn a_desktop_specific_list_beats_the_general_one_and_the_user_beats_the_system() {
        let f = Fixture::new();
        for (id, name) in [("a", "A"), ("b", "B")] {
            f.app(&format!("sys/applications/{id}.desktop"), name, "");
        }
        f.write(
            "config/mimeapps.list",
            "[Default Applications]\ntext/plain=a.desktop;\n",
        );
        f.write(
            "config/kde-mimeapps.list",
            "[Default Applications]\ntext/plain=b.desktop;\n",
        );
        let with = |desktops: &[&str]| Associations::load(f.xdg(desktops));
        assert_eq!(with(&["kde"]).default_for("text/plain").unwrap().name, "B");
        assert_eq!(
            with(&["gnome"]).default_for("text/plain").unwrap().name,
            "A"
        );
        // The same ID in the user's directory shadows the system's file.
        f.app("data/applications/a.desktop", "A (mine)", "");
        assert_eq!(
            with(&[]).default_for("text/plain").unwrap().name,
            "A (mine)"
        );
    }

    #[test]
    fn an_id_with_a_dash_finds_a_file_in_a_subdirectory() {
        let f = Fixture::new();
        f.app("sys/applications/kde/foo.desktop", "Foo", "");
        f.write(
            "config/mimeapps.list",
            "[Default Applications]\ntext/plain=kde-foo.desktop;\n",
        );
        assert_eq!(f.load().default_for("text/plain").unwrap().name, "Foo");
    }

    #[test]
    fn without_any_database_nothing_is_offered_and_nothing_fails() {
        let empty = Associations::load(XdgDirs::default());
        assert!(empty.apps_for("text/plain").is_empty());
        assert!(empty.default_for("image/png").is_none());
        assert_eq!(empty.mime_of(Path::new("/x/notes.md")), "text/markdown");
    }

    #[test]
    fn a_choice_carries_the_launch_command_and_the_terminal_flag() {
        let f = Fixture::new();
        f.app("sys/applications/t.desktop", "Tool", "Terminal=true");
        f.write(
            "config/mimeapps.list",
            "[Default Applications]\ntext/plain=t.desktop;\n",
        );
        let entry = f.load().default_for("text/plain").unwrap();
        let picked = choice(&entry);
        assert_eq!(
            picked,
            OpenWith {
                name: "Tool".into(),
                command: "sh {}".into(),
                terminal: true
            }
        );
    }

    proptest! {
        /// Whatever the lists say, the result never repeats an app and never holds one the user
        /// removed, and every default the user named that is installed is offered.
        #[test]
        fn offers_have_no_repeats_and_no_removed_apps(
            defaults in proptest::collection::vec(0usize..5, 0..5),
            added in proptest::collection::vec(0usize..5, 0..5),
            cached in proptest::collection::vec(0usize..5, 0..8),
            removed in proptest::collection::vec(0usize..5, 0..3),
        ) {
            let f = Fixture::new();
            let id = |i: &usize| format!("app{i}.desktop");
            for i in 0..5 {
                f.app(&format!("sys/applications/app{i}.desktop"), &format!("App{i}"), "");
            }
            let list = |v: &[usize]| v.iter().map(id).collect::<Vec<_>>().join(";");
            f.write(
                "sys/applications/mimeinfo.cache",
                &format!("[MIME Cache]\ntext/plain={}\n", list(&cached)),
            );
            f.write(
                "config/mimeapps.list",
                &format!(
                    "[Default Applications]\ntext/plain={}\n[Added Associations]\ntext/plain={}\n\
                     [Removed Associations]\ntext/plain={}\n",
                    list(&defaults), list(&added), list(&removed)
                ),
            );
            let apps = f.load().apps_for("text/plain");
            let got: Vec<&str> = apps.iter().map(|a| a.id.as_str()).collect();
            let mut seen = std::collections::HashSet::new();
            prop_assert!(got.iter().all(|g| seen.insert(*g)), "repeats in {got:?}");
            for r in &removed {
                prop_assert!(!got.contains(&id(r).as_str()), "removed app offered: {got:?}");
            }
            for d in defaults.iter().chain(&added).chain(&cached) {
                if !removed.contains(d) {
                    prop_assert!(got.contains(&id(d).as_str()), "{} missing from {got:?}", id(d));
                }
            }
        }
    }
}
