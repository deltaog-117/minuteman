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

//! Draws the grid view: framed tiles, each a thumbnail over its name, filling the window like a
//! graphical file manager's icon view. The geometry is `view_mode::GridLayout`; each tile is
//! `tile_view::render_tile`, the same one the filmstrip's strip uses.

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect, Size};
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use shared::DirEntryInfo;
use theming::Config;

use crate::style;
use crate::thumbnails::Thumbnails;
use crate::tile_view::{TileState, render_tile};
use crate::view_mode::{GridLayout, tile_picture};

/// Draws the tiles of the rows from `top` down. Asks `thumbs` for the pictures on screen,
/// top-left first, and draws whatever has arrived.
#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame<'_>,
    body: Rect,
    grid: &GridLayout,
    top: usize,
    entries: &[DirEntryInfo],
    selected: usize,
    is_marked: impl Fn(&Path) -> bool,
    thumbs: &mut Thumbnails,
    config: &Config,
) {
    if entries.is_empty() {
        let dim = Style::default().fg(style::color(&config.theme.status_fg));
        frame.render_widget(
            Paragraph::new("empty folder")
                .style(dim)
                .alignment(Alignment::Center),
            Rect::new(
                body.x,
                body.y + body.height / 2,
                body.width,
                1.min(body.height),
            ),
        );
        return;
    }
    let visible = grid.visible(top, entries.len());
    let Some(first) = grid.rect_of(visible.start, top) else {
        return;
    };
    let picture = tile_picture(first);
    thumbs.poll();
    thumbs.request(
        visible.clone().map(|index| entries[index].path.as_path()),
        Size::new(picture.width, picture.height),
    );

    for index in visible {
        let Some(tile) = grid.rect_of(index, top) else {
            continue;
        };
        let entry = &entries[index];
        render_tile(
            frame,
            tile,
            entry,
            TileState {
                selected: index == selected,
                marked: is_marked(&entry.path),
                thumb: thumbs.get(&entry.path),
            },
            config,
        );
    }
}
