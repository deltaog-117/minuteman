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

//! How a folder's entries are ordered. Folders always come first; the key orders each group on its
//! own. Sorting happens when a listing is stored (see `BrowserState`), not when it is drawn, so the
//! cursor, `/` search and mouse hit-testing all index the list the user sees.

use std::cmp::Ordering;

use shared::DirEntryInfo;

/// What the entries are ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Modified,
    Mode,
}

impl SortKey {
    /// The next key in the `o` cycle; the last wraps round to the first.
    #[must_use]
    pub fn cycled(self) -> Self {
        match self {
            Self::Name => Self::Size,
            Self::Size => Self::Modified,
            Self::Modified => Self::Mode,
            Self::Mode => Self::Name,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Size => "size",
            Self::Modified => "modified",
            Self::Mode => "mode",
        }
    }

    /// Size and time put the biggest and newest first, which is what people look for; name and
    /// permissions start from the beginning.
    fn starts_descending(self) -> bool {
        matches!(self, Self::Size | Self::Modified)
    }
}

/// A key and which way round it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sort {
    pub key: SortKey,
    /// The opposite of the key's usual direction.
    pub reversed: bool,
}

impl Sort {
    /// Whether the order runs from large to small (or new to old, or Z to A) as shown.
    pub fn descending(self) -> bool {
        self.key.starts_descending() != self.reversed
    }

    /// What clicking `key`'s column title does: its usual direction if it was not the sort, the
    /// opposite direction if it was.
    #[must_use]
    pub fn clicked(self, key: SortKey) -> Self {
        if self.key == key {
            Self {
                key,
                reversed: !self.reversed,
            }
        } else {
            Self {
                key,
                reversed: false,
            }
        }
    }

    /// The order in words, for the status line: `size (largest first)`.
    pub fn describe(self) -> String {
        let way = match (self.key, self.descending()) {
            (SortKey::Name, false) => "A to Z",
            (SortKey::Name, true) => "Z to A",
            (SortKey::Size, true) => "largest first",
            (SortKey::Size, false) => "smallest first",
            (SortKey::Modified, true) => "newest first",
            (SortKey::Modified, false) => "oldest first",
            (SortKey::Mode, false) => "lowest first",
            (SortKey::Mode, true) => "highest first",
        };
        format!("{} ({way})", self.key.label())
    }

    /// The order of two entries: folders before files, then the key (flipped if descending), then
    /// the name so equal keys never shuffle between refreshes.
    pub fn compare(self, a: &DirEntryInfo, b: &DirEntryInfo) -> Ordering {
        match (a.is_dir, b.is_dir) {
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            _ => {}
        }
        let by_name = || a.name.to_lowercase().cmp(&b.name.to_lowercase());
        // A folder's size means nothing, so folders order by name whichever key is chosen.
        let by_key = match self.key {
            SortKey::Name => by_name(),
            SortKey::Size if a.is_dir => Ordering::Equal,
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Modified => a.modified.cmp(&b.modified),
            SortKey::Mode => a.mode.cmp(&b.mode),
        };
        let by_key = if self.descending() {
            by_key.reverse()
        } else {
            by_key
        };
        by_key.then_with(by_name)
    }

    pub fn sort(self, entries: &mut [DirEntryInfo]) {
        entries.sort_by(|a, b| self.compare(a, b));
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    fn entry(name: &str, is_dir: bool, size: u64, secs: u64, mode: u32) -> DirEntryInfo {
        DirEntryInfo {
            name: name.into(),
            path: PathBuf::from(name),
            is_dir,
            size,
            modified: Some(UNIX_EPOCH + Duration::from_secs(secs)),
            mode: Some(mode),
        }
    }

    fn sample() -> Vec<DirEntryInfo> {
        vec![
            entry("b.txt", false, 10, 300, 0o644),
            entry("A.txt", false, 500, 100, 0o600),
            entry("c.txt", false, 50, 200, 0o755),
            entry("zdir", true, 0, 50, 0o755),
            entry("adir", true, 0, 400, 0o700),
        ]
    }

    fn names(entries: &[DirEntryInfo]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    fn sorted(sort: Sort) -> Vec<DirEntryInfo> {
        let mut entries = sample();
        sort.sort(&mut entries);
        entries
    }

    #[test]
    fn name_order_ignores_case_and_keeps_folders_first() {
        let entries = sorted(Sort::default());
        assert_eq!(names(&entries), ["adir", "zdir", "A.txt", "b.txt", "c.txt"]);
    }

    #[test]
    fn size_starts_with_the_largest_file_and_orders_folders_by_name() {
        let entries = sorted(Sort {
            key: SortKey::Size,
            reversed: false,
        });
        assert_eq!(names(&entries), ["adir", "zdir", "A.txt", "c.txt", "b.txt"]);
    }

    #[test]
    fn modified_starts_with_the_newest() {
        let entries = sorted(Sort {
            key: SortKey::Modified,
            reversed: false,
        });
        assert_eq!(names(&entries), ["adir", "zdir", "b.txt", "c.txt", "A.txt"]);
    }

    #[test]
    fn reversing_flips_each_group_but_never_puts_files_before_folders() {
        let entries = sorted(Sort {
            key: SortKey::Name,
            reversed: true,
        });
        assert_eq!(names(&entries), ["zdir", "adir", "c.txt", "b.txt", "A.txt"]);
        let entries = sorted(Sort {
            key: SortKey::Size,
            reversed: true,
        });
        assert_eq!(names(&entries), ["adir", "zdir", "b.txt", "c.txt", "A.txt"]);
    }

    #[test]
    fn equal_keys_fall_back_to_the_name() {
        let mut entries = vec![
            entry("b", false, 5, 1, 0o644),
            entry("a", false, 5, 1, 0o644),
            entry("c", false, 5, 1, 0o644),
        ];
        Sort {
            key: SortKey::Size,
            reversed: false,
        }
        .sort(&mut entries);
        assert_eq!(names(&entries), ["a", "b", "c"]);
    }

    #[test]
    fn clicking_a_title_picks_it_then_flips_it() {
        let by_name = Sort::default();
        let size = by_name.clicked(SortKey::Size);
        assert_eq!(size.key, SortKey::Size);
        assert!(!size.reversed);
        assert!(size.descending(), "size starts largest first");
        let flipped = size.clicked(SortKey::Size);
        assert!(flipped.reversed);
        assert!(!flipped.descending());
        assert!(!by_name.descending());
        assert!(
            by_name.clicked(SortKey::Name).descending(),
            "name flips to Z-A"
        );
    }

    #[test]
    fn the_order_is_described_in_words() {
        assert_eq!(Sort::default().describe(), "name (A to Z)");
        let by_size = Sort::default().clicked(SortKey::Size);
        assert_eq!(by_size.describe(), "size (largest first)");
        assert_eq!(
            by_size.clicked(SortKey::Size).describe(),
            "size (smallest first)"
        );
        let by_time = Sort::default().clicked(SortKey::Modified);
        assert_eq!(by_time.describe(), "modified (newest first)");
    }

    #[test]
    fn the_key_cycle_visits_every_key_and_comes_back() {
        let mut key = SortKey::Name;
        let mut seen = vec![key];
        for _ in 0..4 {
            key = key.cycled();
            seen.push(key);
        }
        assert_eq!(seen.first(), seen.last());
        assert_eq!(seen.len(), 5);
        assert!(
            seen[..4]
                .iter()
                .all(|k| seen[..4].iter().filter(|o| o == &k).count() == 1)
        );
    }
}
