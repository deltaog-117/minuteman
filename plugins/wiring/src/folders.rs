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

//! The folders the wiring diagram has unfolded below a filesystem. The hardware is a `Topology`
//! that is rebuilt from the machine every second; the folders are not hardware, so they live
//! here, outlast every rebuild, and are added to a copy of the topology as more nodes to the right
//! (`compose`), where the layout treats them like any other.
//!
//! Opening a folder reads its subfolders on a thread of its own, so a slow disk or a share never
//! holds up the plugin's loop: until the answer arrives the folder shows a "loading…" note in place of
//! its children. Only folders are shown (files are what the browser is for), hidden ones only if
//! the browser is showing them, and a folder with many subfolders shows some of them and a note
//! saying how many more there are. Which ones: always the one on the way to the folder being
//! browsed and the one chosen, so the path down to where you are is never cut, and then the
//! siblings nearest to those, so the cut falls on what is furthest from where you are looking.
//! How many is a cap the layout chooses to make the whole diagram fit the window.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use shared::Vfs;
use shared::devices::topology::{Node, NodeKind, Topology};

/// The most subfolders a folder shows before the rest are folded into "+N more"; the layout
/// asks for fewer when the window is too small for that many.
pub const MAX_SHOWN: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Listing {
    Loading,
    /// The names of the subfolders, sorted.
    Loaded(Vec<String>),
    Failed(String),
}

type Answer = (PathBuf, Result<Vec<String>, String>);

pub struct Folders {
    vfs: Arc<dyn Vfs>,
    show_hidden: bool,
    expanded: HashSet<PathBuf>,
    listings: HashMap<PathBuf, Listing>,
    tx: Sender<Answer>,
    rx: Receiver<Answer>,
    /// The folder being browsed, whose way down is always shown.
    toward: Option<PathBuf>,
    /// The folder chosen in the diagram, whose way down is always shown too.
    chosen: Option<PathBuf>,
}

impl Folders {
    pub fn new(vfs: Arc<dyn Vfs>, show_hidden: bool) -> Self {
        let (tx, rx) = channel();
        Self {
            vfs,
            show_hidden,
            expanded: HashSet::new(),
            listings: HashMap::new(),
            tx,
            rx,
            toward: None,
            chosen: None,
        }
    }

    /// Says which folder is being browsed, so the way down to it is never folded away.
    pub fn set_toward(&mut self, here: Option<PathBuf>) {
        self.toward = here;
    }

    /// Says which folder is chosen in the diagram, so choosing a folder and then making the
    /// window smaller does not trim the folder away from under the cursor.
    pub fn set_chosen(&mut self, chosen: Option<PathBuf>) {
        self.chosen = chosen;
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    /// Opens `path`, reading its subfolders if they have not been read.
    pub fn expand(&mut self, path: &Path) {
        self.expanded.insert(path.to_path_buf());
        if self.listings.contains_key(path) {
            return;
        }
        self.listings.insert(path.to_path_buf(), Listing::Loading);
        let (vfs, tx, hidden, path) = (
            Arc::clone(&self.vfs),
            self.tx.clone(),
            self.show_hidden,
            path.to_path_buf(),
        );
        thread::spawn(move || {
            let answer = vfs
                .list_dir(&path)
                .map(|entries| {
                    let mut names: Vec<String> = entries
                        .into_iter()
                        .filter(|e| e.is_dir && (hidden || !e.name.starts_with('.')))
                        .map(|e| e.name)
                        .collect();
                    names.sort_by_key(|n| n.to_lowercase());
                    names
                })
                .map_err(|e| e.to_string());
            let _ = tx.send((path, answer));
        });
    }

    /// Whether hidden folders are shown from now on. A change forgets what was read, since the
    /// lists differ, and reads every open folder again; `true` when it changed anything.
    pub fn set_hidden(&mut self, hidden: bool) -> bool {
        if hidden == self.show_hidden {
            return false;
        }
        self.show_hidden = hidden;
        self.listings.clear();
        let open: Vec<PathBuf> = self.expanded.iter().cloned().collect();
        for path in open {
            self.expand(&path);
        }
        true
    }

    /// Closes `path` and everything opened below it. What was read there is forgotten, so opening
    /// it again shows what is on the disk then.
    pub fn collapse(&mut self, path: &Path) {
        self.expanded.retain(|p| !p.starts_with(path));
        self.listings.retain(|p, _| !p.starts_with(path));
    }

    /// Takes in the subfolder lists that have arrived, and says whether any did.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok((path, answer)) = self.rx.try_recv() {
            // An answer for a folder that was closed meanwhile has nowhere to go.
            if !self.listings.contains_key(&path) {
                continue;
            }
            self.listings.insert(
                path,
                match answer {
                    Ok(names) => Listing::Loaded(names),
                    Err(why) => Listing::Failed(why),
                },
            );
            any = true;
        }
        any
    }

    /// How many subfolders `path` has, once they have been read.
    pub fn count_of(&self, path: &Path) -> Option<usize> {
        match self.listings.get(path)? {
            Listing::Loaded(names) => Some(names.len()),
            _ => None,
        }
    }

    /// Whether a folder is still being read.
    pub fn pending(&self) -> bool {
        self.listings.values().any(|l| *l == Listing::Loading)
    }

    /// The names of the children of `path` on the way to the folder being browsed and to the one
    /// chosen (one name if they go the same way, none if neither goes through `path`).
    fn on_the_way(&self, path: &Path) -> Vec<String> {
        let child = |target: &Option<PathBuf>| {
            let rest = target.as_deref()?.strip_prefix(path).ok()?;
            Some(
                rest.components()
                    .next()?
                    .as_os_str()
                    .to_string_lossy()
                    .into_owned(),
            )
        };
        let mut names: Vec<String> = [&self.toward, &self.chosen]
            .into_iter()
            .filter_map(child)
            .collect();
        names.dedup();
        names
    }

    /// `base` with every opened folder added below the filesystem it is on, each showing at most
    /// `MAX_SHOWN` subfolders.
    pub fn compose(&self, base: &Topology) -> Topology {
        self.compose_capped(base, MAX_SHOWN)
    }

    /// Like [`compose`](Self::compose), with each folder showing at most `cap` subfolders (the
    /// way down to the browsed and the chosen folder excepted, so a very small cap can still
    /// show two).
    pub fn compose_capped(&self, base: &Topology, cap: usize) -> Topology {
        let mut topology = base.clone();
        let roots: Vec<(usize, PathBuf)> = base
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                let path = n.folder_path()?;
                self.expanded
                    .contains(path)
                    .then(|| (i, path.to_path_buf()))
            })
            .collect();
        for (at, path) in roots {
            self.attach(&mut topology, at, &path, cap);
        }
        topology
    }

    fn note(topology: &mut Topology, parent: usize, path: &Path, text: &str) {
        let mut note = Node::new(&format!("{}\u{0}note", path.display()), NodeKind::More);
        note.name = Some(text.to_string());
        note.path = Some(path.to_path_buf());
        let at = topology.add(note);
        topology.link(parent, at);
    }

    fn attach(&self, topology: &mut Topology, parent: usize, path: &Path, cap: usize) {
        match self.listings.get(path) {
            None | Some(Listing::Loading) => Self::note(topology, parent, path, "loading…"),
            Some(Listing::Failed(why)) => {
                Self::note(topology, parent, path, &format!("cannot read: {why}"));
            }
            Some(Listing::Loaded(names)) => {
                let musts = self.on_the_way(path);
                let musts: Vec<&str> = musts.iter().map(String::as_str).collect();
                let shown = pick(names, cap, &musts);
                for name in &shown {
                    let child_path = path.join(name);
                    let mut folder = Node::new(&child_path.display().to_string(), NodeKind::Folder);
                    folder.name = Some((*name).clone());
                    folder.path = Some(child_path.clone());
                    let child = topology.add(folder);
                    topology.link(parent, child);
                    if self.expanded.contains(&child_path) {
                        self.attach(topology, child, &child_path, cap);
                    }
                }
                if names.len() > shown.len() {
                    let more = names.len() - shown.len();
                    Self::note(topology, parent, path, &format!("+{more} more"));
                }
            }
        }
    }
}

/// The names to show, at most `cap` of them, in their order: every one of `musts` that is among
/// `names` (up to `cap`), then the names nearest to those, earlier ones first on a tie. With no
/// `musts` among the names, the first `cap`.
fn pick<'a>(names: &'a [String], cap: usize, musts: &[&str]) -> Vec<&'a String> {
    if names.len() <= cap {
        return names.iter().collect();
    }
    let mut anchors: Vec<usize> = musts
        .iter()
        .filter_map(|m| names.iter().position(|n| n == m))
        .collect();
    anchors.sort_unstable();
    anchors.dedup();
    anchors.truncate(cap);
    let mut keep = vec![false; names.len()];
    for &a in &anchors {
        keep[a] = true;
    }
    let mut rest: Vec<usize> = (0..names.len()).filter(|&i| !keep[i]).collect();
    // With no anchors every name is 0 away from nothing, so the order is simply the first ones.
    rest.sort_by_key(|&i| (anchors.iter().map(|&a| a.abs_diff(i)).min().unwrap_or(0), i));
    for &i in rest.iter().take(cap - anchors.len()) {
        keep[i] = true;
    }
    names
        .iter()
        .enumerate()
        .filter_map(|(i, n)| keep[i].then_some(n))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use shared::MemVfs;

    use super::*;

    /// A machine with one filesystem, mounted at `/data`.
    fn base() -> Topology {
        let mut t = Topology::default();
        let mut fs = Node::new("sda1", NodeKind::Partition);
        fs.mount = Some("/data".into());
        fs.fs = Some("ext4".into());
        fs.usage = Some("filesystem".into());
        t.add(fs);
        t
    }

    fn vfs() -> Arc<MemVfs> {
        let vfs = MemVfs::new();
        for dir in [
            "/data/alpha/inner/deep",
            "/data/Beta",
            "/data/.hidden",
            "/data/gamma",
        ] {
            vfs.add_dir(dir);
        }
        vfs.add_file("/data/readme.txt", b"files are not folders");
        Arc::new(vfs)
    }

    fn settle(folders: &mut Folders) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while folders.pending() {
            folders.poll();
            assert!(Instant::now() < deadline, "a folder was never read");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn names(t: &Topology) -> Vec<String> {
        t.nodes.iter().map(|n| n.title()).collect()
    }

    #[test]
    fn nothing_is_added_until_something_is_opened() {
        let folders = Folders::new(vfs(), false);
        assert_eq!(folders.compose(&base()), base());
    }

    #[test]
    fn opening_a_filesystem_shows_a_note_and_then_its_folders_without_files_or_hidden_ones() {
        let mut folders = Folders::new(vfs(), false);
        folders.expand(Path::new("/data"));
        let loading = folders.compose(&base());
        assert!(
            names(&loading).contains(&"loading…".to_string()),
            "{:?}",
            names(&loading)
        );
        assert!(folders.pending());
        settle(&mut folders);
        let t = folders.compose(&base());
        assert_eq!(
            names(&t),
            ["sda1", "alpha", "Beta", "gamma"],
            "sorted without regard to case; no readme.txt, no .hidden"
        );
        assert!(t.nodes[1..].iter().all(|n| n.kind == NodeKind::Folder));
        assert_eq!(t.nodes[1].path.as_deref(), Some(Path::new("/data/alpha")));
        assert_eq!(t.nodes[0].children, [1, 2, 3]);
        assert_eq!(t.nodes[2].parents, [0]);
    }

    #[test]
    fn hidden_folders_appear_when_the_browser_shows_them() {
        let mut folders = Folders::new(vfs(), true);
        folders.expand(Path::new("/data"));
        settle(&mut folders);
        assert!(names(&folders.compose(&base())).contains(&".hidden".to_string()));
    }

    #[test]
    fn a_folder_opened_inside_a_folder_hangs_under_it() {
        let mut folders = Folders::new(vfs(), false);
        folders.expand(Path::new("/data"));
        folders.expand(Path::new("/data/alpha"));
        settle(&mut folders);
        let t = folders.compose(&base());
        let alpha = t.nodes.iter().position(|n| n.title() == "alpha").unwrap();
        let inner = t.nodes.iter().position(|n| n.title() == "inner").unwrap();
        assert_eq!(t.nodes[inner].parents, [alpha]);
        assert!(folders.is_expanded(Path::new("/data/alpha")));
        assert!(!folders.is_expanded(Path::new("/data/gamma")));
    }

    #[test]
    fn closing_a_folder_hides_what_was_below_it_and_forgets_it_so_opening_again_reads_afresh() {
        let vfs = vfs();
        let mut folders = Folders::new(Arc::clone(&vfs) as Arc<dyn Vfs>, false);
        folders.expand(Path::new("/data"));
        folders.expand(Path::new("/data/alpha"));
        settle(&mut folders);
        folders.collapse(Path::new("/data"));
        assert_eq!(folders.compose(&base()), base());
        assert!(
            !folders.is_expanded(Path::new("/data/alpha")),
            "what was inside is closed too"
        );
        vfs.add_dir("/data/new");
        folders.expand(Path::new("/data"));
        settle(&mut folders);
        let again = names(&folders.compose(&base()));
        assert!(again.contains(&"new".to_string()), "{again:?}");
        assert!(
            !again.contains(&"inner".to_string()),
            "alpha is closed again"
        );
    }

    #[test]
    fn a_folder_that_cannot_be_read_says_why() {
        let mut folders = Folders::new(vfs(), false);
        folders.expand(Path::new("/data/does-not-exist"));
        settle(&mut folders);
        let mut t = Topology::default();
        let mut fs = Node::new("x", NodeKind::Partition);
        fs.mount = Some("/data/does-not-exist".into());
        fs.usage = Some("filesystem".into());
        t.add(fs);
        let shown = names(&folders.compose(&t));
        assert!(
            shown.iter().any(|n| n.starts_with("cannot read")),
            "{shown:?}"
        );
    }

    #[test]
    fn a_crowded_folder_shows_the_first_few_and_how_many_more() {
        let vfs = MemVfs::new();
        for i in 0..30 {
            vfs.add_dir(format!("/data/d{i:02}"));
        }
        let mut folders = Folders::new(Arc::new(vfs), false);
        folders.expand(Path::new("/data"));
        settle(&mut folders);
        let t = folders.compose(&base());
        let shown: Vec<&Node> = t
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Folder)
            .collect();
        assert_eq!(shown.len(), MAX_SHOWN);
        assert_eq!(shown[0].title(), "d00");
        let note = t.nodes.iter().find(|n| n.kind == NodeKind::More).unwrap();
        assert_eq!(note.title(), format!("+{} more", 30 - MAX_SHOWN));
        assert_eq!(
            note.path.as_deref(),
            Some(Path::new("/data")),
            "it opens that folder"
        );
    }

    #[test]
    fn the_way_down_to_the_folder_being_browsed_is_never_folded_away() {
        let vfs = MemVfs::new();
        for i in 0..30 {
            vfs.add_dir(format!("/data/d{i:02}"));
        }
        let mut folders = Folders::new(Arc::new(vfs), false);
        folders.set_toward(Some("/data/d27/sub".into()));
        folders.expand(Path::new("/data"));
        settle(&mut folders);
        let shown: Vec<String> = folders
            .compose(&base())
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Folder)
            .map(Node::title)
            .collect();
        assert_eq!(shown.len(), MAX_SHOWN);
        assert!(shown.contains(&"d27".to_string()), "{shown:?}");
        assert_eq!(
            (shown[0].as_str(), shown[MAX_SHOWN - 1].as_str()),
            ("d18", "d29"),
            "the siblings nearest the way down, not the first twelve: {shown:?}"
        );
    }

    #[test]
    fn a_smaller_cap_shows_fewer_folders_and_the_note_counts_the_rest() {
        let vfs = MemVfs::new();
        for i in 0..30 {
            vfs.add_dir(format!("/data/d{i:02}"));
        }
        let mut folders = Folders::new(Arc::new(vfs), false);
        folders.expand(Path::new("/data"));
        settle(&mut folders);
        for cap in [12, 8, 5, 3] {
            let t = folders.compose_capped(&base(), cap);
            let shown = t
                .nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Folder)
                .count();
            assert_eq!(shown, cap);
            let note = t.nodes.iter().find(|n| n.kind == NodeKind::More).unwrap();
            assert_eq!(note.title(), format!("+{} more", 30 - cap));
        }
    }

    #[test]
    fn the_chosen_folder_is_kept_as_well_as_the_way_to_the_browsed_one() {
        let vfs = MemVfs::new();
        for i in 0..30 {
            vfs.add_dir(format!("/data/d{i:02}"));
        }
        let mut folders = Folders::new(Arc::new(vfs), false);
        folders.set_toward(Some("/data/d02".into()));
        folders.set_chosen(Some("/data/d25".into()));
        folders.expand(Path::new("/data"));
        settle(&mut folders);
        let shown: Vec<String> = folders
            .compose_capped(&base(), 3)
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Folder)
            .map(Node::title)
            .collect();
        assert_eq!(shown, ["d01", "d02", "d25"], "{shown:?}");
    }

    #[test]
    fn picking_never_exceeds_the_cap_and_keeps_the_required_names_in_order() {
        let names: Vec<String> = (0..20).map(|i| format!("n{i:02}")).collect();
        let texts = |picked: Vec<&String>| -> Vec<String> { picked.into_iter().cloned().collect() };
        assert_eq!(pick(&names, 5, &[]).len(), 5);
        assert_eq!(
            texts(pick(&names, 5, &[]))[4],
            "n04",
            "no anchor: the first ones"
        );
        assert_eq!(
            texts(pick(&names, 5, &["n02"])),
            ["n00", "n01", "n02", "n03", "n04"]
        );
        // Far down: a window ending at the end of the list, nearest first.
        assert_eq!(
            texts(pick(&names, 5, &["n17"])),
            ["n15", "n16", "n17", "n18", "n19"]
        );
        assert_eq!(
            texts(pick(&names, 4, &["n03", "n15"])),
            ["n02", "n03", "n04", "n15"],
            "two anchors share the room; on a tie the earlier name goes first"
        );
        assert_eq!(pick(&names, 5, &["missing"]).len(), 5);
        assert_eq!(pick(&names[..3], 5, &["n02"]).len(), 3);
        assert!(pick(&[], 5, &["x"]).is_empty());
        assert!(pick(&names, 0, &["n17"]).is_empty());
        assert_eq!(
            pick(&names, 1, &["n03", "n15"]).len(),
            1,
            "more anchors than room"
        );
    }

    #[test]
    fn an_answer_for_a_folder_closed_meanwhile_is_dropped() {
        let mut folders = Folders::new(vfs(), false);
        folders.expand(Path::new("/data"));
        folders.collapse(Path::new("/data"));
        std::thread::sleep(Duration::from_millis(100));
        assert!(!folders.poll(), "nothing is waiting for it");
        assert_eq!(folders.compose(&base()), base());
    }
}
