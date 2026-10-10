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

//! `file://` URIs for the paths the portal replies with.

use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// `file://` followed by `path`, percent-encoded byte by byte. Only the unreserved characters of
/// RFC 3986 and the `/` between components stay as they are, so a name with a space, a `#`, a `%`
/// or bytes that are not UTF-8 reads back as exactly the same path.
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(char::from(byte));
            }
            other => uri.push_str(&format!("%{other:02X}")),
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::ffi::OsStr;

    /// The inverse, for the tests only: nothing in the daemon reads a URI.
    fn decode(uri: &str) -> Option<Vec<u8>> {
        let rest = uri.strip_prefix("file://")?.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            if rest[i] == b'%' {
                let hex = std::str::from_utf8(rest.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            } else {
                out.push(rest[i]);
                i += 1;
            }
        }
        Some(out)
    }

    #[test]
    fn plain_paths_are_left_alone_and_odd_bytes_are_escaped() {
        assert_eq!(file_uri(Path::new("/home/u/a.txt")), "file:///home/u/a.txt");
        assert_eq!(file_uri(Path::new("/a b/#c%d")), "file:///a%20b/%23c%25d");
        assert_eq!(file_uri(Path::new("/é")), "file:///%C3%A9");
    }

    proptest! {
        /// The URI always reads back as the same bytes, and holds nothing but URI characters.
        #[test]
        fn a_uri_round_trips_any_path(raw in proptest::collection::vec(1u8..=255, 0..40)) {
            let path = Path::new("/").join(OsStr::from_bytes(&raw));
            let uri = file_uri(&path);
            prop_assert!(uri.bytes().all(|b| b.is_ascii_graphic()));
            prop_assert_eq!(decode(&uri), Some(path.as_os_str().as_bytes().to_vec()));
        }
    }
}
