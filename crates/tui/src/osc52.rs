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

//! Putting text on the system clipboard through the terminal. OSC 52 asks the terminal emulator
//! itself to set the clipboard, which is the one route that works over `ssh` and inside `tmux`
//! without a clipboard tool installed on the far side. Not every terminal honours it, so the
//! caller says "sent", never "copied".

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding (RFC 4648), which is what OSC 52 carries its payload in.
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let [a, b, c] = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let word = (u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            // A chunk of n bytes fills n + 1 sextets; the rest is padding.
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(word >> shift) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The escape sequence that sets the terminal's clipboard to `text`.
pub fn set_clipboard(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn the_rfc_4648_vectors_encode_exactly() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), encoded);
        }
    }

    #[test]
    fn the_sequence_wraps_the_payload_in_osc_52() {
        assert_eq!(set_clipboard("foo"), "\x1b]52;c;Zm9v\x07");
    }

    proptest! {
        #[test]
        fn the_output_is_padded_to_four_and_uses_only_the_alphabet(
            bytes in proptest::collection::vec(any::<u8>(), 0..200)
        ) {
            let encoded = base64(&bytes);
            prop_assert_eq!(encoded.len(), bytes.len().div_ceil(3) * 4);
            prop_assert!(encoded.bytes().all(|b| ALPHABET.contains(&b) || b == b'='));
        }
    }
}
