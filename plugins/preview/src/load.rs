//! Decides what the preview column shows for a file and reads it, which can take a while on a slow
//! disk or a far-away host: the plugin runs it on a thread of its own and drops the answer if the
//! cursor has moved on.
//!
//! In order: a file a configured hook covers is shown as what the hook extracted; an archive (by
//! name) is listed; a file that is text, by its name or its content, is read whole up to 1 MiB and
//! colored; everything else is shown as its first bytes. A text-named file over the size cap, or
//! one that cannot be read, is a failure the interface says so about. Pictures are the
//! interface's own business, so they come back as nothing to show.

use std::io::Read;
use std::path::Path;

use plugin_api::{ArchiveListing, HexHead, MAX_TEXT_CHARS, PreviewContent};
use shared::{FileKind, Vfs, is_image, is_text};

use crate::archive;
use crate::head::read_head;
use crate::highlight;
use crate::hook::{self, Hook};

/// Caps how much of a file [`load_text`] will read into memory for a preview — large enough for
/// any normal source/config file, small enough that a huge log or data file can't stall the
/// read or bloat the document.
pub const MAX_TEXT_PREVIEW_BYTES: u64 = MAX_TEXT_CHARS as u64;

/// What a file turned out to be, before any coloring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    Text(String),
    /// A binary file's first bytes, for the hex view.
    Bytes(HexHead),
    Archive(ArchiveListing),
    /// The file should have a preview but could not be read, or is too big for one.
    Failed,
    /// Not a regular file (a socket, a device, a named pipe): nothing to show.
    Unsupported,
}

/// Reads `path` as UTF-8 text. Returns `None` if the file exceeds [`MAX_TEXT_PREVIEW_BYTES`],
/// contains a null byte (a binary file mislabeled with a text-like name), or isn't valid UTF-8.
pub fn load_text(vfs: &dyn Vfs, path: &Path) -> Option<String> {
    if vfs.metadata(path).ok()?.len > MAX_TEXT_PREVIEW_BYTES {
        return None;
    }
    // Read one byte past the cap, so a file that grew since the length was taken is still refused
    // rather than read whole.
    let mut bytes = Vec::new();
    vfs.open_read(path)
        .ok()?
        .take(MAX_TEXT_PREVIEW_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_TEXT_PREVIEW_BYTES || bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Whether `head`, the start of a file, looks like text: no null byte and valid UTF-8 (a
/// multi-byte character cut by the end of `head` does not count against it).
pub fn looks_like_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        // `error_len() == None` is a character that was merely cut off at the end.
        Err(error) => error.error_len().is_none(),
    }
}

/// Decides what to show for the regular file at `path` and reads it.
pub fn load(vfs: &dyn Vfs, path: &Path) -> Loaded {
    let Some(head) = read_head(vfs, path) else {
        // `read_head` refuses anything that is not a regular file, and a regular file it could
        // not read is a failure worth saying so; tell the two apart by asking again.
        return match vfs.metadata(path) {
            Ok(meta) if meta.kind == FileKind::File => Loaded::Failed,
            _ => Loaded::Unsupported,
        };
    };
    if let Some(listing) = archive::list(vfs, path) {
        return Loaded::Archive(listing);
    }
    let named_text = is_text(path);
    if named_text && head.total > MAX_TEXT_PREVIEW_BYTES {
        return Loaded::Failed;
    }
    if head.total <= MAX_TEXT_PREVIEW_BYTES
        && (named_text || looks_like_text(&head.bytes))
        && let Some(text) = load_text(vfs, path)
    {
        return Loaded::Text(text);
    }
    Loaded::Bytes(head)
}

/// The document for the file at `path`, or `None` when `stop` said the answer is no longer wanted.
/// `hooks` are the configured text-extracting commands; one that covers `path` is used when the
/// file is on this machine's disk, since it hands the file to another program.
pub fn content(
    vfs: &dyn Vfs,
    path: &Path,
    hooks: &[Hook],
    stop: &dyn Fn() -> bool,
) -> Option<PreviewContent> {
    if is_image(path) {
        return Some(PreviewContent::Empty);
    }
    let covered = hook::hook_for(hooks, path).zip(vfs.local_path(path));
    let loaded = match covered {
        Some((hook, local)) => match hook::run(hook, &local) {
            Some(text) => Loaded::Text(text),
            // A hook that fails, times out or is missing falls back to just the file's name, like
            // a file with no preview, not to the "preview failed" a real read failure gives.
            None => Loaded::Unsupported,
        },
        None => load(vfs, path),
    };
    if stop() {
        return None;
    }
    Some(match loaded {
        Loaded::Text(text) => {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Tokenizing runs syntect's line-oriented parser over the whole file, the same amount
            // of work as the read just before it.
            PreviewContent::Text {
                lines: highlight::highlight_until(&text, name, stop)?,
            }
        }
        Loaded::Bytes(head) => PreviewContent::Hex(head),
        Loaded::Archive(listing) => PreviewContent::Archive(listing),
        Loaded::Failed => PreviewContent::Failed,
        Loaded::Unsupported => PreviewContent::Empty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugin_api::Token;
    use shared::MemVfs;

    /// The kind of every piece of a text document, in order.
    fn kinds(content: &PreviewContent) -> Vec<Token> {
        match content {
            PreviewContent::Text { lines } => {
                lines.iter().flatten().map(|(kind, _)| *kind).collect()
            }
            _ => Vec::new(),
        }
    }

    fn scratch_dir(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("minuteman-load-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn never() -> bool {
        false
    }

    #[test]
    fn load_text_reads_a_real_text_file() {
        let dir = scratch_dir("real-text");
        let path = dir.join("notes.txt");
        std::fs::write(&path, "hello, minuteman").unwrap();
        assert_eq!(
            load_text(&shared::LocalVfs, &path).unwrap(),
            "hello, minuteman"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_text_returns_none_for_binary_content() {
        let dir = scratch_dir("binary");
        let path = dir.join("fake.rs");
        std::fs::write(&path, [b'a', 0, b'b']).unwrap();
        assert!(load_text(&shared::LocalVfs, &path).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_text_returns_none_for_a_file_over_the_size_cap() {
        let dir = scratch_dir("big");
        let path = dir.join("big.txt");
        std::fs::write(&path, vec![b'a'; (MAX_TEXT_PREVIEW_BYTES + 1) as usize]).unwrap();
        assert!(load_text(&shared::LocalVfs, &path).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn text_is_told_from_binary_by_content() {
        assert!(looks_like_text(b"plain words\nand more"));
        assert!(looks_like_text("naïve — ünïcode".as_bytes()));
        assert!(looks_like_text(b""));
        assert!(!looks_like_text(b"has a \0 in it"));
        assert!(!looks_like_text(&[0xff, 0xfe, 0x41]));
        // A character cut in half by the end of what was read is not a reason to call it binary.
        let cut = &"é".as_bytes()[..1];
        assert!(looks_like_text(&[b"ab", cut].concat()));
    }

    #[test]
    fn load_picks_text_bytes_or_an_archive_for_each_kind_of_file() {
        let dir = scratch_dir("load");
        let write = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };

        assert_eq!(
            load(&shared::LocalVfs, &write("a.txt", b"hi")),
            Loaded::Text("hi".into())
        );
        // No known extension, but plainly text.
        assert_eq!(
            load(&shared::LocalVfs, &write("notes", b"hi")),
            Loaded::Text("hi".into())
        );
        // A text name over a binary body falls back to the bytes.
        assert!(matches!(
            load(&shared::LocalVfs, &write("bin.txt", &[0, 1, 2])),
            Loaded::Bytes(_)
        ));
        assert!(matches!(
            load(&shared::LocalVfs, &write("prog", &[0x7f, b'E', 0, 0])),
            Loaded::Bytes(_)
        ));
        // A damaged archive is just bytes; if it happens to be readable text, it is text.
        assert!(matches!(
            load(&shared::LocalVfs, &write("bad.zip", &[0x50, 0x4b, 0, 1])),
            Loaded::Bytes(_)
        ));
        assert_eq!(
            load(&shared::LocalVfs, &write("words.zip", b"not a zip")),
            Loaded::Text("not a zip".into())
        );

        let big = write(
            "big.txt",
            &vec![b'a'; (MAX_TEXT_PREVIEW_BYTES + 1) as usize],
        );
        assert_eq!(
            load(&shared::LocalVfs, &big),
            Loaded::Failed,
            "a text-named file over the cap still fails"
        );
        let big_unnamed = write(
            "big.data",
            &vec![b'a'; (MAX_TEXT_PREVIEW_BYTES + 1) as usize],
        );
        assert!(matches!(
            load(&shared::LocalVfs, &big_unnamed),
            Loaded::Bytes(_)
        ));

        assert_eq!(load(&shared::LocalVfs, &dir), Loaded::Unsupported);
        assert_eq!(
            load(&shared::LocalVfs, &dir.join("missing")),
            Loaded::Unsupported
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn text_and_bytes_load_from_a_backend_that_is_not_the_disk() {
        let mem = MemVfs::new();
        mem.add_file("/notes.txt", "hello from memory");
        mem.add_file("/blob", vec![0x7f, b'E', 0, 0]);
        mem.add_symlink("/link.txt", "/notes.txt");

        assert_eq!(
            load(&mem, Path::new("/notes.txt")),
            Loaded::Text("hello from memory".into())
        );
        assert_eq!(
            load(&mem, Path::new("/link.txt")),
            Loaded::Text("hello from memory".into()),
            "a link is followed"
        );
        assert!(matches!(load(&mem, Path::new("/blob")), Loaded::Bytes(_)));
        assert_eq!(load(&mem, Path::new("/")), Loaded::Unsupported);
        assert_eq!(load(&mem, Path::new("/missing")), Loaded::Unsupported);
    }

    #[test]
    fn an_archive_off_the_local_disk_is_listed_through_the_backend() {
        // A real zip, in a backend that has no local file to hand out: the listing seeks through
        // it by `open_read` all the same.
        let dir = scratch_dir("mem-zip");
        let zip = dir.join("a.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
        writer
            .start_file("inside.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut writer, b"hi").unwrap();
        writer.finish().unwrap();
        let bytes = std::fs::read(&zip).unwrap();
        let mem = MemVfs::new();
        mem.add_file("/a.zip", bytes);

        assert!(matches!(load(&shared::LocalVfs, &zip), Loaded::Archive(_)));
        assert!(matches!(
            load(&mem, Path::new("/a.zip")),
            Loaded::Archive(_)
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_text_file_over_the_cap_is_refused_on_any_backend() {
        let mem = MemVfs::new();
        mem.add_file(
            "/big.txt",
            vec![b'a'; (MAX_TEXT_PREVIEW_BYTES + 1) as usize],
        );
        assert!(load_text(&mem, Path::new("/big.txt")).is_none());
        assert_eq!(load(&mem, Path::new("/big.txt")), Loaded::Failed);
    }

    #[test]
    fn the_document_is_colored_text_a_hex_head_a_listing_or_nothing() {
        let mem = MemVfs::new();
        mem.add_file("/main.rs", "fn main() {}\n");
        mem.add_file("/blob", vec![1u8, 0, 2]);
        mem.add_file("/pic.png", "not really a picture");
        mem.add_dir("/d");

        let text = content(&mem, Path::new("/main.rs"), &[], &never).unwrap();
        assert!(kinds(&text).contains(&Token::Keyword), "{text:?}");
        assert!(matches!(
            content(&mem, Path::new("/blob"), &[], &never),
            Some(PreviewContent::Hex(_))
        ));
        assert_eq!(
            content(&mem, Path::new("/pic.png"), &[], &never),
            Some(PreviewContent::Empty),
            "pictures are the interface's own business"
        );
        assert_eq!(
            content(&mem, Path::new("/d"), &[], &never),
            Some(PreviewContent::Empty)
        );
        assert_eq!(
            content(&mem, Path::new("/missing"), &[], &never),
            Some(PreviewContent::Empty)
        );
    }

    #[test]
    fn a_failed_read_says_so_and_a_cancelled_one_says_nothing() {
        let mem = MemVfs::new();
        mem.add_file(
            "/big.txt",
            vec![b'a'; (MAX_TEXT_PREVIEW_BYTES + 1) as usize],
        );
        assert_eq!(
            content(&mem, Path::new("/big.txt"), &[], &never),
            Some(PreviewContent::Failed)
        );
        mem.add_file("/small.rs", "fn a() {}\n");
        assert_eq!(content(&mem, Path::new("/small.rs"), &[], &|| true), None);
    }

    #[test]
    fn a_covered_file_is_shown_as_what_the_hook_extracted() {
        let dir = scratch_dir("hook");
        let source = dir.join("report.pdf");
        std::fs::write(&source, b"%PDF-not-real-content").unwrap();
        let hooks = vec![Hook {
            extensions: vec!["pdf".into()],
            command: "printf 'extracted text' > {out}".into(),
            timeout_ms: None,
        }];

        let shown = content(&shared::LocalVfs, &source, &hooks, &never).unwrap();
        assert_eq!(
            shown,
            PreviewContent::Text {
                lines: vec![vec![(Token::Plain, "extracted text".into())]]
            }
        );

        // A hook that fails leaves just the name, not "preview failed".
        let failing = vec![Hook {
            command: "false".into(),
            ..hooks[0].clone()
        }];
        assert_eq!(
            content(&shared::LocalVfs, &source, &failing, &never),
            Some(PreviewContent::Empty)
        );
        // Off the local disk there is no file to hand the program, so it is read directly.
        let mem = MemVfs::new();
        mem.add_file("/report.pdf", "plain words");
        assert!(matches!(
            content(&mem, Path::new("/report.pdf"), &hooks, &never),
            Some(PreviewContent::Text { .. })
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
