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

//! Decides whether a file can be shown as an inline image or text preview, and loads it. Pure
//! and terminal-agnostic — `tui` owns the graphics-protocol rendering and background threading
//! built on top of this.

use std::io::{BufReader, Read};
use std::path::Path;

use image::DynamicImage;
use shared::{FileKind, Vfs};

pub mod archive;
pub mod hex;
pub mod highlight;

const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "tiff", "tif", "webp",
];

/// Whether `path`'s extension matches a raster image format minuteman can decode and preview.
/// Extension-based, not content-sniffed — a mislabeled file just fails to decode in
/// [`load_image`] and the caller falls back to the plain listing/name preview, same as any other
/// decode failure.
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
}

/// Decodes `path` as an image. The format is sniffed from the file's own bytes rather than
/// trusted from its extension, so a preview hook's scratch output — whatever extension it
/// happens to have — still decodes correctly. Returns `None` on any I/O or decode failure — the
/// caller falls back to the plain file-name preview rather than surfacing an error for a corrupt
/// or unsupported image.
pub fn load_image(vfs: &dyn Vfs, path: &Path) -> Option<DynamicImage> {
    image::ImageReader::new(BufReader::new(vfs.open_read(path).ok()?))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()
}

const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "rst", "tex", "csv", "tsv", "log", "diff", "patch", "rs", "py", "js",
    "mjs", "cjs", "jsx", "ts", "tsx", "go", "c", "h", "cpp", "cc", "cxx", "hpp", "hxx", "java",
    "kt", "kts", "swift", "rb", "php", "pl", "pm", "lua", "sh", "bash", "zsh", "fish", "ps1",
    "sql", "css", "scss", "sass", "less", "html", "htm", "xml", "svg", "json", "jsonc", "yaml",
    "yml", "toml", "ini", "cfg", "conf", "env", "vue", "svelte", "r", "jl", "hs", "ex", "exs",
    "erl", "clj", "cljs", "scala", "dart", "nim", "zig", "vim", "el", "asm", "s", "proto",
    "graphql", "gql", "cmake", "gradle",
];

/// Files that are plain text despite carrying no extension (or an extension not covered by
/// [`TEXT_EXTENSIONS`]), matched case-insensitively against the full file name.
const TEXT_FILENAMES: &[&str] = &[
    "makefile",
    "dockerfile",
    "containerfile",
    "vagrantfile",
    "rakefile",
    "gemfile",
    "license",
    "readme",
    "changelog",
    "authors",
    "contributing",
    ".gitignore",
    ".gitattributes",
    ".env",
    ".bashrc",
    ".zshrc",
    ".vimrc",
    ".editorconfig",
];

/// Caps how much of a file [`load_text`] will read into memory for a preview — large enough for
/// any normal source/config file, small enough that a huge log or data file can't stall the
/// background read thread or bloat the render buffer.
const MAX_TEXT_PREVIEW_BYTES: u64 = 1 << 20;

/// Whether `path` looks like plain text minuteman can preview: a known text/code extension, or a
/// known extensionless filename (`Makefile`, `.gitignore`, ...). Extension/name-based, not
/// content-sniffed — a mislabeled or binary file just fails in [`load_text`] and the caller falls
/// back to the plain listing/name preview, same as any other read failure.
pub fn is_text(path: &Path) -> bool {
    let has_text_extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| TEXT_EXTENSIONS.contains(&ext.to_lowercase().as_str()));
    if has_text_extension {
        return true;
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| TEXT_FILENAMES.contains(&name.to_lowercase().as_str()))
}

/// Reads `path` as UTF-8 text. Returns `None` if the file exceeds [`MAX_TEXT_PREVIEW_BYTES`],
/// contains a null byte (a binary file mislabeled with a text-like name), or isn't valid UTF-8 —
/// the caller falls back to the plain file-name preview rather than surfacing an error.
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

/// What the preview pane shows for a file that is not an image or a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    Text(String),
    /// A binary file's first bytes, for the hex view.
    Bytes(hex::Head),
    Archive(archive::Listing),
    /// The file should have a preview but could not be read, or is too big for one.
    Failed,
    /// Not a regular file (a socket, a device, a named pipe): nothing to show.
    Unsupported,
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

/// Decides what to show for the regular file at `path` and reads it, which can take a while on a
/// slow disk — call it off the render thread. In order: an archive (by name) is listed; a file
/// that is text, by its name or its content, is read whole up to 1 MiB; everything else is shown
/// as bytes. A text-named file over the size cap, or one that cannot be read, is `Failed`.
pub fn load(vfs: &dyn Vfs, path: &Path) -> Loaded {
    let Some(head) = hex::read_head(vfs, path) else {
        // `read_head` refuses anything that is not a regular file, and a regular file it could
        // not read is a failure worth saying so; tell the two apart by asking again.
        return match vfs.metadata(path) {
            Ok(meta) if meta.kind == FileKind::File => Loaded::Failed,
            _ => Loaded::Unsupported,
        };
    };
    // A listing reads the archive by seeking through its own directory structure, which needs a
    // file on this machine's disk; on any other backend the archive is shown as bytes instead.
    if let Some(local) = vfs.local_path(path)
        && let Some(listing) = archive::list(&local)
    {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_common_image_extensions_case_insensitively() {
        assert!(is_image(Path::new("photo.PNG")));
        assert!(is_image(Path::new("photo.jpg")));
        assert!(is_image(Path::new("photo.jpeg")));
        assert!(!is_image(Path::new("notes.txt")));
        assert!(!is_image(Path::new("no_extension")));
    }

    fn scratch_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "minuteman-preview-test-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_image_returns_none_for_a_non_image_file() {
        let dir = scratch_dir("non-image");
        let path = dir.join("not_an_image.png");
        std::fs::write(&path, b"not actually a png").unwrap();

        assert!(load_image(&shared::LocalVfs, &path).is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_image_decodes_a_real_image() {
        let dir = scratch_dir("real-image");
        let path = dir.join("pixel.png");
        image::RgbImage::from_pixel(1, 1, image::Rgb([255, 0, 0]))
            .save(&path)
            .unwrap();

        let decoded = load_image(&shared::LocalVfs, &path).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1, 1));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recognises_text_extensions_and_extensionless_filenames_case_insensitively() {
        assert!(is_text(Path::new("main.RS")));
        assert!(is_text(Path::new("notes.md")));
        assert!(is_text(Path::new("Makefile")));
        assert!(is_text(Path::new(".gitignore")));
        assert!(!is_text(Path::new("photo.png")));
        assert!(!is_text(Path::new("no_extension")));
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
        let dir = scratch_dir("binary-content");
        let path = dir.join("notes.txt");
        std::fs::write(&path, [0u8, 159, 146, 150]).unwrap();

        assert!(load_text(&shared::LocalVfs, &path).is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_text_returns_none_for_a_file_over_the_size_cap() {
        let dir = scratch_dir("oversized-text");
        let path = dir.join("huge.txt");
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

    // ---- a backend that is not the local disk ----

    fn memory() -> shared::MemVfs {
        shared::MemVfs::new()
    }

    #[test]
    fn text_and_bytes_load_from_a_backend_that_is_not_the_disk() {
        let mem = memory();
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
    fn an_image_decodes_from_a_backend_that_is_not_the_disk() {
        let mut png = Vec::new();
        DynamicImage::new_rgb8(3, 2)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let mem = memory();
        mem.add_file("/pic.png", png);
        mem.add_file("/fake.png", "not an image");

        let decoded = load_image(&mem, Path::new("/pic.png")).unwrap();

        assert_eq!((decoded.width(), decoded.height()), (3, 2));
        assert!(load_image(&mem, Path::new("/fake.png")).is_none());
    }

    #[test]
    fn an_archive_off_the_local_disk_is_shown_as_bytes_not_listed() {
        // A real zip, but the backend cannot hand back a local file to read its directory from.
        let dir = scratch_dir("mem-zip");
        let zip = dir.join("a.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
        writer
            .start_file("inside.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut writer, b"hi").unwrap();
        writer.finish().unwrap();
        let bytes = std::fs::read(&zip).unwrap();
        let mem = memory();
        mem.add_file("/a.zip", bytes);

        assert!(matches!(load(&shared::LocalVfs, &zip), Loaded::Archive(_)));
        assert!(matches!(load(&mem, Path::new("/a.zip")), Loaded::Bytes(_)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_text_file_over_the_cap_is_refused_on_any_backend() {
        let mem = memory();
        mem.add_file(
            "/big.txt",
            vec![b'a'; (MAX_TEXT_PREVIEW_BYTES + 1) as usize],
        );

        assert!(load_text(&mem, Path::new("/big.txt")).is_none());
        assert_eq!(load(&mem, Path::new("/big.txt")), Loaded::Failed);
    }
}
