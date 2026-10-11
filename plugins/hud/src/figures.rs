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

//! What the pill says: `up 12m  load 0.42 0.38 0.31  mem 3.2G/16G`. The numbers come from
//! `/proc/loadavg` and `/proc/meminfo`, two small files that cannot hang the way a subprocess can,
//! so they are read inline. Without them (not Linux) the pill is the uptime alone.

use std::time::Duration;

/// `/proc/loadavg`'s first three fields: the 1, 5 and 15-minute load averages.
pub fn parse_loadavg(text: &str) -> Option<(f64, f64, f64)> {
    let mut fields = text.split_whitespace();
    let one = fields.next()?.parse().ok()?;
    let five = fields.next()?.parse().ok()?;
    let fifteen = fields.next()?.parse().ok()?;
    Some((one, five, fifteen))
}

fn kb_of(text: &str, key: &str) -> Option<u64> {
    let value = text.lines().find_map(|line| line.strip_prefix(key))?;
    value
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

/// `/proc/meminfo`'s `(used, total)` bytes. Used is total less `MemAvailable` rather than
/// `MemFree`, because available counts the cache that can be given back, which is what a reader
/// means by memory that is actually free.
pub fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let total = kb_of(text, "MemTotal:")?;
    let available = kb_of(text, "MemAvailable:")?;
    Some((total.saturating_sub(available), total))
}

/// A byte count in at most five cells: `512B`, `1.2K`, `14K`, `3.4M`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    if bytes < 1024 {
        return format!("{bytes}B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    // Rounding up to 1024 must roll into the next unit rather than print "1024K".
    let rounded = if value < 10.0 {
        (value * 10.0).round() / 10.0
    } else {
        value.round()
    };
    if rounded >= 1024.0 && unit < UNITS.len() - 1 {
        return format!("1.0{}", UNITS[unit + 1]);
    }
    if value < 10.0 {
        format!("{rounded:.1}{}", UNITS[unit])
    } else {
        format!("{rounded:.0}{}", UNITS[unit])
    }
}

/// How long the session has run, in at most three cells: `now`, `5m`, `2h`, `3d`, `4w`, `8mo`,
/// `2y`.
pub fn format_uptime(uptime: Duration) -> String {
    let secs = uptime.as_secs();
    match secs {
        0..=59 => "now".into(),
        60..=3_599 => format!("{}m", secs / 60),
        3_600..=86_399 => format!("{}h", secs / 3_600),
        86_400..=604_799 => format!("{}d", secs / 86_400),
        604_800..=2_591_999 => format!("{}w", secs / 604_800),
        2_592_000..=31_535_999 => format!("{}mo", secs / 2_592_000),
        _ => format!("{}y", secs / 31_536_000),
    }
}

/// What was read from `/proc` at one moment; either figure is absent where there is no such file
/// or it did not parse.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Figures {
    pub load: Option<(f64, f64, f64)>,
    pub mem: Option<(u64, u64)>,
}

impl Figures {
    pub fn read() -> Self {
        Self {
            load: std::fs::read_to_string("/proc/loadavg")
                .ok()
                .and_then(|text| parse_loadavg(&text)),
            mem: std::fs::read_to_string("/proc/meminfo")
                .ok()
                .and_then(|text| parse_meminfo(&text)),
        }
    }

    /// `up 12m  load 0.42 0.38 0.31  mem 3.2G/16G`, whichever parts are known. The uptime is always
    /// known; it needs no file.
    pub fn summary(&self, uptime: Duration) -> String {
        let mut parts = vec![format!("up {}", format_uptime(uptime))];
        if let Some((one, five, fifteen)) = self.load {
            parts.push(format!("load {one:.2} {five:.2} {fifteen:.2}"));
        }
        if let Some((used, total)) = self.mem {
            parts.push(format!("mem {}/{}", format_size(used), format_size(total)));
        }
        parts.join("  ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn loadavg_parses_the_first_three_fields() {
        assert_eq!(
            parse_loadavg("0.42 0.38 0.31 2/456 12345"),
            Some((0.42, 0.38, 0.31))
        );
        assert_eq!(parse_loadavg(""), None);
        assert_eq!(parse_loadavg("0.42 0.38"), None);
        assert_eq!(parse_loadavg("not a number here at all"), None);
    }

    #[test]
    fn meminfo_derives_used_from_available_whatever_the_key_order() {
        let text =
            "MemTotal:       16384000 kB\nMemFree:         512000 kB\nMemAvailable:   8192000 kB\n";
        assert_eq!(parse_meminfo(text), Some((8192000 * 1024, 16384000 * 1024)));
        let swapped = "MemAvailable:   1000 kB\nMemTotal:       2000 kB\n";
        assert_eq!(parse_meminfo(swapped), Some((1000 * 1024, 2000 * 1024)));
        assert_eq!(parse_meminfo("MemTotal: 2000 kB\n"), None);
        assert_eq!(parse_meminfo("garbage\nmore garbage\n"), None);
    }

    #[test]
    fn sizes_stay_short_and_roll_over_into_the_next_unit() {
        assert_eq!(format_size(512), "512B");
        assert_eq!(format_size(1024), "1.0K");
        assert_eq!(format_size(1024 * 1024 - 1), "1.0M");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024 + 1), "3.0G");
    }

    #[test]
    fn uptime_reads_in_the_largest_whole_unit() {
        let at = |secs| format_uptime(Duration::from_secs(secs));
        assert_eq!(at(59), "now");
        assert_eq!(at(60), "1m");
        assert_eq!(at(3_600), "1h");
        assert_eq!(at(86_400), "1d");
        assert_eq!(at(31_536_000), "1y");
    }

    #[test]
    fn the_summary_is_the_uptime_alone_without_proc_and_gains_the_rest_when_known() {
        let bare = Figures::default().summary(Duration::from_secs(300));
        assert_eq!(bare, "up 5m");
        let full = Figures {
            load: Some((0.42, 0.38, 0.31)),
            mem: Some((1024, 2048)),
        }
        .summary(Duration::from_secs(300));
        assert_eq!(full, "up 5m  load 0.42 0.38 0.31  mem 1.0K/2.0K");
    }

    proptest! {
        #[test]
        fn parsing_never_panics_on_arbitrary_text(text in ".{0,500}") {
            let _ = parse_loadavg(&text);
            let _ = parse_meminfo(&text);
        }

        #[test]
        // Past 1000 PiB the petabyte count needs a fifth digit; no machine's memory gets near.
        fn a_size_is_never_wider_than_five_cells(bytes in 0..(1000u64 << 50)) {
            prop_assert!(format_size(bytes).chars().count() <= 5);
        }
    }
}
