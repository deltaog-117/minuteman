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

//! Pure RGB/HSV/hex color math for the appearance popup's color picker (`tui::appearance_popup`).
//! No terminal, rendering or config concerns live here — just the conversions a picker needs to
//! turn slider nudges into the `#rrggbb` strings `appearance.toml` and `Theme` already speak.

/// A color in the space a picker's sliders actually move in: an angle (`h`) and two percentages
/// (`s`, `v`), rather than three coupled bytes — nudging "hue" alone in RGB would otherwise also
/// have to touch two or three channels at once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsv {
    /// Hue, in degrees. Not normalized to `[0, 360)` on its own — call [`Hsv::clamped`] after any
    /// nudge before reading or converting it.
    pub h: f64,
    /// Saturation, as a percentage.
    pub s: f64,
    /// Value (brightness), as a percentage.
    pub v: f64,
}

impl Hsv {
    /// `h` wrapped into `[0, 360)` (it's an angle, so it never needs clamping, only wrapping) and
    /// `s`/`v` clamped into `[0, 100]`. A slider nudge always produces a value that needs this
    /// before it's valid to convert or display.
    pub fn clamped(self) -> Self {
        Self {
            h: self.h.rem_euclid(360.0),
            s: self.s.clamp(0.0, 100.0),
            v: self.v.clamp(0.0, 100.0),
        }
    }

    /// Renders as the lowercase `#rrggbb` form every other color value in `appearance.toml` uses.
    /// Always 7 characters; never fails, since every `Hsv` (once clamped) maps to some color.
    pub fn to_hex(self) -> String {
        let Self { h, s, v } = self.clamped();
        let (r, g, b) = hsv_to_rgb(h, s, v);
        format!("#{r:02x}{g:02x}{b:02x}")
    }
}

/// Parses a `#rrggbb`/`#rgb` string (as `theme.rs`'s color fields accept) into HSV, for seeding
/// the picker from whatever a color row already holds. `None` for anything else — a bare color
/// name (`"cyan"`) can't be inverted without the terminal's own palette, so the caller falls back
/// to a sensible default instead of guessing.
pub fn hex_to_hsv(value: &str) -> Option<Hsv> {
    let (r, g, b) = hex_to_rgb(value)?;
    Some(rgb_to_hsv(r, g, b))
}

fn hex_to_rgb(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.strip_prefix('#')?;
    if !hex.is_ascii() {
        return None;
    }
    let channel = |s: &str| u8::from_str_radix(s, 16).ok();
    match hex.len() {
        6 => Some((
            channel(&hex[0..2])?,
            channel(&hex[2..4])?,
            channel(&hex[4..6])?,
        )),
        3 => {
            let double = |i: usize| channel(&hex[i..=i]).map(|v| v * 17);
            Some((double(0)?, double(1)?, double(2)?))
        }
        _ => None,
    }
}

fn rgb_to_hsv(r: u8, g: u8, b: u8) -> Hsv {
    let (r, g, b) = (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { delta / max * 100.0 };
    Hsv {
        h,
        s,
        v: max * 100.0,
    }
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let (s, v) = (s / 100.0, v / 100.0);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (r1, g1, b1) = match (h.rem_euclid(360.0) / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |chan: f64| ((chan + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (byte(r1), byte(g1), byte(b1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_and_three_digit_hex_parse_the_same_color() {
        assert_eq!(hex_to_hsv("#00aaff"), hex_to_hsv("#0af"));
        assert!(hex_to_hsv("#0af").is_some());
    }

    #[test]
    fn a_color_name_does_not_parse_as_hex() {
        assert_eq!(hex_to_hsv("cyan"), None);
        assert_eq!(hex_to_hsv("keep"), None);
    }

    #[test]
    fn malformed_hex_does_not_parse() {
        for bad in ["#", "#12", "#gggggg", "#1234567"] {
            assert_eq!(hex_to_hsv(bad), None, "{bad}");
        }
    }

    #[test]
    fn primary_colors_land_on_the_expected_hue() {
        let red = hex_to_hsv("#ff0000").expect("valid hex");
        assert!((red.h - 0.0).abs() < 1.0);
        assert!((red.s - 100.0).abs() < 1.0);
        assert!((red.v - 100.0).abs() < 1.0);

        let green = hex_to_hsv("#00ff00").expect("valid hex");
        assert!((green.h - 120.0).abs() < 1.0);

        let blue = hex_to_hsv("#0000ff").expect("valid hex");
        assert!((blue.h - 240.0).abs() < 1.0);
    }

    #[test]
    fn black_and_white_have_no_hue_or_saturation_to_speak_of() {
        let white = hex_to_hsv("#ffffff").expect("valid hex");
        assert_eq!((white.s, white.v), (0.0, 100.0));
        let black = hex_to_hsv("#000000").expect("valid hex");
        assert_eq!(black.v, 0.0);
    }

    #[test]
    fn clamped_wraps_hue_and_clamps_saturation_and_value() {
        let clamped = Hsv {
            h: -30.0,
            s: 150.0,
            v: -10.0,
        }
        .clamped();
        assert_eq!(clamped.h, 330.0);
        assert_eq!(clamped.s, 100.0);
        assert_eq!(clamped.v, 0.0);
    }

    #[test]
    fn to_hex_always_produces_a_seven_char_lowercase_hex_string() {
        let hex = Hsv {
            h: 275.0,
            s: 40.0,
            v: 90.0,
        }
        .to_hex();
        assert_eq!(hex.len(), 7);
        assert!(hex.starts_with('#'));
        assert!(
            hex[1..]
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    proptest::proptest! {
        /// Every byte triple survives an RGB -> HSV -> RGB round trip within rounding error —
        /// the picker must never visibly shift a color just by being opened.
        #[test]
        fn rgb_round_trips_through_hsv(r in 0u8..=255, g in 0u8..=255, b in 0u8..=255) {
            let hex = format!("#{r:02x}{g:02x}{b:02x}");
            let roundtrip = hex_to_hsv(&hex).expect("valid hex").to_hex();
            let (r2, g2, b2) = hex_to_rgb(&roundtrip).expect("valid hex");
            let close = |a: u8, b: u8| (i32::from(a) - i32::from(b)).abs() <= 1;
            proptest::prop_assert!(close(r, r2) && close(g, g2) && close(b, b2), "{hex} -> {roundtrip}");
        }

        /// Whatever `Hsv` a sequence of slider nudges produces, `clamped` always leaves it in a
        /// range `to_hex` can convert without panicking.
        #[test]
        fn any_hsv_converts_after_clamping(h in -1000.0f64..1000.0, s in -1000.0f64..1000.0, v in -1000.0f64..1000.0) {
            let clamped = Hsv { h, s, v }.clamped();
            proptest::prop_assert!((0.0..360.0).contains(&clamped.h));
            proptest::prop_assert!((0.0..=100.0).contains(&clamped.s));
            proptest::prop_assert!((0.0..=100.0).contains(&clamped.v));
            let hex = clamped.to_hex();
            proptest::prop_assert_eq!(hex.len(), 7);
        }
    }
}
