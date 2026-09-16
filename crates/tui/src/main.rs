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

mod app;
mod image_preview;

use std::io::{self, Stdout};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use app::App;
use browser::BrowserState;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use image_preview::{ImagePreview, PreviewStatus};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui_image::StatefulImage;
use shared::{DirEntryInfo, LocalVfs};
use theming::{Action, Config};

/// Restores the terminal (raw mode + alternate screen) on drop, so a panic or an early return
/// from `run` never leaves the user's shell in a broken state.
struct TerminalGuard;

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(Self)
    }

    /// Hands the real terminal back to normal (cooked) mode, e.g. so a child process like an
    /// interactive shell can use it directly. Pair with `resume`.
    fn suspend(&self) -> Result<()> {
        disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen)?;
        Ok(())
    }

    /// Reverses `suspend`. The caller must also force a full redraw (`Terminal::clear`)
    /// afterward — ratatui's diffing buffer doesn't know the screen was replaced meanwhile.
    fn resume(&self) -> Result<()> {
        execute!(io::stdout(), EnterAlternateScreen)?;
        enable_raw_mode()?;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn main() -> Result<()> {
    let start_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let start_dir = start_dir.canonicalize().unwrap_or(start_dir);

    let config = Config::load();
    let vfs = LocalVfs;
    let mut browser = BrowserState::new(&vfs, start_dir)?;

    // Backs the blocking thread pool that copy/move/delete run on so a large operation never
    // freezes the render loop. Kept alive for the rest of `main` — dropping it would shut the
    // pool down out from under any operation still running.
    let runtime = tokio::runtime::Runtime::new()?;
    let mut app = App::new(runtime.handle().clone());

    let guard = TerminalGuard::new()?;
    // Must run after entering the alternate screen but before the event loop reads any input —
    // it briefly reads/writes stdio itself to probe the terminal's graphics-protocol support.
    let mut image_preview = ImagePreview::new(runtime.handle().clone());
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = run(
        &mut terminal,
        &guard,
        &vfs,
        &mut browser,
        &mut app,
        &mut image_preview,
        &config,
    );

    drop(guard);
    result
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    guard: &TerminalGuard,
    vfs: &LocalVfs,
    browser: &mut BrowserState,
    app: &mut App,
    image_preview: &mut ImagePreview,
    config: &Config,
) -> Result<()> {
    loop {
        app.poll_bulk(browser, vfs)?;
        image_preview.update(browser.selected_entry().map(|e| e.path.as_path()));
        terminal.draw(|frame| draw(frame, browser, app, image_preview, vfs, config))?;

        // A timed poll (rather than a blocking read) so the loop keeps ticking — and picking up
        // background-operation progress — even while the user isn't pressing anything.
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if app.prompt.is_some() {
                app.handle_prompt_key(key.code, vfs, browser)?;
                continue;
            }

            if app.is_busy() {
                if key.code == KeyCode::Esc {
                    app.cancel_bulk();
                }
                continue;
            }

            match config.keys.resolve(key.code) {
                Some(Action::Quit) => return Ok(()),
                Some(Action::MoveDown) => browser.move_down(),
                Some(Action::MoveUp) => browser.move_up(),
                Some(Action::Enter) => browser.enter(vfs)?,
                Some(Action::Leave) => browser.leave(vfs)?,
                Some(Action::Yank) => app.yank(browser),
                Some(Action::Cut) => app.cut(browser),
                Some(Action::Paste) => app.begin_paste(browser),
                Some(Action::Delete) => app.begin_delete(browser),
                Some(Action::Rename) => app.begin_rename(browser),
                Some(Action::Create) => app.begin_create(),
                Some(Action::Shell) => {
                    guard.suspend()?;
                    let result = shell_overlay::spawn_shell(browser.current_dir());
                    guard.resume()?;
                    // Force a full redraw of every cell, since the shell left arbitrary content
                    // on screen. `resize` to the current size does this (and resets ratatui's
                    // diffing buffer) without `clear`'s cursor-position query, which needs the
                    // terminal to answer an escape-code probe and can time out on some
                    // terminals/multiplexers — not worth risking a crash right after the user
                    // returns from their shell.
                    let area = terminal.size()?.into();
                    terminal.resize(area)?;
                    app.status = Some(match result {
                        Ok(status) if status.success() => "shell exited".into(),
                        Ok(status) => format!("shell exited: {status}"),
                        Err(e) => format!("failed to start shell: {e}"),
                    });
                }
                None => {}
            }
        }
    }
}

fn draw(
    frame: &mut ratatui::Frame<'_>,
    browser: &BrowserState,
    app: &App,
    image_preview: &mut ImagePreview,
    vfs: &LocalVfs,
    config: &Config,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(frame.area());

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(40),
            Constraint::Percentage(40),
        ])
        .split(rows[0]);

    let selection_style = Style::default()
        .bg(color_from_name(&config.theme.selection_bg))
        .fg(color_from_name(&config.theme.selection_fg));

    // Parent pane — context only, no selection highlight.
    let parent_items: Vec<ListItem> = browser
        .parent_entries()
        .iter()
        .map(|e| entry_item(e, config))
        .collect();
    frame.render_widget(
        List::new(parent_items).block(themed_block(config, "..")),
        columns[0],
    );

    // Current pane — the active column, with the selection highlighted.
    let current_items: Vec<ListItem> = browser
        .current_entries()
        .iter()
        .map(|e| entry_item(e, config))
        .collect();
    let mut current_state = ListState::default();
    if !browser.current_entries().is_empty() {
        current_state.select(Some(browser.selected_index()));
    }
    let title = browser.current_dir().to_string_lossy().into_owned();
    frame.render_stateful_widget(
        List::new(current_items)
            .block(themed_block(config, &title))
            .highlight_style(selection_style),
        columns[1],
        &mut current_state,
    );

    // Preview pane — an inline image for image files, children of a selected directory, or the
    // file's name as a placeholder (text preview is a future roadmap item).
    let is_selected_image = browser
        .selected_entry()
        .is_some_and(|e| !e.is_dir && preview::is_image(&e.path));

    if is_selected_image {
        let block = themed_block(config, "preview");
        let inner = block.inner(columns[2]);
        frame.render_widget(block, columns[2]);
        match image_preview.status() {
            PreviewStatus::Ready => {
                frame.render_stateful_widget(
                    StatefulImage::default(),
                    inner,
                    image_preview.protocol_mut(),
                );
            }
            PreviewStatus::Loading => {
                frame.render_widget(Paragraph::new("loading preview…"), inner);
            }
            PreviewStatus::Failed => {
                frame.render_widget(Paragraph::new("preview failed"), inner);
            }
            PreviewStatus::Empty => {}
        }
    } else if browser.selected_entry().map(|e| e.is_dir).unwrap_or(false) {
        let items: Vec<ListItem> = browser
            .preview_entries(vfs)
            .iter()
            .map(|e| entry_item(e, config))
            .collect();
        frame.render_widget(
            List::new(items).block(themed_block(config, "preview")),
            columns[2],
        );
    } else {
        let label = browser
            .selected_entry()
            .map(|e| e.name.clone())
            .unwrap_or_default();
        let style = Style::default().fg(color_from_name(&config.theme.file_fg));
        frame.render_widget(
            List::new(vec![ListItem::new(Span::styled(label, style))])
                .block(themed_block(config, "preview")),
            columns[2],
        );
    }

    let status_style = Style::default().fg(color_from_name(&config.theme.status_fg));
    frame.render_widget(
        Paragraph::new(app.status_line()).style(status_style),
        rows[1],
    );
}

/// A pane `Block` styled with the theme's border/title colors — every pane uses the same frame.
fn themed_block<'a>(config: &Config, title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color_from_name(&config.theme.border_fg)))
        .title(title)
        .title_style(Style::default().fg(color_from_name(&config.theme.title_fg)))
}

fn entry_item(entry: &DirEntryInfo, config: &Config) -> ListItem<'static> {
    let label = entry_label(entry);
    let color = if entry.is_dir {
        color_from_name(&config.theme.dir_fg)
    } else {
        color_from_name(&config.theme.file_fg)
    };
    ListItem::new(Span::styled(label, Style::default().fg(color)))
}

fn entry_label(entry: &DirEntryInfo) -> String {
    if entry.is_dir {
        format!("{}/", entry.name)
    } else {
        entry.name.clone()
    }
}

fn color_from_name(name: &str) -> Color {
    match name.to_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        _ => Color::Reset,
    }
}
