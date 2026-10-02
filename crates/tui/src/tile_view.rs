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

//! One framed tile — a picture over the entry's name — as the grid and the filmstrip's strip both
//! draw it. The geometry (`view_mode::tile_picture`, `tile_label`) is pure; this paints it.

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
use crate::thumbnails::Thumb;
use crate::view_mode::{tile_label, tile_picture};

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

/// What a tile shows besides the entry itself.
#[derive(Clone, Copy)]
pub struct TileState<'a> {
    pub selected: bool,
    pub marked: bool,
    /// The cached picture, if one was ever asked for.
    pub thumb: Option<&'a Thumb>,
}

/// Draws one tile into `tile`: a frame (bright and filled when selected), the picture or a
/// placeholder centred in it, and the name, marked with `*`, in the row under the picture.
pub fn render_tile(
    frame: &mut Frame<'_>,
    tile: Rect,
    entry: &DirEntryInfo,
    state: TileState<'_>,
    config: &Config,
) {
    let theme = &config.theme;
    let mut block = style::themed_block(config, "", state.selected);
    if state.selected {
        block = block.style(Style::default().bg(style::color(&theme.selection_bg)));
    }
    frame.render_widget(block, tile);

    let area = tile_picture(tile);
    match state.thumb {
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
    if state.selected {
        label_style = label_style.bg(style::color(&theme.selection_bg));
        if let Some(fg) = style::selection_fg(theme) {
            label_style = label_style.fg(fg);
        }
        label_style = style::styled(label_style, config.styles.selection);
    }
    if state.marked {
        label_style = style::styled(label_style, config.styles.mark);
    }
    let label_area = tile_label(tile);
    let mark = if state.marked { "*" } else { "" };
    let name = if entry.is_dir {
        format!("{mark}{}/", entry.name)
    } else {
        format!("{mark}{}", entry.name)
    };
    frame.render_widget(
        Paragraph::new(hud::pad_to(&name, label_area.width as usize)).style(label_style),
        label_area,
    );
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

        /// The picture and the name never leave their tile and never overlap each other.
        #[test]
        fn a_tiles_parts_stay_inside_it(
            x in 0u16..50, y in 0u16..50, w in 0u16..60, h in 0u16..30,
        ) {
            let tile = Rect::new(x, y, w, h);
            let (picture, label) = (tile_picture(tile), tile_label(tile));
            // An empty rectangle draws nothing wherever it sits, so only a real one must be inside.
            prop_assert!(picture.is_empty() || picture.intersection(tile) == picture);
            prop_assert!(label.is_empty() || label.intersection(tile) == label);
            prop_assert!(picture.intersection(label).is_empty());
        }
    }
}
