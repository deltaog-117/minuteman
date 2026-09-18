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
mod popup_shell;
mod shell_layout;
mod text_preview;

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use app::App;
use browser::BrowserState;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use image_preview::{ImagePreview, PreviewStatus as ImagePreviewStatus};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui_image::StatefulImage;
use shared::{DirEntryInfo, LocalVfs};
use shell_layout::{ShellPanes, SplitDirection};
use text_preview::{PreviewStatus as TextPreviewStatus, TextPreview};
use theming::{Action, Config};

/// Restores the terminal (raw mode + alternate screen) on drop, so a panic or an early return
/// from `run` never leaves the user's shell in a broken state.
struct TerminalGuard;

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
    }
}

/// The region shell panes render into and size their ptys against: the frame minus the bottom
/// status-bar row. Mirrors `draw`'s own top-level `Layout` split exactly (same constraints), so a
/// pane's pty size can never drift from its actual rendered rect.
fn shell_area(frame_area: Rect) -> Rect {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(frame_area)[0]
}

/// The image and text preview pipelines, bundled together since every call site drives both in
/// lockstep off the same selected entry.
struct Previews {
    image: ImagePreview,
    text: TextPreview,
}

impl Previews {
    fn new(handle: tokio::runtime::Handle) -> Self {
        Self {
            image: ImagePreview::new(handle.clone()),
            text: TextPreview::new(handle),
        }
    }

    fn update(&mut self, selected: Option<&Path>) {
        self.image.update(selected);
        self.text.update(selected);
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
    let mut previews = Previews::new(runtime.handle().clone());
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = run(
        &mut terminal,
        &vfs,
        &mut browser,
        &mut app,
        &mut previews,
        &config,
    );

    drop(guard);
    result
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    vfs: &LocalVfs,
    browser: &mut BrowserState,
    app: &mut App,
    previews: &mut Previews,
    config: &Config,
) -> Result<()> {
    // The tmux-style split-pane shell tree (see `shell_layout`) — `Some` for the whole time any
    // shell pane is open. Never suspends raw mode/the alternate screen: it's just tiled into the
    // frame's own area (below the status bar) as part of the normal draw.
    let mut shells: Option<ShellPanes> = None;
    // Whether keystrokes go to the focused pane (true) or drive the browser underneath while the
    // pane(s) stay open and visible (false). Meaningless while `shells` is `None`.
    let mut shell_focused = true;
    // The divider (by id) currently being dragged, from a mouse-down that hit one. `None` means
    // no drag is in progress.
    let mut dragging_divider: Option<usize> = None;

    loop {
        app.poll_bulk(browser, vfs)?;
        previews.update(browser.selected_entry().map(|e| e.path.as_path()));

        if let Some(mut panes) = shells.take() {
            let area = shell_area(terminal.size()?.into());
            let exited = panes.poll_exits()?;
            let mut current = Some(panes);
            for (id, outcome) in exited {
                let Some(p) = current.take() else { break };
                app.status = Some(if outcome.success {
                    "shell exited".into()
                } else {
                    format!("shell exited: code {}", outcome.code)
                });
                current = p.close(id, area)?;
            }
            shells = current;
        }

        terminal.draw(|frame| draw(frame, browser, app, previews, vfs, config, shells.as_ref()))?;

        // While a shell pane is open, poll faster so its output (e.g. a redrawing `vim` or `top`)
        // feels responsive rather than updating in 100ms steps.
        let poll_timeout = if shells.is_some() {
            Duration::from_millis(16)
        } else {
            Duration::from_millis(100)
        };
        if !event::poll(poll_timeout)? {
            continue;
        }

        match event::read()? {
            Event::Resize(cols, rows) => {
                if let Some(panes) = shells.as_ref() {
                    panes.resize(shell_area(Rect::new(0, 0, cols, rows)))?;
                }
            }
            Event::Mouse(mouse) => {
                let Some(panes) = shells.as_mut() else {
                    continue;
                };
                let area = shell_area(terminal.size()?.into());
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(divider) = panes
                            .dividers(area)
                            .into_iter()
                            .find(|d| d.hit(mouse.column, mouse.row))
                        {
                            dragging_divider = Some(divider.id());
                        } else if panes.focus_at(area, mouse.column, mouse.row) {
                            shell_focused = true;
                        }
                    }
                    MouseEventKind::Drag(MouseButton::Left) => {
                        if let Some(id) = dragging_divider {
                            let ratio = panes
                                .dividers(area)
                                .into_iter()
                                .find(|d| d.id() == id)
                                .map(|d| d.ratio_at(mouse.column, mouse.row));
                            if let Some(ratio) = ratio {
                                panes.set_ratio(id, ratio, area)?;
                            }
                        }
                    }
                    MouseEventKind::Up(MouseButton::Left) => {
                        dragging_divider = None;
                    }
                    _ => {}
                }
            }
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if shells.is_some() {
                    let area = shell_area(terminal.size()?.into());
                    if shell_focused {
                        if key.code == KeyCode::Esc {
                            let panes = shells.take().expect("`shells.is_some()` checked above");
                            let id = panes.focused_id();
                            let remaining = panes.close(id, area)?;
                            app.status = Some(if remaining.is_some() {
                                "pane closed".into()
                            } else {
                                "shell closed".into()
                            });
                            shells = remaining;
                        } else if config.keys.resolve(key.code) == Some(Action::ShellFocus) {
                            shell_focused = false;
                            app.status = Some("browsing — tab to refocus the shell".into());
                        } else if config.keys.resolve(key.code)
                            == Some(Action::ShellSplitHorizontal)
                        {
                            let panes = shells.take().expect("`shells.is_some()` checked above");
                            shells = Some(panes.split(
                                SplitDirection::Horizontal,
                                browser.current_dir(),
                                area,
                            )?);
                        } else if config.keys.resolve(key.code) == Some(Action::ShellSplitVertical)
                        {
                            let panes = shells.take().expect("`shells.is_some()` checked above");
                            shells = Some(panes.split(
                                SplitDirection::Vertical,
                                browser.current_dir(),
                                area,
                            )?);
                        } else if config.keys.resolve(key.code) == Some(Action::ShellPaneNext) {
                            shells
                                .as_mut()
                                .expect("`shells.is_some()` checked above")
                                .focus_next();
                        } else if let Some(bytes) = popup_shell::encode_key(key) {
                            shells
                                .as_mut()
                                .expect("`shells.is_some()` checked above")
                                .focused_shell()
                                .write_input(&bytes)?;
                        }
                        continue;
                    } else if config.keys.resolve(key.code) == Some(Action::ShellFocus) {
                        shell_focused = true;
                        app.status = Some("shell focused".into());
                        continue;
                    }
                    // Panes open, unfocused, and not a shell-control key — fall through so the
                    // browser dispatch below still handles it.
                }

                if app.prompt.is_some() {
                    if app.handle_prompt_key(key.code, vfs, browser)?.is_break() {
                        return Ok(());
                    }
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
                    Some(Action::Search) => app.begin_search(browser),
                    Some(Action::Command) => app.begin_command(),
                    Some(Action::Select) => browser.toggle_mark(),
                    // Inert for now — reserved for future chorded commands. Deliberately does
                    // not enter any modal/capturing state, so every other key keeps working
                    // exactly as if leader didn't exist.
                    Some(Action::Leader) => {}
                    // All meaningless without an open pane — handled above (before this match)
                    // whenever `shells` is `Some`.
                    Some(Action::ShellFocus)
                    | Some(Action::ShellSplitHorizontal)
                    | Some(Action::ShellSplitVertical)
                    | Some(Action::ShellPaneNext) => {}
                    // Guarded explicitly rather than relying on it being unreachable while panes
                    // are already open (the branch above handles that case) — opening a second
                    // tree here would silently drop the running one without closing it. Use a
                    // split instead once a shell is already open.
                    Some(Action::Shell) if shells.is_none() => {
                        let area = shell_area(terminal.size()?.into());
                        match ShellPanes::open(browser.current_dir(), area) {
                            Ok(panes) => {
                                shells = Some(panes);
                                shell_focused = true;
                            }
                            Err(e) => app.status = Some(format!("failed to start shell: {e}")),
                        }
                    }
                    Some(Action::Shell) | None => {}
                }
            }
            _ => {}
        }
    }
}

fn draw(
    frame: &mut ratatui::Frame<'_>,
    browser: &BrowserState,
    app: &App,
    previews: &mut Previews,
    vfs: &LocalVfs,
    config: &Config,
    shells: Option<&ShellPanes>,
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
        .map(|e| entry_item(e, config, false))
        .collect();
    frame.render_widget(
        List::new(parent_items).block(themed_block(config, "..")),
        columns[0],
    );

    // Current pane — the active column, with the selection highlighted and marked entries
    // prefixed (Ranger-style) so a pending multi-select is visible before acting on it.
    let current_items: Vec<ListItem> = browser
        .current_entries()
        .iter()
        .map(|e| entry_item(e, config, browser.is_marked(&e.path)))
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

    // Preview pane — an inline image for image files, rendered text for code/text files,
    // children of a selected directory, or the file's name as a placeholder for anything else.
    let is_selected_image = browser
        .selected_entry()
        .is_some_and(|e| !e.is_dir && preview::is_image(&e.path));
    let is_selected_text = browser
        .selected_entry()
        .is_some_and(|e| !e.is_dir && preview::is_text(&e.path));

    if is_selected_image {
        let block = themed_block(config, "preview");
        let inner = block.inner(columns[2]);
        frame.render_widget(block, columns[2]);
        match previews.image.status() {
            ImagePreviewStatus::Ready => {
                frame.render_stateful_widget(
                    StatefulImage::default(),
                    inner,
                    previews.image.protocol_mut(),
                );
            }
            ImagePreviewStatus::Loading => {
                frame.render_widget(Paragraph::new("loading preview…"), inner);
            }
            ImagePreviewStatus::Failed => {
                frame.render_widget(Paragraph::new("preview failed"), inner);
            }
            ImagePreviewStatus::Empty => {}
        }
    } else if is_selected_text {
        let block = themed_block(config, "preview");
        let inner = block.inner(columns[2]);
        frame.render_widget(block, columns[2]);
        let style = Style::default().fg(color_from_name(&config.theme.file_fg));
        match previews.text.status() {
            TextPreviewStatus::Ready => {
                frame.render_widget(
                    Paragraph::new(previews.text.content())
                        .style(style)
                        .wrap(Wrap { trim: false }),
                    inner,
                );
            }
            TextPreviewStatus::Loading => {
                frame.render_widget(Paragraph::new("loading preview…"), inner);
            }
            TextPreviewStatus::Failed => {
                frame.render_widget(Paragraph::new("preview failed"), inner);
            }
            TextPreviewStatus::Empty => {}
        }
    } else if browser.selected_entry().map(|e| e.is_dir).unwrap_or(false) {
        let items: Vec<ListItem> = browser
            .preview_entries(vfs)
            .iter()
            .map(|e| entry_item(e, config, false))
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

    // Drawn last, over the browser columns above — the tiled shell panes, docked into the same
    // area `rows[0]` occupies so they never cover the status bar.
    if let Some(panes) = shells {
        panes.render(frame, rows[0], config);
    }
}

/// A pane `Block` styled with the theme's border/title colors — every pane uses the same frame.
fn themed_block<'a>(config: &Config, title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color_from_name(&config.theme.border_fg)))
        .title(title)
        .title_style(Style::default().fg(color_from_name(&config.theme.title_fg)))
}

fn entry_item(entry: &DirEntryInfo, config: &Config, marked: bool) -> ListItem<'static> {
    let label = entry_label(entry);
    let color = if entry.is_dir {
        color_from_name(&config.theme.dir_fg)
    } else {
        color_from_name(&config.theme.file_fg)
    };
    let mut style = Style::default().fg(color);
    let label = if marked {
        style = style.add_modifier(Modifier::BOLD);
        format!("* {label}")
    } else {
        label
    };
    ListItem::new(Span::styled(label, style))
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
