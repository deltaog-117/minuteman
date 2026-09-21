//! The hex view: what a binary file shows in the preview pane.
//!
//! Only the first [`HEAD_BYTES`] of a file are ever read, so selecting a multi-gigabyte binary
//! costs the same as selecting a small one. Rows are formatted on demand from those bytes rather
//! than up front, because how many bytes fit in a row depends on the pane's width, which changes
//! when the terminal is resized, and only the rows on screen need formatting at all.

use std::fs::File;
use std::io::Read;
use std::path::Path;

/// How much of a file the hex view (and the text-or-binary check) reads.
pub const HEAD_BYTES: usize = 64 * 1024;

/// The start of a file, and how big the whole file is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub bytes: Vec<u8>,
    /// The file's length on disk; larger than `bytes.len()` when the file was cut at
    /// [`HEAD_BYTES`].
    pub total: u64,
}

impl Head {
    /// Whether the file continues past what was read.
    pub fn is_cut(&self) -> bool {
        self.total > self.bytes.len() as u64
    }
}

/// Reads the first [`HEAD_BYTES`] of the regular file at `path`. `None` for anything that is not
/// a regular file — opening a named pipe for reading would block forever, and a device or socket
/// has no meaningful first bytes — or that cannot be read.
pub fn read_head(path: &Path) -> Option<Head> {
    // Checked before opening: `open` on a FIFO with no writer never returns.
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let file = File::open(path).ok()?;
    let total = file.metadata().ok().filter(|m| m.is_file())?.len();
    let mut bytes = Vec::new();
    file.take(HEAD_BYTES as u64).read_to_end(&mut bytes).ok()?;
    Some(Head { bytes, total })
}

/// Cells taken by the offset column: eight hex digits and two spaces.
pub const OFFSET_CELLS: usize = 10;

/// The width of a row showing `per_row` bytes: the offset, three cells per byte (`"4a "`), then
/// one ASCII cell per byte.
pub const fn row_width(per_row: usize) -> usize {
    OFFSET_CELLS + 4 * per_row
}

/// How many bytes to show per row in a pane `width` cells wide: 16 when that fits, then 8, then
/// 4. A pane narrower than four bytes' worth still gets four, and the row is cut at its edge.
pub fn bytes_per_row(width: usize) -> usize {
    [16, 8, 4]
        .into_iter()
        .find(|&per_row| row_width(per_row) <= width)
        .unwrap_or(4)
}

/// How many rows `len` bytes take at `per_row` bytes each.
pub fn row_count(len: usize, per_row: usize) -> usize {
    len.div_ceil(per_row.max(1))
}

/// Row number `row` of `bytes`, `per_row` bytes to a row: `00000010  48 65 6c 6c 6f 20 77 6f  Hello wo`.
/// The last row is padded so the ASCII column lines up. A row past the end is blank.
pub fn format_row(bytes: &[u8], row: usize, per_row: usize) -> String {
    let per_row = per_row.max(1);
    let start = row.saturating_mul(per_row).min(bytes.len());
    let chunk = &bytes[start..(start + per_row).min(bytes.len())];

    let mut out = format!("{start:08x}  ");
    for byte in chunk {
        out.push_str(&format!("{byte:02x} "));
    }
    for _ in chunk.len()..per_row {
        out.push_str("   ");
    }
    out.extend(chunk.iter().map(|&byte| {
        if (0x20..0x7f).contains(&byte) {
            byte as char
        } else {
            '.'
        }
    }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn scratch(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("minuteman-hex-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_row_shows_the_offset_the_bytes_and_their_printable_form() {
        let bytes = b"Hello, world\x00\x01\xff!";
        assert_eq!(
            format_row(bytes, 0, 16),
            "00000000  48 65 6c 6c 6f 2c 20 77 6f 72 6c 64 00 01 ff 21 Hello, world...!"
        );
    }

    #[test]
    fn the_last_row_is_padded_so_the_ascii_column_lines_up() {
        let bytes = b"0123456789abcdefXYZ";
        let last = format_row(bytes, 1, 16);
        assert_eq!(last, format!("00000010  58 59 5a {}XYZ", "   ".repeat(13)));
        assert_eq!(last.len(), row_width(16) - 13);
        assert_eq!(format_row(bytes, 0, 16).len(), row_width(16));
    }

    #[test]
    fn a_row_past_the_end_is_blank_rather_than_a_panic() {
        assert_eq!(format_row(b"ab", 5, 4).trim(), format!("{:08x}", 2));
        assert_eq!(format_row(b"", 0, 8).len(), row_width(8) - 8);
    }

    #[test]
    fn wider_panes_get_more_bytes_per_row_and_a_narrow_one_never_none() {
        assert_eq!(bytes_per_row(200), 16);
        assert_eq!(bytes_per_row(row_width(16)), 16);
        assert_eq!(bytes_per_row(row_width(16) - 1), 8);
        assert_eq!(bytes_per_row(row_width(8)), 8);
        assert_eq!(bytes_per_row(row_width(8) - 1), 4);
        assert_eq!(bytes_per_row(0), 4);
    }

    #[test]
    fn row_count_rounds_up_and_survives_a_zero_width() {
        assert_eq!(row_count(0, 16), 0);
        assert_eq!(row_count(16, 16), 1);
        assert_eq!(row_count(17, 16), 2);
        assert_eq!(row_count(5, 0), 5);
    }

    proptest! {
        /// Every byte appears exactly once across the rows, in order, whatever the row width;
        /// every full row has the same width (the last may be shorter, as its ASCII column has
        /// fewer bytes to show); and the ASCII column is printable.
        #[test]
        fn every_byte_appears_once_in_order_at_any_row_width(
            bytes in proptest::collection::vec(any::<u8>(), 0..300),
            per_row in prop_oneof![Just(4usize), Just(8), Just(16)],
        ) {
            let rows = row_count(bytes.len(), per_row);
            let mut seen = Vec::new();
            for row in 0..rows {
                let text = format_row(&bytes, row, per_row);
                let shown = per_row.min(bytes.len() - row * per_row);
                prop_assert_eq!(
                    text.len(),
                    OFFSET_CELLS + 3 * per_row + shown,
                    "row {} is ragged: {:?}", row, text
                );
                prop_assert!(text.is_ascii());
                let offset = usize::from_str_radix(&text[..8], 16).unwrap();
                prop_assert_eq!(offset, row * per_row);
                let hex = &text[OFFSET_CELLS..OFFSET_CELLS + 3 * per_row];
                for pair in hex.split_whitespace() {
                    seen.push(u8::from_str_radix(pair, 16).unwrap());
                }
                let ascii = &text[OFFSET_CELLS + 3 * per_row..];
                prop_assert!(ascii.chars().all(|c| (' '..='~').contains(&c)));
            }
            prop_assert_eq!(seen, bytes);
        }

        /// Any pane width gets a per-row count whose rows fit, unless even the narrowest does not.
        #[test]
        fn the_chosen_row_fits_the_pane_whenever_any_row_can(width in 0usize..300) {
            let per_row = bytes_per_row(width);
            prop_assert!(row_width(per_row) <= width || per_row == 4);
        }
    }

    #[test]
    fn a_file_is_read_up_to_the_cap_and_reports_its_full_size() {
        let dir = scratch("cap");
        let path = dir.join("big.bin");
        std::fs::write(&path, vec![7u8; HEAD_BYTES + 500]).unwrap();
        let head = read_head(&path).unwrap();
        assert_eq!(head.bytes.len(), HEAD_BYTES);
        assert_eq!(head.total, (HEAD_BYTES + 500) as u64);
        assert!(head.is_cut());

        std::fs::write(dir.join("small.bin"), b"abc").unwrap();
        let small = read_head(&dir.join("small.bin")).unwrap();
        assert_eq!(
            (small.bytes.as_slice(), small.total, small.is_cut()),
            (&b"abc"[..], 3, false)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_directory_or_a_missing_path_has_no_head() {
        let dir = scratch("nothead");
        assert!(read_head(&dir).is_none());
        assert!(read_head(&dir.join("nope")).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_named_pipe_is_refused_instead_of_blocking_forever() {
        let dir = scratch("fifo");
        let fifo = dir.join("pipe");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        assert!(
            made.is_ok_and(|s| s.success()),
            "mkfifo is needed for this test"
        );
        // With no writer, opening it for reading would hang the test rather than fail it.
        assert!(read_head(&fifo).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
