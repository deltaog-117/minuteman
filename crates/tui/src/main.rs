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
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
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
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, &guard, &vfs, &mut browser, &mut app, &config);

    drop(guard);
    result
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    guard: &TerminalGuard,
    vfs: &LocalVfs,
    browser: &mut BrowserState,
    app: &mut App,
    config: &Config,
) -> Result<()> {
    loop {
        app.poll_bulk(browser, vfs)?;
        terminal.draw(|frame| draw(frame, browser, app, vfs, config))?;

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

    frame.render_widget(Paragraph::new(app.status_line()), rows[1]);
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
