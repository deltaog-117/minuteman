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

//! Asks the terminal for its own colors — foreground and background (OSC 10/11) and the 16 ANSI
//! slots (OSC 4) — so the default theme can match whatever the user's wallpaper or theming tool
//! pushed into it. Terminals that don't answer simply yield `None`; nothing here may block
//! startup for longer than [`TOTAL_TIMEOUT`].

use theming::TerminalPalette;

type Rgb = (u8, u8, u8);

/// Parses an X11 color spec as terminals report it: `rgb:R/G/B` with 1–4 hex digits a channel,
/// scaled to 8 bits.
fn parse_color(spec: &str) -> Option<Rgb> {
    let mut parts = spec.strip_prefix("rgb:")?.split('/');
    let mut channel = || {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 4 || !part.is_ascii() {
            return None;
        }
        let value = u32::from_str_radix(part, 16).ok()?;
        let max = (1u32 << (4 * part.len())) - 1;
        u8::try_from((value * 255 + max / 2) / max).ok()
    };
    let color = (channel()?, channel()?, channel()?);
    parts.next().is_none().then_some(color)
}

/// Collects every OSC 4/10/11 color reply found in `bytes` — the garbage between them (the
/// device-attributes sentinel, stray input) is skipped. `None` unless the background and all 16
/// ANSI slots were answered; the foreground is optional.
fn parse_replies(bytes: &[u8]) -> Option<TerminalPalette> {
    let text = String::from_utf8_lossy(bytes);
    let mut background = None;
    let mut foreground = None;
    let mut ansi: [Option<Rgb>; 16] = [None; 16];

    for reply in text.split("\x1b]").skip(1) {
        let body = reply.split(['\x07', '\x1b']).next().unwrap_or("");
        let mut fields = body.splitn(3, ';');
        match (fields.next(), fields.next(), fields.next()) {
            (Some("10"), Some(spec), None) => foreground = parse_color(spec),
            (Some("11"), Some(spec), None) => background = parse_color(spec),
            (Some("4"), Some(index), Some(spec)) => {
                if let (Ok(index), Some(color)) = (index.parse::<usize>(), parse_color(spec))
                    && let Some(slot) = ansi.get_mut(index)
                {
                    *slot = Some(color);
                }
            }
            _ => {}
        }
    }

    let mut resolved = [(0, 0, 0); 16];
    for (out, color) in resolved.iter_mut().zip(ansi) {
        *out = color?;
    }
    Some(TerminalPalette {
        background: background?,
        foreground,
        ansi: resolved,
    })
}

/// The device-attributes reply (`ESC [ ? … c`) every terminal sends, and in order, so seeing it
/// means all earlier queries were answered or ignored and waiting longer is pointless.
fn sentinel_seen(bytes: &[u8]) -> bool {
    bytes
        .windows(3)
        .enumerate()
        .any(|(i, w)| w == b"\x1b[?" && bytes[i..].contains(&b'c'))
}

#[cfg(unix)]
pub fn query() -> Option<TerminalPalette> {
    use std::io::Write;
    use std::time::{Duration, Instant};

    const TOTAL_TIMEOUT: Duration = Duration::from_millis(300);

    // SAFETY: `isatty` only inspects the descriptor.
    let on_tty = unsafe { libc::isatty(libc::STDIN_FILENO) == 1 };
    if !on_tty {
        return None;
    }

    let mut request = String::from("\x1b]10;?\x1b\\\x1b]11;?\x1b\\");
    for slot in 0..16 {
        request.push_str(&format!("\x1b]4;{slot};?\x1b\\"));
    }
    request.push_str("\x1b[c");
    let mut stdout = std::io::stdout();
    stdout.write_all(request.as_bytes()).ok()?;
    stdout.flush().ok()?;

    let deadline = Instant::now() + TOTAL_TIMEOUT;
    let mut collected = Vec::new();
    let mut buffer = [0u8; 1024];
    while !sentinel_seen(&collected) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let mut poll = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
        // SAFETY: `poll` points at one valid `pollfd`, matching the count of 1.
        if unsafe { libc::poll(&mut poll, 1, millis) } <= 0 {
            break;
        }
        // SAFETY: `buffer` is valid for `buffer.len()` writable bytes.
        let read =
            unsafe { libc::read(libc::STDIN_FILENO, buffer.as_mut_ptr().cast(), buffer.len()) };
        if read <= 0 {
            break;
        }
        collected.extend_from_slice(&buffer[..read as usize]);
    }
    parse_replies(&collected)
}

#[cfg(not(unix))]
pub fn query() -> Option<TerminalPalette> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_reply() -> String {
        let mut reply =
            String::from("\x1b]10;rgb:dfdf/dada/d1d1\x1b\\\x1b]11;rgb:0f0f/1010/1212\x07");
        for slot in 0..16 {
            reply.push_str(&format!("\x1b]4;{slot};rgb:{slot:02x}/80/ff\x1b\\"));
        }
        reply.push_str("\x1b[?62;c");
        reply
    }

    #[test]
    fn one_two_and_four_digit_channels_scale_to_eight_bits() {
        assert_eq!(parse_color("rgb:f/0/8"), Some((255, 0, 136)));
        assert_eq!(parse_color("rgb:ff/00/80"), Some((255, 0, 128)));
        assert_eq!(parse_color("rgb:ffff/0000/8080"), Some((255, 0, 128)));
    }

    #[test]
    fn malformed_specs_do_not_parse() {
        for bad in [
            "",
            "rgb:",
            "rgb:ff/ff",
            "rgb:ff/ff/ff/ff",
            "rgb:gg/00/00",
            "#ffffff",
            "rgb:fffff/0/0",
        ] {
            assert_eq!(parse_color(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_complete_reply_yields_the_whole_palette() {
        let palette = parse_replies(full_reply().as_bytes()).expect("complete reply");
        assert_eq!(palette.background, (0x0f, 0x10, 0x12));
        assert_eq!(palette.foreground, Some((0xdf, 0xda, 0xd1)));
        assert_eq!(palette.ansi[3], (3, 128, 255));
    }

    #[test]
    fn a_missing_foreground_is_tolerated() {
        let reply = full_reply().replacen("\x1b]10;rgb:dfdf/dada/d1d1\x1b\\", "", 1);
        let palette = parse_replies(reply.as_bytes()).expect("background and ansi answered");
        assert_eq!(palette.foreground, None);
    }

    #[test]
    fn a_missing_background_or_ansi_slot_yields_none() {
        let no_background = full_reply().replacen("\x1b]11;rgb:0f0f/1010/1212\x07", "", 1);
        assert_eq!(parse_replies(no_background.as_bytes()), None);
        let no_slot = full_reply().replacen("\x1b]4;7;rgb:07/80/ff\x1b\\", "", 1);
        assert_eq!(parse_replies(no_slot.as_bytes()), None);
    }

    #[test]
    fn silence_and_garbage_yield_none() {
        assert_eq!(parse_replies(b""), None);
        assert_eq!(parse_replies(b"\x1b[?62;c"), None);
        assert_eq!(parse_replies(b"\x1b]4;99;rgb:00/00/00\x07\x1b]4;x"), None);
    }

    #[test]
    fn the_sentinel_is_only_a_complete_device_attributes_reply() {
        assert!(sentinel_seen(b"\x1b]11;rgb:00/00/00\x07\x1b[?62;4c"));
        assert!(!sentinel_seen(b"\x1b[?62;4"));
        assert!(!sentinel_seen(b"\x1b]11;rgb:00/00/00\x07"));
    }
}
