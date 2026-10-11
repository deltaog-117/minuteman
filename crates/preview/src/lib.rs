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

//! Decodes an image for the inline preview. Pure and terminal-agnostic — `tui` owns the
//! graphics-protocol rendering and background threading built on top of this. Text, hex and
//! archive previews are not here any more: the `mman-preview` plugin reads and describes those,
//! and `shared::names` holds the file-name rules both sides agree on.

use std::io::BufReader;
use std::path::Path;

use image::DynamicImage;
use shared::Vfs;

pub use shared::is_image;

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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn an_image_decodes_from_a_backend_that_is_not_the_disk() {
        let mut png = Vec::new();
        DynamicImage::new_rgb8(3, 2)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let mem = shared::MemVfs::new();
        mem.add_file("/pic.png", png);
        mem.add_file("/fake.png", "not an image");

        let decoded = load_image(&mem, Path::new("/pic.png")).unwrap();

        assert_eq!((decoded.width(), decoded.height()), (3, 2));
        assert!(load_image(&mem, Path::new("/fake.png")).is_none());
    }
}
