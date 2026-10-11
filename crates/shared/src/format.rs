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

//! Numbers and times as text: the forms the file lists, the status bar and the Inspect plugin
//! all print. Kept here, apart from any interface, so a plugin and the interface cannot drift
//! apart on what `1.2K` or a permission string looks like.

use std::time::{SystemTime, UNIX_EPOCH};

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
    // One decimal below 10, none above — and rounding up to 1024 must roll into the next unit
    // rather than print "1024K".
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

/// `1234567` as `1,234,567`.
pub fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// `ls -l`'s ten-character mode: type letter, then rwx for user, group and other, with the
/// setuid, setgid and sticky bits shown in the execute slots (lower case when that execute bit
/// is also set, upper case when it is not).
pub fn mode_string(mode: u32) -> String {
    let kind = match mode & 0o170_000 {
        0o040_000 => 'd',
        0o120_000 => 'l',
        0o100_000 => '-',
        0o060_000 => 'b',
        0o020_000 => 'c',
        0o010_000 => 'p',
        0o140_000 => 's',
        _ => '?',
    };
    let mut out = String::with_capacity(10);
    out.push(kind);
    // (read, write, execute, the special bit that replaces execute, its two letters)
    let classes = [
        (0o400, 0o200, 0o100, 0o4000, ('s', 'S')),
        (0o040, 0o020, 0o010, 0o2000, ('s', 'S')),
        (0o004, 0o002, 0o001, 0o1000, ('t', 'T')),
    ];
    for (read, write, exec, special, (with_exec, without_exec)) in classes {
        out.push(if mode & read != 0 { 'r' } else { '-' });
        out.push(if mode & write != 0 { 'w' } else { '-' });
        out.push(match (mode & special != 0, mode & exec != 0) {
            (true, true) => with_exec,
            (true, false) => without_exec,
            (false, true) => 'x',
            (false, false) => '-',
        });
    }
    out
}

/// `2023-11-14 22:13:20 +01:00` in this machine's time zone, or `—` for a missing or pre-1970
/// time. The offset is written out so the time is unambiguous.
pub fn format_time(time: Option<SystemTime>) -> String {
    format_in(time, &chrono::Local)
}

/// [`format_time`] in a chosen zone, so the formatting can be tested without depending on the
/// zone of the machine running the tests.
pub fn format_in<Tz>(time: Option<SystemTime>, zone: &Tz) -> String
where
    Tz: chrono::TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let Some(secs) = time
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_secs()).ok())
    else {
        return "—".into();
    };
    match zone.timestamp_opt(secs, 0).single() {
        Some(local) => local.format("%Y-%m-%d %H:%M:%S %:z").to_string(),
        None => "—".into(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use proptest::prelude::*;

    use super::*;

    fn at(secs: u64) -> Option<SystemTime> {
        Some(UNIX_EPOCH + Duration::from_secs(secs))
    }

    #[test]
    fn sizes_stay_within_five_cells_and_never_print_1024() {
        assert_eq!(format_size(0), "0B");
        assert_eq!(format_size(1023), "1023B");
        assert_eq!(format_size(1024), "1.0K");
        assert_eq!(format_size(14 * 1024), "14K");
        assert_eq!(format_size(1_048_575), "1.0M");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0G");
        // Up to just under 1 EiB — beyond any real file, and past the largest unit.
        for exp in 0..60 {
            let n = 1u64 << exp;
            for bytes in [n - 1, n, n + 1] {
                let s = format_size(bytes);
                assert!(s.chars().count() <= 5, "{bytes} -> {s}");
                assert!(!s.starts_with("1024"), "{bytes} -> {s}");
            }
        }
    }

    #[test]
    fn modes_read_like_ls() {
        assert_eq!(mode_string(0o040_755), "drwxr-xr-x");
        assert_eq!(mode_string(0o100_644), "-rw-r--r--");
        assert_eq!(mode_string(0o120_777), "lrwxrwxrwx");
        assert_eq!(mode_string(0o104_755), "-rwsr-xr-x");
        assert_eq!(mode_string(0o104_644), "-rwSr--r--");
        assert_eq!(mode_string(0o041_777), "drwxrwxrwt");
        assert_eq!(mode_string(0o041_776), "drwxrwxrwT");
        assert_eq!(mode_string(0o102_755), "-rwxr-sr-x");
    }

    #[test]
    fn times_are_exact_across_leap_days_and_the_epoch() {
        use chrono::Utc;
        assert_eq!(format_in(at(0), &Utc), "1970-01-01 00:00:00 +00:00");
        assert_eq!(
            format_in(at(951_782_400), &Utc),
            "2000-02-29 00:00:00 +00:00"
        );
        assert_eq!(
            format_in(at(1_700_000_000), &Utc),
            "2023-11-14 22:13:20 +00:00"
        );
        assert_eq!(
            format_in(at(4_102_444_799), &Utc),
            "2099-12-31 23:59:59 +00:00"
        );
        assert_eq!(format_in(None, &Utc), "—");
    }

    #[test]
    fn a_time_is_shown_in_the_zone_it_is_given_with_its_offset() {
        let plus_two = chrono::FixedOffset::east_opt(2 * 3600).unwrap();
        let minus_330 = chrono::FixedOffset::west_opt(3 * 3600 + 1800).unwrap();
        assert_eq!(
            format_in(at(1_700_000_000), &plus_two),
            "2023-11-15 00:13:20 +02:00"
        );
        assert_eq!(
            format_in(at(1_700_000_000), &minus_330),
            "2023-11-14 18:43:20 -03:30"
        );
    }

    #[test]
    fn local_time_is_the_same_instant_as_utc() {
        use chrono::{DateTime, Utc};
        let shown = format_time(at(1_700_000_000));
        let parsed = DateTime::parse_from_str(&shown, "%Y-%m-%d %H:%M:%S %:z").unwrap();
        assert_eq!(parsed.with_timezone(&Utc).timestamp(), 1_700_000_000);
    }

    #[test]
    fn digits_are_grouped_in_threes() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1,000");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
    }

    proptest! {
        #[test]
        fn a_mode_is_always_ten_characters(mode in any::<u32>()) {
            prop_assert_eq!(mode_string(mode).chars().count(), 10);
        }

        #[test]
        fn later_times_never_sort_before_earlier_ones(a in 0u64..4_000_000_000, b in 0u64..4_000_000_000) {
            let (early, late) = (a.min(b), a.max(b));
            prop_assert!(format_in(at(early), &chrono::Utc) <= format_in(at(late), &chrono::Utc));
        }

        #[test]
        fn grouping_only_adds_commas(n in any::<u64>()) {
            prop_assert_eq!(group_thousands(n).replace(',', ""), n.to_string());
        }
    }
}
