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

//! How busy each disk is: the kernel's running counts of bytes read and written
//! (`/proc/diskstats`), turned into a rate per second and a short history for a sparkline.

use std::collections::{HashMap, VecDeque};

/// How many samples of history are kept per disk.
pub const HISTORY: usize = 32;

/// Bytes read and written since boot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub read: u64,
    pub written: u64,
}

/// Bytes per second.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rates {
    pub read: u64,
    pub written: u64,
}

/// The counters of every disk and partition in the text of `/proc/diskstats`, by kernel name.
pub fn parse_diskstats(text: &str) -> HashMap<String, Counters> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let name = fields.get(2)?;
            let sectors_read: u64 = fields.get(5)?.parse().ok()?;
            let sectors_written: u64 = fields.get(9)?.parse().ok()?;
            // Sectors in this file are always 512 bytes, whatever the disk's own sector size.
            Some((
                (*name).to_string(),
                Counters {
                    read: sectors_read.saturating_mul(512),
                    written: sectors_written.saturating_mul(512),
                },
            ))
        })
        .collect()
}

/// Rates and history, kept up to date by feeding it the counters each time they are read.
#[derive(Debug, Default)]
pub struct Activity {
    last: HashMap<String, Counters>,
    rates: HashMap<String, Rates>,
    history: HashMap<String, VecDeque<u64>>,
}

impl Activity {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes in the counters read `seconds` after the previous ones. A disk seen for the first
    /// time has no rate yet; one that disappeared is forgotten; a counter that went backwards (a
    /// disk that was removed and came back) counts as no activity rather than a huge number.
    pub fn update(&mut self, now: &HashMap<String, Counters>, seconds: f64) {
        let seconds = seconds.max(0.001);
        self.rates.clear();
        self.history.retain(|name, _| now.contains_key(name));
        for (name, counters) in now {
            let rate = self
                .last
                .get(name)
                .map_or_else(Rates::default, |before| Rates {
                    read: per_second(counters.read.saturating_sub(before.read), seconds),
                    written: per_second(counters.written.saturating_sub(before.written), seconds),
                });
            self.rates.insert(name.clone(), rate);
            let ring = self.history.entry(name.clone()).or_default();
            if ring.len() == HISTORY {
                ring.pop_front();
            }
            ring.push_back(rate.read.saturating_add(rate.written));
        }
        self.last.clone_from(now);
    }

    pub fn rates(&self, name: &str) -> Rates {
        self.rates.get(name).copied().unwrap_or_default()
    }

    /// Combined read and write rate, oldest first.
    pub fn history(&self, name: &str) -> impl Iterator<Item = u64> + '_ {
        self.history.get(name).into_iter().flatten().copied()
    }
}

fn per_second(bytes: u64, seconds: f64) -> u64 {
    // A rate is far below 2^53 bytes a second, so the float is exact enough, and it is not
    // negative, so the conversion back cannot wrap.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let rate = (bytes as f64 / seconds).round() as u64;
    rate
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counters(read: u64, written: u64) -> HashMap<String, Counters> {
        HashMap::from([("sda".to_string(), Counters { read, written })])
    }

    #[test]
    fn diskstats_lines_become_byte_counters() {
        let stats = parse_diskstats(
            "   8       0 sda 100 0 2000 50 30 0 400 20 0 10 70\n   8       1 sda1 90 0 1000 40 20 0 200 10 0 8 50\nshort line\n",
        );
        assert_eq!(
            stats["sda"],
            Counters {
                read: 2000 * 512,
                written: 400 * 512
            }
        );
        assert_eq!(stats["sda1"].read, 1000 * 512);
        assert_eq!(stats.len(), 2);
    }

    #[test]
    fn the_first_reading_has_no_rate_and_the_next_has_one() {
        let mut activity = Activity::new();
        activity.update(&counters(1000, 0), 1.0);
        assert_eq!(activity.rates("sda"), Rates::default());
        activity.update(&counters(3000, 500), 2.0);
        assert_eq!(
            activity.rates("sda"),
            Rates {
                read: 1000,
                written: 250
            }
        );
        assert_eq!(activity.history("sda").collect::<Vec<_>>(), [0, 1250]);
    }

    #[test]
    fn a_counter_that_went_backwards_is_no_activity() {
        let mut activity = Activity::new();
        activity.update(&counters(5000, 5000), 1.0);
        activity.update(&counters(10, 10), 1.0);
        assert_eq!(activity.rates("sda"), Rates::default());
    }

    #[test]
    fn history_is_bounded_and_a_vanished_disk_is_forgotten() {
        let mut activity = Activity::new();
        for i in 0..(HISTORY as u64 + 10) {
            activity.update(&counters(i * 100, 0), 1.0);
        }
        assert_eq!(activity.history("sda").count(), HISTORY);
        activity.update(&HashMap::new(), 1.0);
        assert_eq!(activity.history("sda").count(), 0);
        assert_eq!(activity.rates("sda"), Rates::default());
    }

    #[test]
    fn an_unknown_disk_has_no_rate_and_no_history() {
        let activity = Activity::new();
        assert_eq!(activity.rates("nope"), Rates::default());
        assert_eq!(activity.history("nope").count(), 0);
    }
}
