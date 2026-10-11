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

//! What a plugin that previews files exchanges with the host. The host tells it which file the
//! preview column is about (the `preview` event); the plugin reads the file through the host
//! (`read_file`, so a browsed ssh folder or an archive previews like the local disk does) and
//! answers with a finished document (`set_preview`): colored text, the first bytes of a binary
//! file, or an archive's listing. The plugin never draws. The interface keeps the scrolling,
//! the wrapping, the scrollbar and the colors, so moving through a long file stays instant and
//! does not wait on the plugin.
//!
//! A plugin is a separate program that may be wrong or hostile, and its text ends up in the
//! user's terminal, so nothing in a document is trusted until [`SetPreview::checked`] has cleaned
//! it: control and direction-changing characters are removed from text and replaced in names, and
//! size limits are applied. The host calls it on every `set_preview` it reads.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use shared::names::{clean_name, is_unsafe as is_unsafe_in_name};

use crate::view::is_unsafe;

/// The notification that gives the preview column its document: `set_preview`.
pub const METHOD_SET_PREVIEW: &str = "set_preview";
/// The request that reads part of a file through the host: `read_file`.
pub const METHOD_READ_FILE: &str = "read_file";
/// The event name a previewing plugin gets when the file the preview column shows changes, or
/// that file changed on disk and wants reading again.
pub const EVENT_PREVIEW: &str = "preview";
/// What a plugin lists under `provides` to be the one that previews files.
pub const PROVIDES_PREVIEW: &str = "preview";

/// Most bytes one `read_file` returns, however many are asked for.
pub const MAX_READ_BYTES: u64 = 4 << 20;
/// Most characters of text a document holds: what a text file may be previewed at.
pub const MAX_TEXT_CHARS: usize = 1 << 20;
/// Most bytes of a binary file a document holds.
pub const MAX_HEX_BYTES: usize = 64 * 1024;
/// Most entries of an archive a document lists.
pub const MAX_ARCHIVE_ENTRIES: usize = 5_000;

/// The file the preview column is about, in a `preview` event: `null` when it shows no file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewTarget {
    pub path: Option<PathBuf>,
}

/// How a piece of source text should be colored, in the interface's own palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Token {
    Plain,
    Keyword,
    String,
    Comment,
    Number,
    Function,
    Type,
}

/// A run of text of one kind. On the wire it is `["keyword","fn"]`: a document of a megabyte
/// holds hundreds of thousands of them, and field names would double its size.
pub type Piece = (Token, String);

/// The pieces of one source line, in order; joined they are the line without its newline.
pub type PreviewLine = Vec<Piece>;

/// The start of a file, for the hex view, and how big the whole file is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HexHead {
    #[serde(with = "hex_string")]
    pub bytes: Vec<u8>,
    /// The file's length; larger than `bytes.len()` when the file was cut.
    pub total: u64,
}

impl HexHead {
    /// Whether the file continues past what is held.
    pub fn is_cut(&self) -> bool {
        self.total > self.bytes.len() as u64
    }
}

/// The archive formats that can be listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveKind {
    Zip,
    Tar,
    TarGz,
}

impl ArchiveKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Tar => "tar",
            Self::TarGz => "tar.gz",
        }
    }
}

/// One thing in an archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveEntry {
    pub name: String,
    /// Unpacked size in bytes; zero for a directory.
    pub size: u64,
    pub is_dir: bool,
}

/// What an archive listing leaves out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hidden {
    /// Everything was listed.
    Nothing,
    /// This many more entries exist (a zip knows its own count).
    Count(usize),
    /// There are more, but how many is not known (a tar has no index, so counting means reading
    /// it all, or reading was cut short by an error or a limit).
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveListing {
    pub kind: ArchiveKind,
    pub entries: Vec<ArchiveEntry>,
    pub hidden: Hidden,
}

impl ArchiveListing {
    /// What the listed files add up to unpacked.
    pub fn unpacked_bytes(&self) -> u64 {
        self.entries
            .iter()
            .fold(0u64, |sum, entry| sum.saturating_add(entry.size))
    }
}

/// What the preview column shows for one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// Tagged `type`, not `kind`: an archive listing has a `kind` of its own (zip, tar) that would
// otherwise collide with the tag when the two are flattened together.
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PreviewContent {
    /// Nothing to show: not a regular file, or one the plugin has no preview for. The interface
    /// shows the file's name instead.
    Empty,
    /// The file should have a preview but could not be read, or is too big for one.
    Failed,
    Text {
        lines: Vec<PreviewLine>,
    },
    Hex(HexHead),
    Archive(ArchiveListing),
}

/// The parameters of `set_preview`: the document for the file at `path`. The interface keeps it
/// only while `path` is still what the column shows, so a slow answer for a file the cursor has
/// left is dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetPreview {
    pub path: PathBuf,
    pub content: PreviewContent,
}

/// Why a document was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewError {
    TooMuchText,
    TooManyBytes,
    TooManyEntries,
}

impl fmt::Display for PreviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooMuchText => write!(f, "a preview holds at most {MAX_TEXT_CHARS} characters"),
            Self::TooManyBytes => write!(f, "a preview holds at most {MAX_HEX_BYTES} bytes"),
            Self::TooManyEntries => {
                write!(f, "a preview lists at most {MAX_ARCHIVE_ENTRIES} entries")
            }
        }
    }
}

impl std::error::Error for PreviewError {}

impl SetPreview {
    /// This document with its text cleaned, or why it cannot be shown. Cleaning goes first, so the
    /// limits count what would be drawn.
    ///
    /// # Errors
    ///
    /// A text of more than [`MAX_TEXT_CHARS`] characters (or that many lines or pieces, which
    /// cannot be fewer than its characters), more than [`MAX_HEX_BYTES`] bytes of a file, or more
    /// than [`MAX_ARCHIVE_ENTRIES`] archive entries.
    pub fn checked(mut self) -> Result<Self, PreviewError> {
        match &mut self.content {
            PreviewContent::Empty | PreviewContent::Failed => {}
            PreviewContent::Text { lines } => {
                // A line break is the end of a line, never a character inside one, and the
                // line count is bounded by the same budget as the text: an empty line is cheap
                // on the wire but each one is a row the interface must lay out.
                if lines.len() > MAX_TEXT_CHARS {
                    return Err(PreviewError::TooMuchText);
                }
                let mut chars = 0usize;
                for line in lines.iter_mut() {
                    for (_, text) in line.iter_mut() {
                        text.retain(|c| !is_unsafe(c));
                        chars = chars.saturating_add(text.chars().count());
                    }
                    line.retain(|(_, text)| !text.is_empty());
                    if chars > MAX_TEXT_CHARS {
                        return Err(PreviewError::TooMuchText);
                    }
                }
            }
            PreviewContent::Hex(head) => {
                if head.bytes.len() > MAX_HEX_BYTES {
                    return Err(PreviewError::TooManyBytes);
                }
            }
            PreviewContent::Archive(listing) => {
                if listing.entries.len() > MAX_ARCHIVE_ENTRIES {
                    return Err(PreviewError::TooManyEntries);
                }
                for entry in &mut listing.entries {
                    // `clean_name` replaces rather than removes, so a name stays recognisable.
                    entry.name = clean_name(&entry.name);
                    debug_assert!(entry.name.chars().all(|c| !is_unsafe_in_name(c)));
                }
            }
        }
        Ok(self)
    }
}

/// What kind of thing a `read_file` found at its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkKind {
    File,
    Dir,
    /// A socket, a pipe or a device: nothing is read from it, since opening a pipe with no writer
    /// never returns.
    Other,
}

/// The parameters of `read_file`: up to `len` bytes of the file at `path`, from `offset`. With a
/// `len` of zero it reads nothing and only describes the path, which is how a plugin stats it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadFile {
    pub path: PathBuf,
    #[serde(default)]
    pub offset: u64,
    pub len: u64,
}

/// What `read_file` answers with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChunk {
    pub kind: ChunkKind,
    /// The file's whole length.
    pub size: u64,
    /// The same file on this machine's own disk, when the backend is the local disk: what a
    /// program the plugin starts can be handed. Absent for any other backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<PathBuf>,
    /// The bytes read: fewer than asked at the end of the file, and none for anything that is
    /// not a regular file.
    #[serde(with = "hex_string")]
    pub data: Vec<u8>,
}

/// Bytes as a lowercase hexadecimal string, the one way the wire carries them (JSON has no byte
/// string, and an array of numbers is longer).
pub mod hex_string {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    const DIGITS: &[u8; 16] = b"0123456789abcdef";

    pub fn encode(bytes: &[u8]) -> String {
        let mut text = String::with_capacity(bytes.len() * 2);
        for &byte in bytes {
            text.push(DIGITS[usize::from(byte >> 4)] as char);
            text.push(DIGITS[usize::from(byte & 0x0f)] as char);
        }
        text
    }

    /// The bytes `text` spells, or `None` for an odd length or a character that is not a digit.
    pub fn decode(text: &str) -> Option<Vec<u8>> {
        let digits = text.as_bytes();
        if !digits.len().is_multiple_of(2) {
            return None;
        }
        let value = |digit: u8| match digit {
            b'0'..=b'9' => Some(digit - b'0'),
            b'a'..=b'f' => Some(digit - b'a' + 10),
            b'A'..=b'F' => Some(digit - b'A' + 10),
            _ => None,
        };
        digits
            .chunks_exact(2)
            .map(|pair| Some(value(pair[0])? << 4 | value(pair[1])?))
            .collect()
    }

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        decode(&text).ok_or_else(|| D::Error::custom("not a hexadecimal byte string"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn text_document(lines: Vec<PreviewLine>) -> SetPreview {
        SetPreview {
            path: PathBuf::from("/a.rs"),
            content: PreviewContent::Text { lines },
        }
    }

    proptest! {
        /// Whatever the bytes, they come back from the string that spells them.
        #[test]
        fn hex_strings_round_trip(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
            let text = hex_string::encode(&bytes);
            prop_assert_eq!(text.len(), bytes.len() * 2);
            prop_assert_eq!(hex_string::decode(&text), Some(bytes));
        }

        /// A document the host accepted holds no character that could reach the terminal as a
        /// command, whatever a plugin sent.
        #[test]
        fn a_checked_text_has_nothing_unsafe_in_it(
            pieces in proptest::collection::vec(any::<String>(), 0..8),
        ) {
            let line: PreviewLine = pieces.into_iter().map(|text| (Token::Plain, text)).collect();
            if let Ok(set) = text_document(vec![line]).checked()
                && let PreviewContent::Text { lines } = set.content
            {
                for (_, text) in lines.iter().flatten() {
                    prop_assert!(text.chars().all(|c| !is_unsafe(c)));
                    prop_assert!(!text.is_empty());
                }
            }
        }

        /// Cleaned archive names are safe to draw and bounded, however hostile.
        #[test]
        fn a_checked_archive_name_is_safe(raw in any::<String>()) {
            let set = SetPreview {
                path: PathBuf::from("/a.zip"),
                content: PreviewContent::Archive(ArchiveListing {
                    kind: ArchiveKind::Zip,
                    entries: vec![ArchiveEntry { name: raw, size: 1, is_dir: false }],
                    hidden: Hidden::Nothing,
                }),
            };
            let checked = set.checked().expect("one entry is within the limit");
            let PreviewContent::Archive(listing) = checked.content else { unreachable!() };
            prop_assert!(listing.entries[0].name.chars().all(|c| !is_unsafe_in_name(c)));
        }
    }

    #[test]
    fn the_wire_shape_is_the_documented_one() {
        let set = text_document(vec![vec![
            (Token::Keyword, "fn".into()),
            (Token::Plain, " main".into()),
        ]]);
        assert_eq!(
            serde_json::to_string(&set).expect("plain data serializes"),
            r#"{"path":"/a.rs","content":{"type":"text","lines":[[["keyword","fn"],["plain"," main"]]]}}"#
        );
        let hex = SetPreview {
            path: PathBuf::from("/b"),
            content: PreviewContent::Hex(HexHead {
                bytes: vec![0, 255, 16],
                total: 9,
            }),
        };
        assert_eq!(
            serde_json::to_string(&hex).expect("plain data serializes"),
            r#"{"path":"/b","content":{"type":"hex","bytes":"00ff10","total":9}}"#
        );
        let none = SetPreview {
            path: PathBuf::from("/c"),
            content: PreviewContent::Empty,
        };
        assert_eq!(
            serde_json::to_string(&none).expect("plain data serializes"),
            r#"{"path":"/c","content":{"type":"empty"}}"#
        );
    }

    #[test]
    fn every_kind_of_document_survives_the_wire() {
        let documents = [
            PreviewContent::Empty,
            PreviewContent::Failed,
            PreviewContent::Text {
                lines: vec![vec![(Token::Type, "Vec".into())], Vec::new()],
            },
            PreviewContent::Hex(HexHead {
                bytes: vec![1, 2, 3],
                total: 3,
            }),
            PreviewContent::Archive(ArchiveListing {
                kind: ArchiveKind::TarGz,
                entries: vec![ArchiveEntry {
                    name: "a/b".into(),
                    size: 7,
                    is_dir: false,
                }],
                hidden: Hidden::Count(4),
            }),
            PreviewContent::Archive(ArchiveListing {
                kind: ArchiveKind::Zip,
                entries: Vec::new(),
                hidden: Hidden::Unknown,
            }),
        ];
        for content in documents {
            let set = SetPreview {
                path: PathBuf::from("/x"),
                content,
            };
            let json = serde_json::to_string(&set).expect("plain data serializes");
            let back: SetPreview = serde_json::from_str(&json).expect("and reads back");
            assert_eq!(back, set, "{json}");
        }
    }

    #[test]
    fn an_escape_sequence_in_text_is_removed_and_empty_pieces_dropped() {
        let set = text_document(vec![vec![
            (Token::Plain, "a\u{1b}[2Jb".into()),
            (Token::Comment, "\u{202e}".into()),
        ]])
        .checked()
        .expect("small");
        let PreviewContent::Text { lines } = set.content else {
            unreachable!()
        };
        assert_eq!(lines, vec![vec![(Token::Plain, "a[2Jb".to_string())]]);
    }

    #[test]
    fn a_document_over_a_limit_is_refused() {
        let big = text_document(vec![vec![(Token::Plain, "x".repeat(MAX_TEXT_CHARS + 1))]]);
        assert_eq!(big.checked().unwrap_err(), PreviewError::TooMuchText);

        let many_lines = text_document(vec![Vec::new(); MAX_TEXT_CHARS + 1]);
        assert_eq!(many_lines.checked().unwrap_err(), PreviewError::TooMuchText);

        let hex = SetPreview {
            path: PathBuf::from("/b"),
            content: PreviewContent::Hex(HexHead {
                bytes: vec![0; MAX_HEX_BYTES + 1],
                total: 0,
            }),
        };
        assert_eq!(hex.checked().unwrap_err(), PreviewError::TooManyBytes);

        let archive = SetPreview {
            path: PathBuf::from("/a.zip"),
            content: PreviewContent::Archive(ArchiveListing {
                kind: ArchiveKind::Tar,
                entries: vec![
                    ArchiveEntry {
                        name: "a".into(),
                        size: 0,
                        is_dir: false
                    };
                    MAX_ARCHIVE_ENTRIES + 1
                ],
                hidden: Hidden::Nothing,
            }),
        };
        assert_eq!(archive.checked().unwrap_err(), PreviewError::TooManyEntries);
    }

    #[test]
    fn a_document_exactly_at_the_limit_is_accepted() {
        let at_limit = text_document(vec![vec![(Token::Plain, "x".repeat(MAX_TEXT_CHARS))]]);
        assert!(at_limit.checked().is_ok());
    }

    #[test]
    fn a_read_file_reply_carries_its_bytes_as_hex_and_refuses_bad_hex() {
        let chunk = FileChunk {
            kind: ChunkKind::File,
            size: 3,
            local_path: None,
            data: b"abc".to_vec(),
        };
        let json = serde_json::to_string(&chunk).expect("plain data serializes");
        assert_eq!(json, r#"{"kind":"file","size":3,"data":"616263"}"#);
        assert_eq!(serde_json::from_str::<FileChunk>(&json).unwrap(), chunk);
        assert!(
            serde_json::from_str::<FileChunk>(r#"{"kind":"file","size":3,"data":"6g"}"#).is_err()
        );
        assert!(
            serde_json::from_str::<FileChunk>(r#"{"kind":"file","size":3,"data":"616"}"#).is_err()
        );
    }
}
