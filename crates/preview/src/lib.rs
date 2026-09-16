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

//! Decides whether a file can be shown as an inline image preview, and decodes it. Pure and
//! terminal-agnostic — `tui` owns the graphics-protocol rendering and background threading built
//! on top of this.

use std::path::Path;

use image::DynamicImage;

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

/// Decodes `path` as an image. Returns `None` on any I/O or decode failure — the caller falls
/// back to the plain file-name preview rather than surfacing an error for a corrupt or
/// unsupported image.
pub fn load_image(path: &Path) -> Option<DynamicImage> {
    image::ImageReader::open(path).ok()?.decode().ok()
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

        assert!(load_image(&path).is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_image_decodes_a_real_image() {
        let dir = scratch_dir("real-image");
        let path = dir.join("pixel.png");
        image::RgbImage::from_pixel(1, 1, image::Rgb([255, 0, 0]))
            .save(&path)
            .unwrap();

        let decoded = load_image(&path).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1, 1));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
