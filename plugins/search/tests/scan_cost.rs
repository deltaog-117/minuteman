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

//! What checking a big index costs: the numbers behind the search keeping its work in slices.
//! Ignored by default, because a figure from a debug build or a busy machine means nothing; run by
//! hand with `cargo test --release -p search --test scan_cost -- --ignored --nocapture`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use plugin_api::TreeEntry;
use search::query::Search;
use search::scan::Scan;

const ENTRIES: usize = 200_000;
/// What the search checks between looks at its queue.
const SLICE: usize = 2048;
const NOW: u64 = 1_700_000_000;

/// A tree shaped like a real one: folders a few levels deep holding files of ordinary names.
fn index() -> Vec<TreeEntry> {
    const WORDS: [&str; 12] = [
        "src",
        "lib",
        "main",
        "config",
        "README",
        "target",
        "node_modules",
        "docs",
        "test",
        "utils",
        "parser",
        "view",
    ];
    (0..ENTRIES)
        .map(|i| {
            let rel = format!(
                "{}/{}/{}/{}_{}.{}",
                WORDS[i % 12],
                WORDS[(i / 12) % 12],
                WORDS[(i / 144) % 12],
                WORDS[(i / 7) % 12],
                i,
                ["rs", "md", "toml", "txt"][i % 4]
            );
            TreeEntry {
                path: PathBuf::from("/root").join(&rel),
                rel,
                is_dir: false,
                size: Some((i as u64 % 5000) * 1024),
                modified: Some(NOW - (i as u64 % 400) * 86_400),
            }
        })
        .collect()
}

#[test]
#[ignore = "a measurement, run by hand in a release build"]
fn checking_a_large_index() {
    let index = index();
    for query in [
        "main",
        "*.rs",
        "size:>1M",
        "*.rs size:>1M mtime:<30d",
        "re:^src/.*_1\\d*\\.rs$",
    ] {
        let mut scan = Scan::default();
        scan.set_query(Search::parse(query).search, NOW);
        let mut longest = Duration::ZERO;
        let started = Instant::now();
        loop {
            let slice = Instant::now();
            let caught_up = scan.advance(&index, SLICE);
            longest = longest.max(slice.elapsed());
            if caught_up {
                break;
            }
        }
        println!(
            "{query:<28} over {ENTRIES}: {:>8.1?} in total, longest slice of {SLICE} {:>8.1?}, {} match",
            started.elapsed(),
            longest,
            scan.matches()
        );
    }
}
