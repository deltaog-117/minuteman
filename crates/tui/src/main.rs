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

use std::io::{self, Stdout};
use std::path::PathBuf;

use anyhow::Result;
use browser::BrowserState;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
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

    let guard = TerminalGuard::new()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, &vfs, &mut browser, &config);

    drop(guard);
    result
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    vfs: &LocalVfs,
    browser: &mut BrowserState,
    config: &Config,
) -> Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, browser, vfs, config))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match config.keys.resolve(key.code) {
                Some(Action::Quit) => return Ok(()),
                Some(Action::MoveDown) => browser.move_down(),
                Some(Action::MoveUp) => browser.move_up(),
                Some(Action::Enter) => browser.enter(vfs)?,
                Some(Action::Leave) => browser.leave(vfs)?,
                None => {}
            }
        }
    }
}

fn draw(frame: &mut ratatui::Frame<'_>, browser: &BrowserState, vfs: &LocalVfs, config: &Config) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(40),
            Constraint::Percentage(40),
        ])
        .split(frame.area());

    let selection_style = Style::default()
        .bg(color_from_name(&config.theme.selection_bg))
        .fg(color_from_name(&config.theme.selection_fg));

    // Parent pane — context only, no selection highlight.
    let parent_items: Vec<ListItem> = browser
        .parent_entries()
        .iter()
        .map(|e| ListItem::new(entry_label(e)))
        .collect();
    frame.render_widget(
        List::new(parent_items).block(Block::default().borders(Borders::ALL).title("..")),
        columns[0],
    );

    // Current pane — the active column, with the selection highlighted.
    let current_items: Vec<ListItem> = browser
        .current_entries()
        .iter()
        .map(|e| ListItem::new(entry_label(e)))
        .collect();
    let mut current_state = ListState::default();
    if !browser.current_entries().is_empty() {
        current_state.select(Some(browser.selected_index()));
    }
    let title = browser.current_dir().to_string_lossy().into_owned();
    frame.render_stateful_widget(
        List::new(current_items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(selection_style),
        columns[1],
        &mut current_state,
    );

    // Preview pane — children of the selected directory, or the file's name as a placeholder
    // (real text/image preview content arrives with the `preview` crate later in the roadmap).
    let preview_widget = if browser.selected_entry().map(|e| e.is_dir).unwrap_or(false) {
        let items: Vec<ListItem> = browser
            .preview_entries(vfs)
            .iter()
            .map(|e| ListItem::new(entry_label(e)))
            .collect();
        List::new(items).block(Block::default().borders(Borders::ALL).title("preview"))
    } else {
        let label = browser
            .selected_entry()
            .map(|e| e.name.clone())
            .unwrap_or_default();
        List::new(vec![ListItem::new(label)])
            .block(Block::default().borders(Borders::ALL).title("preview"))
    };
    frame.render_widget(preview_widget, columns[2]);
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
