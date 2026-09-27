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

//! The header's optional system segment (`[config] system_hud`): how long this session has run,
//! and — on Linux — the load average and memory used/total, read straight from `/proc/loadavg`
//! and `/proc/meminfo`. No new dependency, the same choice `git_status` and `disk_usage` already
//! made for their own data (see `DIARY.md` for the measurement behind it): a `sysinfo`-based
//! cross-platform version is one to reach for later if this ever needs to work outside Linux, not
//! before.
//!
//! Reading two small `/proc` files can't hang the way `git_status`'s subprocess can, so this has
//! no timeout/kill machinery — just a plain synchronous read, throttled to [`REFRESH_EVERY`] so a
//! fast tick (see `main`'s `poll_timeout`) doesn't reread them every frame.

use std::time::{Duration, Instant, SystemTime};

use crate::hud::{format_age, format_size};

/// Time between refreshes of the load average and memory figures. Session uptime doesn't need
/// this — it's computed from `started`, not read from anywhere — but reading two files a frame
/// during a fast tick (a shell open, a boot splash fading, a preview revealing) would be wasted
/// work for numbers that don't change that often.
const REFRESH_EVERY: Duration = Duration::from_secs(2);

/// `/proc/loadavg`'s first three fields: the 1, 5 and 15-minute load averages.
pub fn parse_loadavg(text: &str) -> Option<(f64, f64, f64)> {
    let mut fields = text.split_whitespace();
    let one = fields.next()?.parse().ok()?;
    let five = fields.next()?.parse().ok()?;
    let fifteen = fields.next()?.parse().ok()?;
    Some((one, five, fifteen))
}

/// One `Key:   value kB` line's value, in bytes.
fn parse_kb_line<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| line.strip_prefix(key))
        .map(str::trim)
}

fn kb_to_bytes(value: &str) -> Option<u64> {
    value
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()
        .map(|kb| kb * 1024)
}

/// `/proc/meminfo`'s `(used, total)` bytes, from `MemTotal` and `MemAvailable` — "available"
/// (unlike "free") already accounts for reclaimable cache, matching what a user means by "how
/// much memory is actually free."
pub fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let total = kb_to_bytes(parse_kb_line(text, "MemTotal:")?)?;
    let available = kb_to_bytes(parse_kb_line(text, "MemAvailable:")?)?;
    Some((total.saturating_sub(available), total))
}

pub struct SystemHud {
    started: Instant,
    last_refresh: Instant,
    load: Option<(f64, f64, f64)>,
    mem: Option<(u64, u64)>,
}

impl SystemHud {
    pub fn new(now: Instant) -> Self {
        let mut hud = Self {
            started: now,
            // Due immediately: a session's first frame should already show what it can.
            last_refresh: now - REFRESH_EVERY,
            load: None,
            mem: None,
        };
        hud.refresh();
        hud
    }

    /// Rereads `/proc` if [`REFRESH_EVERY`] has passed since the last time. Call once per loop
    /// turn; a no-op most of those turns.
    pub fn tick(&mut self, now: Instant) {
        if now.duration_since(self.last_refresh) < REFRESH_EVERY {
            return;
        }
        self.last_refresh = now;
        self.refresh();
    }

    fn refresh(&mut self) {
        self.load = std::fs::read_to_string("/proc/loadavg")
            .ok()
            .and_then(|text| parse_loadavg(&text));
        self.mem = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|text| parse_meminfo(&text));
    }

    /// `"up 12m  load 0.42 0.38 0.31  mem 3.2G/16G"` — whichever parts are known. Uptime is
    /// always known (it needs no `/proc`); the other two are absent together on a platform
    /// without one, or if it existed but didn't parse.
    pub fn summary(&self, now: Instant) -> String {
        let uptime = now.duration_since(self.started);
        let started_at = SystemTime::now() - uptime;
        let mut parts = vec![format!(
            "up {}",
            format_age(SystemTime::now(), Some(started_at))
        )];
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

    #[test]
    fn loadavg_parses_the_first_three_fields() {
        assert_eq!(
            parse_loadavg("0.42 0.38 0.31 2/456 12345"),
            Some((0.42, 0.38, 0.31))
        );
    }

    #[test]
    fn loadavg_is_none_on_anything_that_does_not_look_like_it() {
        assert_eq!(parse_loadavg(""), None);
        assert_eq!(parse_loadavg("0.42 0.38"), None);
        assert_eq!(parse_loadavg("not a number here at all"), None);
    }

    #[test]
    fn meminfo_reads_total_and_derives_used_from_available() {
        let text =
            "MemTotal:       16384000 kB\nMemFree:         512000 kB\nMemAvailable:   8192000 kB\n";
        assert_eq!(
            parse_meminfo(text),
            Some((16384000 * 1024 - 8192000 * 1024, 16384000 * 1024))
        );
    }

    #[test]
    fn meminfo_does_not_care_about_key_order() {
        let text = "MemAvailable:   1000 kB\nMemTotal:       2000 kB\n";
        assert_eq!(parse_meminfo(text), Some((1000 * 1024, 2000 * 1024)));
    }

    #[test]
    fn meminfo_is_none_without_both_keys() {
        assert_eq!(parse_meminfo("MemTotal: 2000 kB\n"), None);
        assert_eq!(parse_meminfo(""), None);
        assert_eq!(parse_meminfo("garbage\nmore garbage\n"), None);
    }

    #[test]
    fn summary_always_has_an_uptime_even_with_nothing_from_proc() {
        let hud = SystemHud {
            started: Instant::now(),
            last_refresh: Instant::now(),
            load: None,
            mem: None,
        };
        let summary = hud.summary(Instant::now());
        assert!(summary.starts_with("up "), "{summary:?}");
        assert!(!summary.contains("load"));
        assert!(!summary.contains("mem"));
    }

    #[test]
    fn summary_adds_load_and_mem_once_known() {
        let hud = SystemHud {
            started: Instant::now(),
            last_refresh: Instant::now(),
            load: Some((0.42, 0.38, 0.31)),
            mem: Some((1024, 2048)),
        };
        let summary = hud.summary(Instant::now());
        assert!(summary.contains("load 0.42 0.38 0.31"), "{summary:?}");
        assert!(summary.contains("mem 1.0K/2.0K"), "{summary:?}");
    }

    #[test]
    fn tick_only_refreshes_after_the_interval() {
        let mut hud = SystemHud::new(Instant::now());
        let before = hud.last_refresh;
        hud.tick(before + Duration::from_millis(1));
        assert_eq!(hud.last_refresh, before, "too soon to refresh again");
        hud.tick(before + REFRESH_EVERY);
        assert_eq!(hud.last_refresh, before + REFRESH_EVERY);
    }

    proptest::proptest! {
        #[test]
        fn loadavg_never_panics_on_arbitrary_text(text in ".{0,200}") {
            let _ = parse_loadavg(&text);
        }

        #[test]
        fn meminfo_never_panics_on_arbitrary_text(text in ".{0,500}") {
            let _ = parse_meminfo(&text);
        }
    }
}
