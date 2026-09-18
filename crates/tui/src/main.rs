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
use shell_layout::{NudgeDir, ShellPanes, SplitDirection};
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

/// The floor either dimension of the shell box can shrink to, regardless of `size_adjust` —
/// below this it stops being usable as a terminal at all.
const MIN_SHELL_BOX_WIDTH: u16 = 20;
const MIN_SHELL_BOX_HEIGHT: u16 = 6;

/// The region shell panes render into and size their ptys against: an 80%-width/70%-height box
/// by default (clamped to `MIN_SHELL_BOX_WIDTH`/`HEIGHT` on the small end and the space available
/// above the status bar on the large end), adjusted by `size_adjust` — a `(dw, dh)` nudge in
/// cells from that default, for resizing the box itself when there's no pane divider to resize
/// instead (see `ShellPanes::resize_focused`'s `bool` return) — and centered plus `offset`, a
/// `(dx, dy)` nudge in cells, applied and then clamped so the box can never be dragged off screen
/// — rather than the full browser area, so the browser stays visible around it just like the old
/// single popup did. Panes still split/tile normally, just within this smaller box instead of
/// across the whole screen. `draw` calls this same function for rendering, so a pane's pty size
/// can never drift from its actual rendered rect.
fn shell_area(frame_area: Rect, offset: (i32, i32), size_adjust: (i32, i32)) -> Rect {
    let browser_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(frame_area)[0];

    let default_width = browser_area.width.saturating_mul(4) / 5;
    let default_height = browser_area.height.saturating_mul(7) / 10;

    // Clamped to `u16::MAX` before the cast (size_adjust, like offset, is never itself clamped at
    // the accumulator — see the resize-chord handling in `run` — so an extreme value must not
    // overflow the cast), then to the actual browser area on the large end.
    let width = ((default_width as i32 + size_adjust.0)
        .clamp(MIN_SHELL_BOX_WIDTH as i32, u16::MAX as i32) as u16)
        .min(browser_area.width);
    let height = ((default_height as i32 + size_adjust.1)
        .clamp(MIN_SHELL_BOX_HEIGHT as i32, u16::MAX as i32) as u16)
        .min(browser_area.height);

    let min_x = browser_area.x as i32;
    let max_x = (browser_area.x + browser_area.width).saturating_sub(width) as i32;
    let centered_x = browser_area.x as i32 + (browser_area.width as i32 - width as i32) / 2;
    let x = (centered_x + offset.0).clamp(min_x, max_x.max(min_x));

    let min_y = browser_area.y as i32;
    let max_y = (browser_area.y + browser_area.height).saturating_sub(height) as i32;
    let centered_y = browser_area.y as i32 + (browser_area.height as i32 - height as i32) / 2;
    let y = (centered_y + offset.1).clamp(min_y, max_y.max(min_y));

    Rect::new(x as u16, y as u16, width, height)
}

/// The tiled shell panes plus the box's current offset/size adjustment from its centered default
/// (see `shell_area`), bundled together since `draw` only ever needs all three or none — keeps
/// its own argument list from growing every time the shell overlay gains one more piece of state.
struct ShellView<'a> {
    panes: &'a ShellPanes,
    offset: (i32, i32),
    size: (i32, i32),
}

/// Which half of the leader chord (`space` while panes are open, see the `Leader` action) is
/// active — `hjkl`/arrows mean something different depending on which. Entered and left entirely
/// at the app layer, so it can never eat a keystroke a real shell would otherwise receive.
#[derive(Debug, Clone, Copy)]
enum ShellChordMode {
    /// `hjkl`/arrows nudge the focused pane's nearest divider (`ShellPanes::resize_focused`) —
    /// or, when there's no divider along that axis to adjust (a lone pane, or every split so far
    /// is the other axis), the box's own width/height instead, so resize mode always does
    /// something regardless of how panes are currently split.
    Resize,
    /// `hjkl`/arrows nudge the whole box's offset, like dragging its title bar one step at a
    /// time.
    Move,
}

/// The distance a single `hjkl`/arrow press moves the box in move-chord mode, in cells —
/// deliberately coarser than `RESIZE_STEP`'s 5%, since a cell is already the finest unit an
/// offset moves in.
const SHELL_MOVE_STEP: i32 = 2;

/// The distance a single `hjkl`/arrow press grows/shrinks the box itself, in cells, when resize
/// mode falls back to it (see `ShellChordMode::Resize`) — the same unit `SHELL_MOVE_STEP` uses,
/// since both are nudging the same box.
const SHELL_BOX_RESIZE_STEP: i32 = 2;

/// Applies one step of an active resize/move chord. `frame_area` is the raw terminal size, not
/// the already-`shell_area`-computed box rect — resizing the box itself needs to recompute that
/// rect from scratch with the updated `shell_size`, so there's nothing for a caller to usefully
/// pre-compute here. The chord's own `r`/`m`/`hjkl` keys are fixed rather than user-configurable
/// — same precedent as the shell-pane `Esc` handling below, which is also hardcoded — so only the
/// chord's entry point (`Leader`, i.e. `space`) goes through the configurable keymap.
fn apply_shell_chord(
    mode: ShellChordMode,
    dir: NudgeDir,
    shells: &mut Option<ShellPanes>,
    shell_offset: &mut (i32, i32),
    shell_size: &mut (i32, i32),
    frame_area: Rect,
) -> Result<()> {
    match mode {
        ShellChordMode::Resize => {
            let area = shell_area(frame_area, *shell_offset, *shell_size);
            let resized_a_divider = match shells.as_mut() {
                Some(panes) => panes.resize_focused(dir, area)?,
                None => false,
            };
            if !resized_a_divider {
                match dir {
                    NudgeDir::Left => shell_size.0 -= SHELL_BOX_RESIZE_STEP,
                    NudgeDir::Right => shell_size.0 += SHELL_BOX_RESIZE_STEP,
                    NudgeDir::Up => shell_size.1 -= SHELL_BOX_RESIZE_STEP,
                    NudgeDir::Down => shell_size.1 += SHELL_BOX_RESIZE_STEP,
                }
                if let Some(panes) = shells.as_ref() {
                    panes.resize(shell_area(frame_area, *shell_offset, *shell_size))?;
                }
            }
        }
        // Only the offset changes here, never the box's size — exactly like dragging the title
        // bar already does, which is why this never needs to call `panes.resize`.
        ShellChordMode::Move => match dir {
            NudgeDir::Left => shell_offset.0 -= SHELL_MOVE_STEP,
            NudgeDir::Right => shell_offset.0 += SHELL_MOVE_STEP,
            NudgeDir::Up => shell_offset.1 -= SHELL_MOVE_STEP,
            NudgeDir::Down => shell_offset.1 += SHELL_MOVE_STEP,
        },
    }
    Ok(())
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
    // The box's `(dx, dy)` offset from centered, in cells — dragging its top border (where the
    // "shell" title renders) moves the whole tiled box like a floating window's title bar,
    // distinct from dragging a divider between two panes inside it. Resets to `(0, 0)` on every
    // new `s` spawn.
    let mut shell_offset: (i32, i32) = (0, 0);
    // The box's `(dw, dh)` size adjustment from its centered default, in cells — the resize
    // chord's fallback for growing/shrinking the box itself when there's no pane divider along
    // the pressed axis to adjust instead (see `apply_shell_chord`). Resets to `(0, 0)` on every
    // new `s` spawn, same as `shell_offset`.
    let mut shell_size: (i32, i32) = (0, 0);
    // The mouse position `(col, row)` last seen while dragging the box's title bar, to compute
    // the next frame's delta. `None` means no such drag is in progress.
    let mut dragging_shell: Option<(u16, u16)> = None;
    // Set for exactly one keystroke after `Leader` (space) is pressed while panes are open,
    // waiting to see whether it's followed by `r` (resize chord) or `m` (move chord); any other
    // key just drops it. See `ShellChordMode`.
    let mut pending_leader = false;
    // The active resize/move chord, if any — `hjkl`/arrows are interpreted specially while this
    // is `Some`, instead of driving the browser or forwarding to a shell.
    let mut shell_chord: Option<ShellChordMode> = None;

    loop {
        app.poll_bulk(browser, vfs)?;
        previews.update(browser.selected_entry().map(|e| e.path.as_path()));

        if let Some(mut panes) = shells.take() {
            let area = shell_area(terminal.size()?.into(), shell_offset, shell_size);
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
        if shells.is_none() {
            // A pane can exit on its own (e.g. `exit` typed into it) between keystrokes, which
            // would otherwise leave a resize/move chord dangling with nothing left to act on.
            pending_leader = false;
            shell_chord = None;
        }

        terminal.draw(|frame| {
            draw(
                frame,
                browser,
                app,
                previews,
                vfs,
                config,
                shells.as_ref().map(|panes| ShellView {
                    panes,
                    offset: shell_offset,
                    size: shell_size,
                }),
            )
        })?;

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
                    panes.resize(shell_area(Rect::new(0, 0, cols, rows), shell_offset, shell_size))?;
                }
            }
            Event::Mouse(mouse) => {
                let Some(panes) = shells.as_mut() else {
                    continue;
                };
                let area = shell_area(terminal.size()?.into(), shell_offset, shell_size);
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(divider) = panes
                            .dividers(area)
                            .into_iter()
                            .find(|d| d.hit(mouse.column, mouse.row))
                        {
                            dragging_divider = Some(divider.id());
                        } else if mouse.row == area.y
                            && mouse.column >= area.x
                            && mouse.column < area.x + area.width
                        {
                            // The box's top border (where the "shell" title renders) — grabbing
                            // it moves the whole box, like a floating window's title bar.
                            dragging_shell = Some((mouse.column, mouse.row));
                            if panes.focus_at(area, mouse.column, mouse.row) {
                                shell_focused = true;
                            }
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
                        } else if let Some((last_col, last_row)) = dragging_shell {
                            shell_offset.0 += mouse.column as i32 - last_col as i32;
                            shell_offset.1 += mouse.row as i32 - last_row as i32;
                            dragging_shell = Some((mouse.column, mouse.row));
                        }
                    }
                    MouseEventKind::Up(MouseButton::Left) => {
                        dragging_divider = None;
                        dragging_shell = None;
                    }
                    _ => {}
                }
            }
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if pending_leader {
                    pending_leader = false;
                    if shells.is_some() {
                        match key.code {
                            KeyCode::Char('r') => {
                                shell_chord = Some(ShellChordMode::Resize);
                                app.status = Some("resize mode — hjkl to size, Esc to exit".into());
                            }
                            KeyCode::Char('m') => {
                                shell_chord = Some(ShellChordMode::Move);
                                app.status = Some("move mode — hjkl to move, Esc to exit".into());
                            }
                            _ => {}
                        }
                    }
                    continue;
                }

                if let Some(mode) = shell_chord {
                    let frame_area: Rect = terminal.size()?.into();
                    let stayed = match key.code {
                        KeyCode::Esc => {
                            shell_chord = None;
                            false
                        }
                        KeyCode::Char('h') | KeyCode::Left => {
                            apply_shell_chord(
                                mode,
                                NudgeDir::Left,
                                &mut shells,
                                &mut shell_offset,
                                &mut shell_size,
                                frame_area,
                            )?;
                            true
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            apply_shell_chord(
                                mode,
                                NudgeDir::Down,
                                &mut shells,
                                &mut shell_offset,
                                &mut shell_size,
                                frame_area,
                            )?;
                            true
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            apply_shell_chord(
                                mode,
                                NudgeDir::Up,
                                &mut shells,
                                &mut shell_offset,
                                &mut shell_size,
                                frame_area,
                            )?;
                            true
                        }
                        KeyCode::Char('l') | KeyCode::Right => {
                            apply_shell_chord(
                                mode,
                                NudgeDir::Right,
                                &mut shells,
                                &mut shell_offset,
                                &mut shell_size,
                                frame_area,
                            )?;
                            true
                        }
                        _ => {
                            // Any other key ends the chord but still gets dispatched normally
                            // below (e.g. `q` should still quit), unlike `Esc`, which only means
                            // "leave this mode."
                            shell_chord = None;
                            false
                        }
                    };
                    if stayed || key.code == KeyCode::Esc {
                        continue;
                    }
                }

                if shells.is_some() {
                    let area = shell_area(terminal.size()?.into(), shell_offset, shell_size);
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
                    // Starts the resize/move chord for shell panes (see `pending_leader` and
                    // `ShellChordMode`). Stays inert — same as before panes existed to chord
                    // against — whenever no shell is open.
                    Some(Action::Leader) => {
                        if shells.is_some() {
                            pending_leader = true;
                        }
                    }
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
                        // Every fresh spawn starts centered at its default size, same as before
                        // the box was made draggable/resizable.
                        shell_offset = (0, 0);
                        shell_size = (0, 0);
                        let area = shell_area(terminal.size()?.into(), shell_offset, shell_size);
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
    shell: Option<ShellView<'_>>,
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

    // Drawn last, over the browser columns above — the tiled shell panes, in a box that never
    // covers the status bar and can be dragged off-center by its title bar (see `shell_area`).
    if let Some(ShellView { panes, offset, size }) = shell {
        panes.render(frame, shell_area(frame.area(), offset, size), config);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_area_centers_with_zero_offset() {
        let area = shell_area(Rect::new(0, 0, 100, 40), (0, 0), (0, 0));
        // 80% width, 70% height of the 39-row browser area (40 minus the 1-row status bar).
        assert_eq!(area, Rect::new(10, 6, 80, 27));
    }

    #[test]
    fn shell_area_applies_a_positive_offset() {
        let centered = shell_area(Rect::new(0, 0, 100, 40), (0, 0), (0, 0));
        let moved = shell_area(Rect::new(0, 0, 100, 40), (5, 3), (0, 0));
        assert_eq!(moved.x, centered.x + 5);
        assert_eq!(moved.y, centered.y + 3);
        assert_eq!((moved.width, moved.height), (centered.width, centered.height));
    }

    #[test]
    fn shell_area_clamps_offset_so_the_box_never_leaves_the_screen() {
        let frame = Rect::new(0, 0, 100, 40);
        let browser_bottom = 39; // rows[0]'s bottom row, one above the status bar.

        let far_right_down = shell_area(frame, (10_000, 10_000), (0, 0));
        assert_eq!(far_right_down.x + far_right_down.width, 100);
        assert_eq!(far_right_down.y + far_right_down.height, browser_bottom);

        let far_left_up = shell_area(frame, (-10_000, -10_000), (0, 0));
        assert_eq!(far_left_up.x, 0);
        assert_eq!(far_left_up.y, 0);
    }

    #[test]
    fn shell_area_applies_a_positive_size_adjust_to_both_dimensions() {
        let default = shell_area(Rect::new(0, 0, 100, 40), (0, 0), (0, 0));
        let grown = shell_area(Rect::new(0, 0, 100, 40), (0, 0), (10, 4));
        assert_eq!(grown.width, default.width + 10);
        assert_eq!(grown.height, default.height + 4);
        // Growing still re-centers around the same point, not just growing from one corner.
        assert_eq!(grown.x, default.x - 5);
        assert_eq!(grown.y, default.y - 2);
    }

    #[test]
    fn shell_area_clamps_size_adjust_to_the_browser_area_and_the_stated_minimum() {
        let frame = Rect::new(0, 0, 100, 40);

        let huge = shell_area(frame, (0, 0), (10_000, 10_000));
        assert_eq!(huge.width, 100);
        assert_eq!(huge.height, 39); // the 39-row browser area, one above the status bar.

        let tiny = shell_area(frame, (0, 0), (-10_000, -10_000));
        assert_eq!(tiny.width, MIN_SHELL_BOX_WIDTH);
        assert_eq!(tiny.height, MIN_SHELL_BOX_HEIGHT);
    }
}
