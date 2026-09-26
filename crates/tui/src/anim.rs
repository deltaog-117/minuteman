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
}
