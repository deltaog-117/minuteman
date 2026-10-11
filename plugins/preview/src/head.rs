//! The start of a binary file, for the hex view.
//!
//! Only the first [`HEAD_BYTES`] of a file are ever read, so selecting a multi-gigabyte binary
//! costs the same as selecting a small one. How those bytes are laid out in rows depends on the
//! width of the pane, which only the interface knows, so it formats the rows itself.

use std::io::Read;
use std::path::Path;

use plugin_api::{HexHead, MAX_HEX_BYTES};
use shared::{FileKind, Vfs};

/// How much of a file the hex view (and the text-or-binary check) reads.
pub const HEAD_BYTES: usize = MAX_HEX_BYTES;

/// Reads the first [`HEAD_BYTES`] of the regular file at `path`. `None` for anything that is not
/// a regular file — opening a named pipe for reading would block forever, and a device or socket
/// has no meaningful first bytes — or that cannot be read.
pub fn read_head(vfs: &dyn Vfs, path: &Path) -> Option<HexHead> {
    // Checked before opening: `open` on a FIFO with no writer never returns.
    let meta = vfs
        .metadata(path)
        .ok()
        .filter(|m| m.kind == FileKind::File)?;
    let mut bytes = Vec::new();
    vfs.open_read(path)
        .ok()?
        .take(HEAD_BYTES as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(HexHead {
        bytes,
        total: meta.len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("minuteman-head-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_file_is_read_up_to_the_cap_and_reports_its_full_size() {
        let dir = scratch("cap");
        let path = dir.join("big.bin");
        std::fs::write(&path, vec![7u8; HEAD_BYTES + 500]).unwrap();
        let head = read_head(&shared::LocalVfs, &path).unwrap();
        assert_eq!(head.bytes.len(), HEAD_BYTES);
        assert_eq!(head.total, (HEAD_BYTES + 500) as u64);
        assert!(head.is_cut());

        std::fs::write(dir.join("small.bin"), b"abc").unwrap();
        let small = read_head(&shared::LocalVfs, &dir.join("small.bin")).unwrap();
        assert_eq!(
            (small.bytes.as_slice(), small.total, small.is_cut()),
            (&b"abc"[..], 3, false)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_directory_or_a_missing_path_has_no_head() {
        let dir = scratch("nothead");
        assert!(read_head(&shared::LocalVfs, &dir).is_none());
        assert!(read_head(&shared::LocalVfs, &dir.join("nope")).is_none());
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
        assert!(read_head(&shared::LocalVfs, &fifo).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
