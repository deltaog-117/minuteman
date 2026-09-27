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

//! Pure wall-clock math for the cinematic layer's first effect: a one-shot "transition" ramp for
//! a pane's focus-border color (see `style::blend_rgb` and `shell_layout`'s per-pane border
//! color). No ratatui or theme types here — just a `Duration` in and an `f64` out — so both the
//! call site and its tests stay simple.

use std::time::Duration;

/// How long a focus-border color transition takes to complete once a pane gains or loses focus.
pub const FOCUS_TRANSITION: Duration = Duration::from_millis(180);

/// How far a focus transition has progressed, from `0.0` (just changed) to `1.0` (settled) —
/// linear, since it only ever plays once per change rather than looping.
pub fn transition_t(elapsed: Duration) -> f64 {
    (elapsed.as_secs_f64() / FOCUS_TRANSITION.as_secs_f64()).min(1.0)
}

/// How many of the preview's source lines the typewriter reveal shows per second — fast enough
/// that even a full pane finishes well under a second, so it reads as a snappy flourish rather
/// than something the user has to wait out.
pub const REVEAL_LINES_PER_SEC: f64 = 90.0;

/// How long the main loop should poll fast after a reveal starts, regardless of the pane's real
/// height — generous enough for even a very tall terminal (a 72-row pane needs ~0.8s at
/// [`REVEAL_LINES_PER_SEC`]) without the caller having to know the pane's actual line count.
pub const REVEAL_MAX_WINDOW: Duration = Duration::from_millis(900);

/// Lines of a typewriter reveal visible after `elapsed`, capped at `total` — `total` once the
/// reveal has run long enough, `0` at `Duration::ZERO`. Floors rather than rounds, so a line is
/// only ever shown once its own moment has fully passed.
pub fn revealed_lines(elapsed: Duration, total: usize) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = (elapsed.as_secs_f64() * REVEAL_LINES_PER_SEC).floor() as usize;
    count.min(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_reaches_and_holds_at_one() {
        assert_eq!(transition_t(Duration::ZERO), 0.0);
        assert_eq!(transition_t(FOCUS_TRANSITION), 1.0);
        assert_eq!(transition_t(FOCUS_TRANSITION * 10), 1.0);
    }

    proptest::proptest! {
        #[test]
        fn transition_never_leaves_zero_to_one(millis in 0u64..1_000_000) {
            let t = transition_t(Duration::from_millis(millis));
            proptest::prop_assert!((0.0..=1.0).contains(&t), "{t}");
        }
    }

    #[test]
    fn reveal_starts_at_zero_and_settles_at_total() {
        assert_eq!(revealed_lines(Duration::ZERO, 40), 0);
        assert_eq!(revealed_lines(Duration::from_secs(10), 40), 40);
        assert_eq!(revealed_lines(Duration::from_secs(10), 0), 0);
    }

    #[test]
    fn reveal_advances_roughly_at_the_configured_rate() {
        // Half a second in, at 90 lines/sec, 45 lines should be showing — well short of a
        // realistic pane's line count, so the cap never kicks in here.
        assert_eq!(revealed_lines(Duration::from_millis(500), 1_000), 45);
    }

    proptest::proptest! {
        #[test]
        fn reveal_never_exceeds_total_and_never_decreases(millis in 0u64..60_000, total in 0usize..500) {
            let a = revealed_lines(Duration::from_millis(millis), total);
            let b = revealed_lines(Duration::from_millis(millis + 1), total);
            proptest::prop_assert!(a <= total);
            proptest::prop_assert!(b >= a);
        }
    }
}
