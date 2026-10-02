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

//! Draws the filmstrip's strip of neighbour thumbnails, from the geometry `view_mode` computed
//! and the pictures `thumbnails` made. The big preview above it is the ordinary preview pane,
//! drawn by `draw` into `FilmstripLayout::big`.

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect, Size};
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use ratatui_image::Image;
use shared::DirEntryInfo;
use theming::Config;

use crate::hud;
use crate::style::{self, FileKind};
use crate::thumbnails::{Thumb, Thumbnails};
use crate::view_mode::FilmstripLayout;

/// What stands in for a picture when there is none (a folder, a text file, an image still being
/// decoded or one that would not decode): the extension in capitals, or `dir`.
pub fn placeholder_text(entry: &DirEntryInfo) -> String {
    if entry.is_dir {
        return "dir".to_string();
    }
    match Path::new(&entry.name).extension().and_then(|e| e.to_str()) {
        Some(ext) if !ext.is_empty() => ext.chars().take(5).collect::<String>().to_uppercase(),
        _ => "file".to_string(),
    }
}

/// Where a picture of `size` sits when centred in `area`; never larger than `area`.
pub fn centered(area: Rect, size: Size) -> Rect {
    let width = size.width.min(area.width);
    let height = size.height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// Draws the strip: a frame titled with the cursor's position, and in it the window of entries
/// around the cursor, each a thumbnail over its name. Asks `thumbs` for the window's pictures,
/// nearest the cursor first, and draws whatever has arrived.
pub fn render(
    frame: &mut Frame<'_>,
    layout: &FilmstripLayout,
    entries: &[DirEntryInfo],
    selected: usize,
    is_marked: impl Fn(&Path) -> bool,
    thumbs: &mut Thumbnails,
    config: &Config,
) {
    let Some(first) = layout.cells.first() else {
        return;
    };
    let window = FilmstripLayout::window(selected, entries.len(), layout.cells.len());
    let picture = FilmstripLayout::picture(*first);

    thumbs.poll();
    let mut wanted: Vec<usize> = window.clone().collect();
    wanted.sort_by_key(|index| index.abs_diff(selected));
    thumbs.request(
        wanted.iter().map(|&index| entries[index].path.as_path()),
        Size::new(picture.width, picture.height),
    );

    let title = format!("{}/{}", selected + 1, entries.len());
    frame.render_widget(style::themed_block(config, &title, false), layout.strip);

    let theme = &config.theme;
    for (slot, index) in window.enumerate() {
        let entry = &entries[index];
        let cell = layout.cells[slot];
        let area = FilmstripLayout::picture(cell);
        let is_selected = index == selected;

        match thumbs.get(&entry.path) {
            Some(Thumb::Ready(protocol)) => {
                frame.render_widget(Image::new(protocol), centered(area, protocol.size()));
            }
            Some(Thumb::Loading) => draw_placeholder(frame, area, "…", config),
            Some(Thumb::Failed) | None => {
                draw_placeholder(frame, area, &placeholder_text(entry), config);
            }
        }

        let kind = FileKind::classify(entry);
        let mut label_style = style::styled(
            Style::default().fg(style::color(kind.theme_color(theme))),
            kind.mods(&config.styles),
        );
        if is_selected {
            label_style = label_style.bg(style::color(&theme.selection_bg));
            if let Some(fg) = style::selection_fg(theme) {
                label_style = label_style.fg(fg);
            }
            label_style = style::styled(label_style, config.styles.selection);
        }
        if is_marked(&entry.path) {
            label_style = style::styled(label_style, config.styles.mark);
        }
        let label_area = FilmstripLayout::label(cell);
        let mark = if is_marked(&entry.path) { "*" } else { "" };
        let text = format!("{mark}{}", entry.name);
        frame.render_widget(
            Paragraph::new(hud::pad_to(&text, label_area.width as usize)).style(label_style),
            label_area,
        );
    }
}

fn draw_placeholder(frame: &mut Frame<'_>, area: Rect, text: &str, config: &Config) {
    if area.height == 0 {
        return;
    }
    let dim = Style::default().fg(style::color(&config.theme.status_fg));
    let middle = Rect::new(area.x, area.y + area.height / 2, area.width, 1);
    frame.render_widget(
        Paragraph::new(text.to_string())
            .style(dim)
            .alignment(Alignment::Center),
        middle,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::path::PathBuf;

    fn entry(name: &str, is_dir: bool) -> DirEntryInfo {
        DirEntryInfo {
            name: name.to_string(),
            path: PathBuf::from(name),
            is_dir,
            size: 0,
            modified: None,
            mode: None,
        }
    }

    #[test]
    fn placeholders_name_the_kind_of_thing() {
        assert_eq!(placeholder_text(&entry("photos", true)), "dir");
        assert_eq!(placeholder_text(&entry("notes.txt", false)), "TXT");
        assert_eq!(placeholder_text(&entry("Makefile", false)), "file");
        assert_eq!(placeholder_text(&entry("a.tar.gz", false)), "GZ");
        assert_eq!(placeholder_text(&entry("a.verylongext", false)), "VERYL");
    }

    proptest! {
        /// Whatever size a picture claims, it is drawn inside the area it was given.
        #[test]
        fn a_centred_picture_never_leaves_its_area(
            ax in 0u16..100, ay in 0u16..100, aw in 0u16..100, ah in 0u16..100,
            w in 0u16..300, h in 0u16..300,
        ) {
            let area = Rect::new(ax, ay, aw, ah);
            let inside = centered(area, Size::new(w, h));
            prop_assert_eq!(inside.intersection(area), inside);
        }
    }
}
