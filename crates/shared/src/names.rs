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

//! What a file's name says about it: whether the preview treats it as a picture or as text, and
//! how a name taken from untrusted input is made safe to put on a terminal. These are plain
//! functions over names, never touching a disk, so the interface and the plugins that draw or
//! describe the same files agree on them.

use std::path::Path;

const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "tiff", "tif", "webp",
];

/// Whether `path`'s extension matches a raster image format minuteman can decode and preview.
/// Extension-based, not content-sniffed — a mislabeled file just fails to decode and the caller
/// falls back to the plain name preview, same as any other decode failure.
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
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

/// Whether `path` looks like plain text minuteman can preview: a known text/code extension, or a
/// known extensionless filename (`Makefile`, `.gitignore`, ...). Extension/name-based, not
/// content-sniffed — a mislabeled or binary file just fails to read as text and the caller falls
/// back to another view.
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

/// The longest name [`clean_name`] keeps, in characters. A tar can carry a name of any length in
/// its extended headers.
pub const MAX_NAME_CHARS: usize = 1024;

/// A name made safe to put on a terminal: control characters (an escape sequence in a file name
/// could otherwise repaint or retitle the screen) and the invisible characters that reorder text
/// (a right-to-left override, U+202E, makes `evil` + U+202E + `txt.exe` display as `evilexe.txt`)
/// become `?`, and an absurdly long name is cut with `…`.
pub fn clean_name(raw: &str) -> String {
    let mut chars = raw.chars();
    let mut name: String = chars
        .by_ref()
        .take(MAX_NAME_CHARS)
        .map(|c| if is_unsafe(c) { '?' } else { c })
        .collect();
    if chars.next().is_some() {
        name.push('…');
    }
    name
}

/// Whether `c` can change what the terminal does or the order text reads in, rather than be text.
pub fn is_unsafe(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn recognises_common_image_extensions_case_insensitively() {
        assert!(is_image(Path::new("photo.PNG")));
        assert!(is_image(Path::new("photo.jpg")));
        assert!(is_image(Path::new("photo.jpeg")));
        assert!(!is_image(Path::new("notes.txt")));
        assert!(!is_image(Path::new("no_extension")));
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

    proptest! {
        /// Whatever a name is called, what reaches the screen has no control or
        /// direction-changing character in it and is bounded in length.
        #[test]
        fn cleaned_names_are_safe_and_bounded(raw in any::<String>()) {
            let name = clean_name(&raw);
            prop_assert!(name.chars().all(|c| !is_unsafe(c)));
            prop_assert!(name.chars().count() <= MAX_NAME_CHARS + 1);
        }
    }

    #[test]
    fn escape_sequences_and_direction_overrides_in_names_are_defused() {
        assert_eq!(clean_name("\x1b]0;pwned\x07.txt"), "?]0;pwned?.txt");
        assert_eq!(clean_name("evil\u{202e}txt.exe"), "evil?txt.exe");
        assert_eq!(clean_name("plain-name.rs"), "plain-name.rs");
        assert_eq!(clean_name("naïve/日本語.txt"), "naïve/日本語.txt");
        let long = "a".repeat(MAX_NAME_CHARS + 50);
        assert!(clean_name(&long).ends_with('…'));
    }
}
