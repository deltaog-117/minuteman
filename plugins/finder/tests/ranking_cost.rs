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

//! What ranking a big index costs: the numbers behind the fuzzy finder keeping its work off the
//! render thread and cutting it into slices. Ignored by default, because a figure from a debug
//! build or a busy machine means nothing; run by hand with
//! `cargo test --release -p finder --test ranking_cost -- --ignored --nocapture`.

use std::path::PathBuf;
use std::time::Instant;

use finder::rank::{Ranking, SHOWN};
use plugin_api::TreeEntry;
use shared::Query;

const ENTRIES: usize = 200_000;
/// What the finder scores between looks at its queue.
const SLICE: usize = 2048;

/// A tree shaped like a real one: folders a few levels deep holding files with ordinary names.
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
                size: None,
                modified: None,
            }
        })
        .collect()
}

#[test]
#[ignore = "a measurement, run by hand in a release build"]
fn ranking_a_large_index() {
    let index = index();
    let mut ranking = Ranking::new();

    let mut worst_slice = std::time::Duration::ZERO;
    let started = Instant::now();
    ranking.set_query(Query::new("a"));
    loop {
        let slice = Instant::now();
        let caught_up = ranking.advance(&index, SLICE);
        worst_slice = worst_slice.max(slice.elapsed());
        if caught_up {
            break;
        }
    }
    println!(
        "first letter  `a`    over {ENTRIES}: {:>8.1?} in total, longest slice of {SLICE} {:>8.1?}, {} match",
        started.elapsed(),
        worst_slice,
        ranking.matches(ENTRIES)
    );

    for typed in ["ab", "abc", "abcd"] {
        let started = Instant::now();
        ranking.set_query(Query::new(typed));
        ranking.advance(&index, usize::MAX);
        println!(
            "narrowed to   `{typed:<4}` {:>8.1?}, {} match",
            started.elapsed(),
            ranking.matches(ENTRIES)
        );
    }

    let started = Instant::now();
    ranking.set_query(Query::new("abc"));
    ranking.advance(&index, usize::MAX);
    println!(
        "deleted back  `abc`  {:>8.1?} (the level was kept)",
        started.elapsed()
    );

    let mut scratch = Ranking::new();
    scratch.set_query(Query::new("main"));
    let started = Instant::now();
    scratch.advance(&index, usize::MAX);
    println!(
        "from scratch  `main` {:>8.1?}, {} match",
        started.elapsed(),
        scratch.matches(ENTRIES)
    );
    let started = Instant::now();
    let top = scratch.top(ENTRIES, SHOWN);
    println!(
        "best {SHOWN} of {} matches picked in {:>8.1?}",
        scratch.matches(ENTRIES),
        started.elapsed()
    );
    assert_eq!(top.len(), SHOWN);
}
