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

//! The name of one history entry, the same in every session and in the saved file.

use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Orders by creation time first, so merging two sessions' entries puts them in the order they
/// were made; the process id and the counter only keep two ids made in one millisecond apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryId {
    millis: u64,
    process: u32,
    count: u32,
}

impl EntryId {
    /// An id no other call, in this process or another, has returned.
    pub fn fresh() -> Self {
        static COUNT: AtomicU32 = AtomicU32::new(0);
        Self {
            // A clock before 1970 gives 0, which only sorts the entry first.
            millis: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| {
                    u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
                }),
            process: std::process::id(),
            count: COUNT.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// The id of the `index`th entry of a file saved before entries had ids. It depends only on
    /// the position, so every session reading the same old file names the same entry alike.
    pub(super) fn legacy(index: usize) -> Self {
        Self {
            millis: 0,
            process: 0,
            count: u32::try_from(index).unwrap_or(u32::MAX),
        }
    }
}

impl fmt::Display for EntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:x}-{:x}-{:x}", self.millis, self.process, self.count)
    }
}

/// Text that is not an id written by `Display`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not an entry id")]
pub struct ParseIdError;

impl FromStr for EntryId {
    type Err = ParseIdError;

    fn from_str(text: &str) -> Result<Self, ParseIdError> {
        let mut parts = text.split('-');
        let (Some(millis), Some(process), Some(count), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(ParseIdError);
        };
        Ok(Self {
            millis: hex(millis)?,
            process: u32::try_from(hex(process)?).map_err(|_| ParseIdError)?,
            count: u32::try_from(hex(count)?).map_err(|_| ParseIdError)?,
        })
    }
}

/// Plain hexadecimal digits only: `from_str_radix` alone would also take a leading `+`, which
/// `Display` never writes.
fn hex(part: &str) -> Result<u64, ParseIdError> {
    if part.is_empty() || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ParseIdError);
    }
    u64::from_str_radix(part, 16).map_err(|_| ParseIdError)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn fresh_ids_are_all_different() {
        let ids: std::collections::HashSet<_> = (0..1000).map(|_| EntryId::fresh()).collect();
        assert_eq!(ids.len(), 1000);
    }

    #[test]
    fn a_later_id_sorts_after_an_earlier_one() {
        let first = EntryId::fresh();
        let second = EntryId::fresh();
        assert!(first < second);
    }

    #[test]
    fn text_that_is_not_an_id_is_refused() {
        for text in [
            "",
            "1-2",
            "1-2-3-4",
            "x-1-2",
            "+1-2-3",
            "1--3",
            "1-2-100000000",
        ] {
            assert_eq!(text.parse::<EntryId>(), Err(ParseIdError), "{text}");
        }
    }

    proptest! {
        #[test]
        fn display_then_parse_gives_back_the_id(millis: u64, process: u32, count: u32) {
            let id = EntryId { millis, process, count };
            prop_assert_eq!(id.to_string().parse::<EntryId>(), Ok(id));
        }

        #[test]
        fn parsing_never_panics(text in "\\PC{0,40}") {
            let _ = text.parse::<EntryId>();
        }
    }
}
