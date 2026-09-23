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

mod alt_keys;
mod app;
mod appearance_popup;
mod browser_mouse;
mod cli;
mod command;
mod context_menu;
mod disk_usage;
mod disk_usage_view;
mod git_status;
mod glyphs;
mod hud;
mod image_preview;
mod inspect;
mod live_refresh;
mod marked_size;
mod open;
mod osc52;
mod overlay_view;
mod popup_shell;
mod preview_view;
mod search_job;
mod settings_popup;
mod shell_init;
mod shell_layout;
mod style;
mod terminal_init;
mod text_preview;

use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::{Duration, Instant, SystemTime};

use alt_keys::AltCommand;
use anyhow::Result;
use app::App;
use appearance_popup::{
    AppearancePopup, AppearanceView, Hit as AppearanceHit, Outcome as AppearanceOutcome,
    Row as AppearanceRow, RowKind as AppearanceRowKind, SaveChoice, SaveTarget,
};
use browser::BrowserState;
use browser_mouse::{BrowserLayout, Click, ClickTracker, Hit, Listing, Pane, Wheel};
use context_menu::{
    Context as MenuContext, ContextMenu, MenuCommand, Nav, Outcome as MenuOutcome,
    Target as MenuTarget,
};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyEventState,
    KeyModifiers, KeyboardEnhancementFlags, ModifierKeyCode, MouseButton, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use disk_usage::DiskUsageView;
use image_preview::{ImagePreview, PreviewStatus as ImagePreviewStatus};
use inspect::InspectView;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui_image::StatefulImage;
use settings_popup::{Outcome as SettingsOutcome, Row as SettingsRow, SettingsPopup, SettingsView};
use shared::{DirEntryInfo, LocalVfs};
use shell_layout::{NudgeDir, ShellPanes, SplitDirection};
use text_preview::{PreviewStatus as TextPreviewStatus, TextPreview};
use theming::{
    Action, ColumnLayout, Config, CustomTheme, GlyphSet, PanelsConfig, RawLocal, RawPanels,
    RawTheme, RawUi, Theme, Ui,
};

/// Restores the terminal (raw mode + alternate screen) on drop, so a panic or an early return
/// from `run` never leaves the user's shell in a broken state.
struct TerminalGuard {
    /// Whether `enhance_keyboard` pushed flags that `drop` must pop again.
    keyboard_enhanced: bool,
}

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        Ok(Self {
            keyboard_enhanced: false,
        })
    }

    /// Switches the terminal to the kitty keyboard protocol, which is the only way a bare `Alt`
    /// press-and-release is reported at all. Every flag is needed: all keys as escape codes makes
    /// modifier-only keys visible, event types add the release that says the tap ended, and
    /// alternate keys keep `Shift`+letter arriving as a capital instead of a lowercase letter
    /// with a shift bit. Only call this once the terminal has said it supports the protocol.
    fn enhance_keyboard(&mut self) -> Result<()> {
        execute!(io::stdout(), PushKeyboardEnhancementFlags(keyboard_flags()))?;
        self.keyboard_enhanced = true;
        Ok(())
    }

    /// Hands the real terminal to `line` (run under `sh -c` in `cwd`) and takes it back when it
    /// exits: leaves the alternate screen, mouse capture, raw mode and the keyboard protocol so
    /// the program sees an ordinary terminal, then restores all four and forces a full repaint.
    /// The outer `Result` is a terminal failure (fatal — the guard's `Drop` cleans up); the inner
    /// one is the command's own, reported to the user. The terminal is restored even when the
    /// command could not be started.
    fn run_foreground(
        &self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        cwd: &Path,
        line: &str,
    ) -> Result<io::Result<ExitStatus>> {
        if self.keyboard_enhanced {
            execute!(io::stdout(), PopKeyboardEnhancementFlags)?;
        }
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        disable_raw_mode()?;

        let result = shell_overlay::run_foreground(cwd, line);

        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        if self.keyboard_enhanced {
            execute!(io::stdout(), PushKeyboardEnhancementFlags(keyboard_flags()))?;
        }
        // `resize` rather than `clear`: it empties the screen and the back buffer so the next
        // frame repaints everything, without `clear`'s cursor-position query, which a terminal
        // that is slow to answer can turn into a hang.
        terminal.resize(Rect::from(terminal.size()?))?;
        Ok(result)
    }
}

/// The kitty keyboard protocol flags Minuteman runs with (see `enhance_keyboard`).
fn keyboard_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.keyboard_enhanced {
            let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
        }
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
    let browser_area = shell_bounds(frame_area);
    let (default_width, default_height) = default_shell_size(browser_area);

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

/// The region the shell box must stay inside: the frame minus the one-row status bar.
fn shell_bounds(frame_area: Rect) -> Rect {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(frame_area)[0]
}

/// The box's size when `size_adjust` is `(0, 0)`: 80% of `bounds`' width, 70% of its height.
fn default_shell_size(bounds: Rect) -> (u16, u16) {
    (
        bounds.width.saturating_mul(4) / 5,
        bounds.height.saturating_mul(7) / 10,
    )
}

/// The inverse of `shell_area`: the `(offset, size_adjust)` pair that makes it return `target`.
/// `shell_area` only knows "centered, then nudged", so anything that wants a box pinned to one
/// edge or anchored at one corner — growing a single side, snapping to the top — has to be
/// expressed as the offset and size adjustment that produce that rect. `target` must already lie
/// inside `shell_bounds` and respect the minimum size, or `shell_area` will clamp it again.
fn shell_params_for(frame_area: Rect, target: Rect) -> ((i32, i32), (i32, i32)) {
    let bounds = shell_bounds(frame_area);
    let (default_width, default_height) = default_shell_size(bounds);
    let centered_x = bounds.x as i32 + (bounds.width as i32 - target.width as i32) / 2;
    let centered_y = bounds.y as i32 + (bounds.height as i32 - target.height as i32) / 2;
    (
        (target.x as i32 - centered_x, target.y as i32 - centered_y),
        (
            target.width as i32 - default_width as i32,
            target.height as i32 - default_height as i32,
        ),
    )
}

/// Grows the side of the box named by `dir` outward by `SHELL_BOX_RESIZE_STEP`, leaving the other
/// three sides where they are — unlike the chord's fallback, which grows symmetrically. Stops at
/// the edge of the screen. Growth only: shrinking is `resize_box_corner`'s job.
fn grow_box_edge(
    frame_area: Rect,
    offset: (i32, i32),
    size_adjust: (i32, i32),
    dir: NudgeDir,
) -> ((i32, i32), (i32, i32)) {
    let bounds = shell_bounds(frame_area);
    let area = shell_area(frame_area, offset, size_adjust);
    let (bounds_right, bounds_bottom) = (
        bounds.x as i32 + bounds.width as i32,
        bounds.y as i32 + bounds.height as i32,
    );
    let (mut left, mut top) = (area.x as i32, area.y as i32);
    let (mut right, mut bottom) = (left + area.width as i32, top + area.height as i32);
    match dir {
        NudgeDir::Left => left = (left - SHELL_BOX_RESIZE_STEP).max(bounds.x as i32),
        NudgeDir::Right => right = (right + SHELL_BOX_RESIZE_STEP).min(bounds_right),
        NudgeDir::Up => top = (top - SHELL_BOX_RESIZE_STEP).max(bounds.y as i32),
        NudgeDir::Down => bottom = (bottom + SHELL_BOX_RESIZE_STEP).min(bounds_bottom),
    }
    let target = Rect::new(
        left as u16,
        top as u16,
        (right - left) as u16,
        (bottom - top) as u16,
    );
    shell_params_for(frame_area, target)
}

/// Moves the box's bottom-right corner by `delta` cells while its top-left stays put — what
/// `Alt`+right-drag does, so dragging right/down grows it and left/up shrinks it. Clamped to the
/// minimum size and to the screen.
fn resize_box_corner(
    frame_area: Rect,
    offset: (i32, i32),
    size_adjust: (i32, i32),
    delta: (i32, i32),
) -> ((i32, i32), (i32, i32)) {
    let bounds = shell_bounds(frame_area);
    let area = shell_area(frame_area, offset, size_adjust);
    let room_width =
        (bounds.x as i32 + bounds.width as i32 - area.x as i32).max(MIN_SHELL_BOX_WIDTH as i32);
    let room_height =
        (bounds.y as i32 + bounds.height as i32 - area.y as i32).max(MIN_SHELL_BOX_HEIGHT as i32);
    let width = (area.width as i32 + delta.0).clamp(MIN_SHELL_BOX_WIDTH as i32, room_width);
    let height = (area.height as i32 + delta.1).clamp(MIN_SHELL_BOX_HEIGHT as i32, room_height);
    shell_params_for(
        frame_area,
        Rect::new(area.x, area.y, width as u16, height as u16),
    )
}

/// The offset that pins the box to the top or bottom edge of the screen while keeping it
/// horizontally centered, at its current size. Computed exactly rather than by pushing the offset
/// to a huge value and letting `shell_area` clamp it, because the offset accumulator is never
/// itself clamped — an inflated value would take just as many key presses to come back from.
fn snap_box_offset(
    frame_area: Rect,
    offset: (i32, i32),
    size_adjust: (i32, i32),
    to_top: bool,
) -> (i32, i32) {
    let bounds = shell_bounds(frame_area);
    let area = shell_area(frame_area, offset, size_adjust);
    let x = bounds.x as i32 + (bounds.width as i32 - area.width as i32) / 2;
    let y = if to_top {
        bounds.y as i32
    } else {
        bounds.y as i32 + bounds.height as i32 - area.height as i32
    };
    shell_params_for(
        frame_area,
        Rect::new(x as u16, y as u16, area.width, area.height),
    )
    .0
}

/// The tiled shell panes plus the box's current offset/size adjustment from its centered default
/// (see `shell_area`), bundled together since `draw` only ever needs all three or none — keeps
/// its own argument list from growing every time the shell overlay gains one more piece of state.
/// Everything `draw` needs about the interactive state layered over the browser: what the app is
/// doing (for the status bar), the shell panes if any are open, and the middle column's scroll
/// state, which `draw` updates and the mouse handler reads back to find the row under a click.
struct Overlay<'a> {
    mode: hud::Mode,
    shell: Option<ShellView<'a>>,
    current_list: &'a mut ListState,
    /// The right-click menu, the Inspect panel and the settings popup, drawn last so they sit
    /// over everything.
    menu: Option<&'a ContextMenu>,
    inspect: Option<&'a InspectView>,
    /// The disk usage view, which covers the whole screen when open.
    usage: Option<&'a DiskUsageView>,
    settings: Option<&'a SettingsPopup>,
    appearance: Option<&'a AppearancePopup>,
    /// `local.toml`'s three tables, live for the running session — see `run`'s `local_theme`/
    /// `local_ui`/`local_panels` and `effective_theme`/`effective_ui`/`effective_panels`.
    local_theme: &'a RawTheme,
    local_ui: &'a RawUi,
    local_panels: &'a RawPanels,
    /// The appearance popup's saved custom themes and which one (if any) is active — see
    /// `RawLocal::custom_themes`/`active_custom_theme`.
    local_custom_themes: &'a [CustomTheme],
    local_active_custom_theme: Option<&'a str>,
}

/// The panels a menu command can open, passed together so `run_menu_command` does not grow an
/// argument for each one.
struct Panels<'a> {
    inspect: &'a mut Option<InspectView>,
    usage: &'a mut Option<DiskUsageView>,
    appearance: &'a mut Option<AppearancePopup>,
}

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

/// Opens a shell box from scratch: centered at its default size, like every fresh spawn.
fn fresh_shell_box(
    cwd: &Path,
    frame_area: Rect,
    shell_offset: &mut (i32, i32),
    shell_size: &mut (i32, i32),
) -> Result<ShellPanes> {
    *shell_offset = (0, 0);
    *shell_size = (0, 0);
    ShellPanes::open(cwd, shell_area(frame_area, *shell_offset, *shell_size))
}

/// Carries out one `Alt` command (see `alt_keys`) against the shell box, returning a status
/// message when there is something worth telling the user. Works the same whether the shell has
/// the keyboard or not, since `Alt` never reaches a shell at all. `hovered` is where the pointer
/// last was, for the one command (`CloseHovered`) that acts on whatever is under it.
fn apply_alt_command(
    command: AltCommand,
    shells: &mut Option<ShellPanes>,
    shell_offset: &mut (i32, i32),
    shell_size: &mut (i32, i32),
    hovered: Option<(u16, u16)>,
    cwd: &Path,
    frame_area: Rect,
) -> Result<Option<String>> {
    let Some(mut panes) = shells.take() else {
        // With nothing open there is nothing to move or close, but a new shell is exactly what
        // `Split` is for, so it doubles as "open the first one".
        if command != AltCommand::Split {
            return Ok(Some("no shell open".into()));
        }
        return Ok(
            match fresh_shell_box(cwd, frame_area, shell_offset, shell_size) {
                Ok(opened) => {
                    *shells = Some(opened);
                    None
                }
                Err(e) => Some(format!("failed to start shell: {e}")),
            },
        );
    };

    let area = shell_area(frame_area, *shell_offset, *shell_size);
    let status = match command {
        AltCommand::Move(dir) => {
            match dir {
                NudgeDir::Left => shell_offset.0 -= SHELL_MOVE_STEP,
                NudgeDir::Right => shell_offset.0 += SHELL_MOVE_STEP,
                NudgeDir::Up => shell_offset.1 -= SHELL_MOVE_STEP,
                NudgeDir::Down => shell_offset.1 += SHELL_MOVE_STEP,
            }
            None
        }
        AltCommand::Grow(dir) => {
            (*shell_offset, *shell_size) =
                grow_box_edge(frame_area, *shell_offset, *shell_size, dir);
            panes.resize(shell_area(frame_area, *shell_offset, *shell_size))?;
            None
        }
        AltCommand::ShrinkWidth | AltCommand::ShrinkHeight => {
            // The same corner resize the mouse does, so the top-left stays put and the box can
            // never drop below its minimum size.
            let delta = if command == AltCommand::ShrinkWidth {
                (-SHELL_BOX_RESIZE_STEP, 0)
            } else {
                (0, -SHELL_BOX_RESIZE_STEP)
            };
            (*shell_offset, *shell_size) =
                resize_box_corner(frame_area, *shell_offset, *shell_size, delta);
            panes.resize(shell_area(frame_area, *shell_offset, *shell_size))?;
            None
        }
        AltCommand::Focus(dir) => {
            (!panes.focus_direction(dir, area)).then(|| "no shell that way".into())
        }
        AltCommand::Split => {
            // Terminal cells are about twice as tall as wide, so a pane at least twice as wide as
            // it is tall is the one that looks landscape and should be cut side by side.
            let rect = panes.focused_rect(area);
            let direction = if rect.width >= rect.height.saturating_mul(2) {
                SplitDirection::Horizontal
            } else {
                SplitDirection::Vertical
            };
            panes = panes.split(direction, cwd, area)?;
            None
        }
        AltCommand::SnapTop | AltCommand::SnapBottom => {
            *shell_offset = snap_box_offset(
                frame_area,
                *shell_offset,
                *shell_size,
                command == AltCommand::SnapTop,
            );
            None
        }
        AltCommand::CloseAll => {
            panes.close_all(area)?;
            return Ok(Some("shells closed".into()));
        }
        AltCommand::CloseHovered => {
            // Off the box entirely (or before any mouse movement) there is nothing hovered, so
            // fall back to the focused pane rather than making a keyboard-only user unable to
            // close anything.
            if let Some((col, row)) = hovered {
                panes.focus_at(area, col, row);
            }
            let id = panes.focused_id();
            match panes.close(id, area)? {
                Some(rest) => {
                    panes = rest;
                    Some("pane closed".into())
                }
                None => return Ok(Some("shell closed".into())),
            }
        }
    };
    *shells = Some(panes);
    Ok(status)
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

    /// The terminal's background color, from the image pipeline's own startup probe (see
    /// `ImagePreview::new`). `None` if the terminal never answered the OSC 11 query.
    fn detected_background(&self) -> Option<(u8, u8, u8)> {
        self.image.detected_background()
    }

    /// `selected` is the entry under the cursor. An image goes to the image pipeline; any other
    /// file (text, an archive, a binary) to the one that reads it for the scrolling pane; a folder
    /// to neither, since the pane lists its children instead.
    fn update(&mut self, selected: Option<&DirEntryInfo>) {
        let file = selected.filter(|e| !e.is_dir).map(|e| e.path.as_path());
        self.image.update(file);
        self.text
            .update(file.filter(|path| !preview::is_image(path)));
    }

    /// Re-reads whichever preview is showing the selected file, after it changed on disk.
    fn reload(&mut self) {
        self.image.reload();
        self.text.reload();
    }
}

fn main() -> Result<()> {
    let args = match cli::parse(std::env::args_os().skip(1)) {
        Ok(cli::Command::Init(shell)) => {
            print!("{}", shell.wrapper());
            return Ok(());
        }
        Ok(cli::Command::InitTerminal(terminal)) => {
            // The font comes from `[font]` in the user's appearance.toml.
            print!("{}", terminal.snippet(&Config::load().font));
            return Ok(());
        }
        Ok(cli::Command::InitAppearance) => {
            print!("{}", include_str!("../../../appearance.example.toml"));
            return Ok(());
        }
        Ok(cli::Command::Glyphs) => {
            for set in [GlyphSet::Unicode, GlyphSet::Nerd, GlyphSet::Ascii] {
                println!("{}", glyphs::sample(set));
            }
            return Ok(());
        }
        Ok(cli::Command::Run(args)) => args,
        Err(e) => {
            eprintln!("minuteman: {e}");
            std::process::exit(2);
        }
    };

    let start_dir = match args.start_dir {
        Some(dir) => dir,
        None => std::env::current_dir()?,
    };
    let start_dir = start_dir.canonicalize().unwrap_or(start_dir);

    let mut config = Config::load();
    let vfs = LocalVfs;
    let mut browser = BrowserState::with_show_hidden(&vfs, start_dir, config.show_hidden)?;

    // Backs the blocking thread pool that copy/move/delete run on so a large operation never
    // freezes the render loop. Kept alive for the rest of `main` — dropping it would shut the
    // pool down out from under any operation still running.
    let runtime = tokio::runtime::Runtime::new()?;
    let mut app = App::new(runtime.handle().clone())
        .with_git_status(config.git_status)
        .with_interactive_commands(config.interactive_commands.clone())
        .with_plugins(config.plugins.clone(), browser.current_dir());

    let mut guard = TerminalGuard::new()?;
    // Both probes must run after entering the alternate screen but before the event loop reads
    // any input — each briefly reads/writes stdio itself. The keyboard one goes first so its
    // reply is fully consumed before the graphics probe starts reading stdio directly.
    let keyboard_protocol = config.alt_tap && supports_keyboard_enhancement().unwrap_or(false);
    let mut previews = Previews::new(runtime.handle().clone());
    if keyboard_protocol {
        guard.enhance_keyboard()?;
    }
    // Nothing in `[theme]` was set, so pick a look from the terminal's own background instead of
    // always falling back to the static neon default — an explicit `name` or even one overridden
    // field always wins over this (see `Config::theme_is_customized`).
    if !config.theme_is_customized {
        let is_dark = previews
            .detected_background()
            .map(|(r, g, b)| Theme::is_dark(r, g, b));
        config.theme = Theme::auto(is_dark);
    }
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = run(
        &mut terminal,
        &vfs,
        &mut browser,
        &mut app,
        &mut previews,
        Session {
            config: &config,
            guard: &guard,
            cwd_file: args.cwd_file.as_deref(),
        },
    );

    drop(guard);
    result
}

/// The theme in effect for the running session, before any in-progress (uncommitted) appearance
/// edit is previewed on top: `local_theme.name`, if the appearance popup ever picked one, wins as
/// the base palette over `config.theme`; either way, `local_theme`'s other fields (colors, border
/// style, separator) layer on top of that base.
fn effective_theme(config: &Config, local_theme: &RawTheme) -> Theme {
    let base = local_theme
        .name
        .as_deref()
        .map(Theme::named)
        .unwrap_or_else(|| config.theme.clone());
    base.overlay_raw(local_theme)
}

/// The glyph set in effect: the appearance popup's own pick if it cycled one, else
/// `config.ui.glyphs`.
fn effective_ui(config: &Config, local_ui: &RawUi) -> Ui {
    Ui {
        glyphs: local_ui
            .glyphs
            .as_deref()
            .and_then(GlyphSet::parse)
            .unwrap_or(config.ui.glyphs),
    }
}

/// The panel layout in effect: the settings popup's own picks layered onto `config.panels`.
fn effective_panels(config: &Config, local_panels: &RawPanels) -> PanelsConfig {
    config.panels.overlay_raw(local_panels)
}

/// Writes the settings and appearance popups' combined live overrides to `local.toml`, so they
/// survive to the next launch — called after every commit from either popup. A failure (e.g. a
/// read-only filesystem) is reported in the status line rather than treated as fatal: the change
/// is already applied for the rest of this session regardless of whether it could be saved.
fn persist_local(
    local_theme: &RawTheme,
    local_ui: &RawUi,
    local_panels: &RawPanels,
    local_custom_themes: &[CustomTheme],
    local_active_custom_theme: &Option<String>,
    app: &mut App,
) {
    let local = RawLocal {
        theme: local_theme.clone(),
        ui: local_ui.clone(),
        panels: local_panels.clone(),
        custom_themes: local_custom_themes.to_vec(),
        active_custom_theme: local_active_custom_theme.clone(),
    };
    if let Err(e) = Config::save_local(&local) {
        app.status = Some(format!("could not save to local.toml: {e}"));
    }
}

/// What the appearance popup's "Save theme" row should offer, given the live theme and the
/// custom theme (if any) it's currently tracked against — `main`'s side of `Outcome::WantSaveTheme`,
/// since the popup itself never sees `Config` or the saved themes list.
fn classify_save(theme: &Theme, custom_themes: &[CustomTheme], active: Option<&str>) -> SaveChoice {
    let Some(name) = active else {
        return SaveChoice::New;
    };
    match custom_themes.iter().find(|t| t.name == name) {
        Some(saved) if Theme::from(saved.theme.clone()) == *theme => {
            SaveChoice::Unchanged(name.to_string())
        }
        Some(_) => SaveChoice::UpdateOrNew(name.to_string()),
        // The active pick was reset or deleted from local.toml by hand; nothing to update.
        None => SaveChoice::New,
    }
}

/// Applies one outcome from the appearance popup (`AppearancePopup::key`/`click_row`) to the
/// running session's live state, and saves it — shared by its keyboard and mouse paths in `run`,
/// so a click and the key that reaches the same row behave identically. `local_panels` is only
/// read here (it's the settings popup's own field of `local.toml`), so every save still carries
/// whatever panel layout was last saved even though this outcome didn't touch it.
#[allow(clippy::too_many_arguments)]
fn apply_appearance_outcome(
    outcome: AppearanceOutcome,
    appearance: &mut Option<AppearancePopup>,
    local_theme: &mut RawTheme,
    local_ui: &mut RawUi,
    local_panels: &RawPanels,
    local_custom_themes: &mut Vec<CustomTheme>,
    local_active_custom_theme: &mut Option<String>,
    config: &Config,
    app: &mut App,
) {
    let current_value = |row: AppearanceRow, local_theme: &RawTheme, local_ui: &RawUi| {
        let theme = effective_theme(config, local_theme);
        let ui = effective_ui(config, local_ui);
        let theme_name = appearance_popup::theme_name(local_theme.name.as_deref(), &theme);
        AppearanceView {
            theme: &theme,
            glyphs: ui.glyphs,
            theme_name,
            // Never actually read: `current_value` is only called for a color or cycle row, and
            // `Row::SaveTheme` is neither.
            active_custom_theme: None,
        }
        .value(row)
    };
    match outcome {
        AppearanceOutcome::Stay => {}
        AppearanceOutcome::Close => *appearance = None,
        AppearanceOutcome::WantEdit(row) => {
            let current = current_value(row, local_theme, local_ui);
            if let Some(popup) = appearance.as_mut() {
                popup.begin_edit(current);
            }
        }
        AppearanceOutcome::Cycle(row) => {
            if let Some(AppearanceRowKind::Cycle(options)) = row.kind() {
                let current = current_value(row, local_theme, local_ui);
                let next = appearance_popup::next_in(options, &current);
                if row == AppearanceRow::Glyphs {
                    local_ui.glyphs = Some(next);
                } else {
                    appearance_popup::commit(local_theme, row, next);
                }
                persist_local(
                    local_theme,
                    local_ui,
                    local_panels,
                    local_custom_themes,
                    local_active_custom_theme,
                    app,
                );
            }
        }
        AppearanceOutcome::Commit(row, value) => {
            appearance_popup::commit(local_theme, row, value);
            persist_local(
                local_theme,
                local_ui,
                local_panels,
                local_custom_themes,
                local_active_custom_theme,
                app,
            );
        }
        AppearanceOutcome::Reset => {
            *local_theme = RawTheme::default();
            *local_ui = RawUi::default();
            // The saved custom themes themselves are kept — only which one (if any) the live
            // look is tracked against is cleared, since the live look is now the plain default.
            *local_active_custom_theme = None;
            persist_local(
                local_theme,
                local_ui,
                local_panels,
                local_custom_themes,
                local_active_custom_theme,
                app,
            );
        }
        AppearanceOutcome::WantSaveTheme => {
            let theme = effective_theme(config, local_theme);
            let choice = classify_save(
                &theme,
                local_custom_themes,
                local_active_custom_theme.as_deref(),
            );
            if let SaveChoice::Unchanged(name) = &choice {
                app.status = Some(format!("theme '{name}' is already saved"));
            } else if let Some(popup) = appearance.as_mut() {
                popup.begin_save(choice);
            }
        }
        AppearanceOutcome::SaveTheme(target) => {
            let theme = effective_theme(config, local_theme);
            let raw = RawTheme::from_theme(&theme);
            let name = match target {
                SaveTarget::New(name) => name,
                SaveTarget::Update(name) => name,
            };
            match local_custom_themes.iter_mut().find(|t| t.name == name) {
                Some(existing) => existing.theme = raw,
                None => local_custom_themes.push(CustomTheme {
                    name: name.clone(),
                    theme: raw,
                }),
            }
            *local_active_custom_theme = Some(name.clone());
            persist_local(
                local_theme,
                local_ui,
                local_panels,
                local_custom_themes,
                local_active_custom_theme,
                app,
            );
            app.status = Some(format!("saved theme '{name}'"));
        }
    }
}

/// What `run` reads from `main` but never changes.
struct Session<'a> {
    config: &'a Config,
    /// Needed to hand the terminal to a `:` command (see `TerminalGuard::run_foreground`).
    guard: &'a TerminalGuard,
    /// The `--cwd-file` to record the browsed directory in on `Q`, if one was given.
    cwd_file: Option<&'a Path>,
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    vfs: &LocalVfs,
    browser: &mut BrowserState,
    app: &mut App,
    previews: &mut Previews,
    session: Session<'_>,
) -> Result<()> {
    let Session {
        config,
        guard,
        cwd_file,
    } = session;
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
    // The mouse column last seen while dragging the box's own right border, to resize its width
    // (see `shell_size`). `None` means no such drag is in progress.
    let mut dragging_shell_width: Option<u16> = None;
    // The mouse position last seen while `Alt`+right-dragging to resize the box from its
    // bottom-right corner (see `resize_box_corner`). `None` means no such drag is in progress.
    let mut alt_resizing: Option<(u16, u16)> = None;
    // Set when `Alt` is pressed on its own and cleared by any other key or mouse press, so that
    // releasing it while still set means it was tapped alone — which switches between typing in
    // the shell and using the browser. Only ever set when the terminal reports bare modifiers.
    let mut alt_tap_armed = false;
    // Where the pointer last was, from any mouse event — what "hovered" means to `Alt+m`. `None`
    // until the mouse first moves.
    let mut mouse_pos: Option<(u16, u16)> = None;
    // Set for exactly one keystroke after `Leader` (space) is pressed while panes are open,
    // waiting to see whether it's followed by `r` (resize chord), `m` (move chord), or `t` (an
    // immediate orientation toggle, no chord); any other key just drops it. See `ShellChordMode`.
    let mut pending_leader = false;
    // The active resize/move chord, if any — `hjkl`/arrows are interpreted specially while this
    // is `Some`, instead of driving the browser or forwarding to a shell.
    let mut shell_chord: Option<ShellChordMode> = None;
    // Lets `app.status` messages clear themselves after a few seconds instead of lingering.
    let mut status_clock = hud::StatusClock::new(Instant::now());
    // Kept across frames rather than rebuilt in `draw`, because a click has to be mapped back to
    // a row and only the state that scrolled the list knows which entry is at its top.
    let mut current_list = ListState::default();
    // Tells a double-click on a row from two single clicks (see `browser_mouse`).
    let mut clicks = ClickTracker::default();
    // The right-click menu while one is open. It owns the mouse and the keyboard until it
    // closes, so nothing else has to know it exists.
    let mut menu: Option<ContextMenu> = None;
    // The Inspect panel while one is open; modal in the same way.
    let mut inspect: Option<InspectView> = None;
    // The disk usage view while one is open; modal, and drawn over the whole screen.
    let mut usage: Option<DiskUsageView> = None;
    // The settings popup (`Space` then `t` with no shell pane open) while one is open; modal too.
    let mut settings: Option<SettingsPopup> = None;
    // The appearance popup (`a`, or "Appearance…" on blank space's right-click menu) while one is
    // open; modal too.
    let mut appearance: Option<AppearancePopup> = None;
    // `local.toml`'s three tables, seeded from what `Config::load` already parsed from it (so a
    // save this session correctly carries forward whatever an earlier session saved) and mutated
    // live by the two popups; `effective_theme`/`effective_ui`/`effective_panels` layer each onto
    // `config`'s own resolution, and `persist_local` writes them back out after every commit.
    // `theme`/`ui`/`panels` are read this same way; only their `local_*` alter egos are ever
    // written to, which is what keeps `local.toml` sparse — see `Config::local_theme`'s doc.
    let mut local_theme = config.local_theme.clone();
    let mut local_ui = config.local_ui.clone();
    let mut local_panels = config.local_panels.clone();
    let mut local_custom_themes = config.local_custom_themes.clone();
    let mut local_active_custom_theme = config.local_active_custom_theme.clone();

    loop {
        app.poll_bulk(browser, vfs)?;
        app.poll_search(browser, vfs);
        if let Some(view) = inspect.as_mut() {
            view.poll();
        }
        if let Some(view) = usage.as_mut() {
            view.poll();
        }
        // A `:` command that needs the whole terminal (`:nvim notes.md`, `:!python3`): the
        // browser steps aside until it exits. Checked here, once per turn of the loop, because
        // the app that recognised the command doesn't own the terminal.
        if let Some(handover) = app.take_handover() {
            let result = guard.run_foreground(terminal, &handover.cwd, &handover.line)?;
            previews.reload();
            app.finish_handover(browser, vfs, &handover, &result)?;
        }
        // Picks up files made or changed by a mini-shell, a `:` command or another program; a
        // change to the selected file itself needs its preview re-read, since that is otherwise
        // keyed on the path alone.
        if app.poll_disk(browser, vfs, Instant::now()) {
            previews.reload();
        }
        app.poll_hud(browser, Instant::now());
        app.poll_plugins();
        previews.update(browser.selected_entry());

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

        status_clock.tick(&mut app.status, Instant::now());

        // Mirrors the order the key handling below checks these in, so the pill always names
        // what the next keystroke will actually do.
        let mode = if shells.is_some() && pending_leader {
            hud::Mode::Leader
        } else if let (Some(chord), true) = (shell_chord, shells.is_some()) {
            match chord {
                ShellChordMode::Resize => hud::Mode::Resize,
                ShellChordMode::Move => hud::Mode::Move,
            }
        } else if shells.is_some() && shell_focused {
            hud::Mode::Shell
        } else if let Some(prompt) = &app.prompt {
            hud::Mode::Prompt {
                label: prompt.label(),
                destructive: prompt.is_destructive(),
                text_input: prompt.is_text_input(),
            }
        } else if app.is_busy() {
            hud::Mode::Busy
        } else {
            hud::Mode::Normal
        };

        terminal.draw(|frame| {
            draw(
                frame,
                browser,
                app,
                previews,
                vfs,
                config,
                Overlay {
                    mode,
                    shell: shells.as_ref().map(|panes| ShellView {
                        panes,
                        offset: shell_offset,
                        size: shell_size,
                    }),
                    current_list: &mut current_list,
                    menu: menu.as_ref(),
                    inspect: inspect.as_ref(),
                    usage: usage.as_ref(),
                    settings: settings.as_ref(),
                    appearance: appearance.as_ref(),
                    local_theme: &local_theme,
                    local_ui: &local_ui,
                    local_panels: &local_panels,
                    local_custom_themes: &local_custom_themes,
                    local_active_custom_theme: local_active_custom_theme.as_deref(),
                },
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
                    panes.resize(shell_area(
                        Rect::new(0, 0, cols, rows),
                        shell_offset,
                        shell_size,
                    ))?;
                }
            }
            Event::Mouse(mouse) => {
                // Alt held for a drag or click is a gesture, not a tap; plain motion is not.
                if !matches!(mouse.kind, MouseEventKind::Moved) {
                    alt_tap_armed = false;
                }
                mouse_pos = Some((mouse.column, mouse.row));

                // The browser gets the pointer only when nothing else has a claim on it: not over
                // the shell box, not mid-drag (a drag that leaves the box must still finish it),
                // and not under a prompt, which owns the selection until it is answered.
                let frame_area: Rect = terminal.size()?.into();

                // The disk usage view, the Inspect panel and the menu are modal: they take every
                // mouse event until they close, so a click meant to dismiss one never also
                // selects the row under it.
                if let Some(view) = usage.as_mut() {
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            let places = disk_usage_view::layout(frame_area);
                            let pointer = Position::new(mouse.column, mouse.row);
                            let len = view.rows().len();
                            if let Some(row) =
                                disk_usage_view::row_at(&places, view.top().get(), len, pointer)
                            {
                                view.move_to(row);
                            }
                        }
                        kind => {
                            if let Some(wheel) = Wheel::of(kind) {
                                view.move_by(browser_mouse::wheel_rows(wheel));
                            }
                        }
                    }
                    continue;
                }
                if inspect.is_some() {
                    if matches!(mouse.kind, MouseEventKind::Down(_)) {
                        inspect = None;
                    }
                    continue;
                }
                if settings.is_some() {
                    if matches!(mouse.kind, MouseEventKind::Down(_)) {
                        settings = None;
                    }
                    continue;
                }
                if appearance.is_some() {
                    if let MouseEventKind::Down(_) = mouse.kind {
                        let popup_area = overlay_view::panel_area(
                            frame_area,
                            appearance.as_ref().expect("checked above").rows().len(),
                        );
                        let outcome = match appearance_popup::hit(
                            popup_area,
                            Position::new(mouse.column, mouse.row),
                        ) {
                            AppearanceHit::Row(i)
                                if mouse.kind == MouseEventKind::Down(MouseButton::Left) =>
                            {
                                appearance.as_mut().expect("checked above").click_row(i)
                            }
                            AppearanceHit::Outside => AppearanceOutcome::Close,
                            AppearanceHit::Row(_) | AppearanceHit::Inert => AppearanceOutcome::Stay,
                        };
                        apply_appearance_outcome(
                            outcome,
                            &mut appearance,
                            &mut local_theme,
                            &mut local_ui,
                            &local_panels,
                            &mut local_custom_themes,
                            &mut local_active_custom_theme,
                            config,
                            app,
                        );
                    }
                    continue;
                }
                let pointer = Position::new(mouse.column, mouse.row);
                let menu_reaction = menu.as_mut().map(|open| match mouse.kind {
                    MouseEventKind::Moved => {
                        open.hover_at(frame_area, pointer);
                        MenuMouse::Keep
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        match open.click_at(frame_area, pointer) {
                            MenuOutcome::Run(command) => MenuMouse::Run(command),
                            MenuOutcome::Stay => MenuMouse::Keep,
                            MenuOutcome::Dismiss => MenuMouse::Close,
                        }
                    }
                    // A right-click away from the menu moves it to the new spot; on the menu it
                    // does nothing.
                    MouseEventKind::Down(MouseButton::Right) => {
                        if open.hit(frame_area, pointer) == context_menu::Hit::Outside {
                            MenuMouse::Reopen
                        } else {
                            MenuMouse::Keep
                        }
                    }
                    MouseEventKind::Down(_)
                    | MouseEventKind::ScrollUp
                    | MouseEventKind::ScrollDown
                    | MouseEventKind::ScrollLeft
                    | MouseEventKind::ScrollRight => MenuMouse::Close,
                    MouseEventKind::Up(_) | MouseEventKind::Drag(_) => MenuMouse::Keep,
                });
                match menu_reaction {
                    None => {}
                    Some(MenuMouse::Keep) => continue,
                    Some(MenuMouse::Close) => {
                        menu = None;
                        continue;
                    }
                    Some(MenuMouse::Run(command)) => {
                        let target = menu.take().map(|open| open.target());
                        if let Some(target) = target {
                            run_menu_command(
                                command,
                                target,
                                browser,
                                app,
                                vfs,
                                config,
                                &mut Panels {
                                    inspect: &mut inspect,
                                    usage: &mut usage,
                                    appearance: &mut appearance,
                                },
                            )?;
                        }
                        continue;
                    }
                    Some(MenuMouse::Reopen) => menu = None,
                }

                let over_shell = shells.is_some()
                    && shell_area(frame_area, shell_offset, shell_size)
                        .contains(Position::new(mouse.column, mouse.row));
                let dragging = dragging_divider.is_some()
                    || dragging_shell.is_some()
                    || dragging_shell_width.is_some()
                    || alt_resizing.is_some();
                if config.browser_mouse && !over_shell && !dragging && app.prompt.is_none() {
                    let panels = effective_panels(config, &local_panels);
                    let hit = browser_mouse::hit_test(
                        &BrowserLayout::split(frame_area, panels.columns, panels.show_hud),
                        Position::new(mouse.column, mouse.row),
                        Listing::unscrolled(browser.parent_entries().len()),
                        Listing {
                            len: browser.current_entries().len(),
                            offset: current_list.offset(),
                        },
                    );
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left) if hit != Hit::Elsewhere => {
                            // Clicking the browser is how a mouse user says "stop typing in the
                            // shell", the same as tapping `Alt`.
                            shell_focused = false;
                            pending_leader = false;
                            shell_chord = None;
                            if matches!(hit, Hit::ParentRow(_) | Hit::CurrentRow(_)) {
                                let click = clicks.register(hit, Instant::now());
                                // Read before the click is applied: opening a directory moves
                                // the cursor into it, and the file to open is the one clicked.
                                let file_to_open = match (hit, click) {
                                    (Hit::CurrentRow(index), Click::Double) => browser
                                        .current_entries()
                                        .get(index)
                                        .filter(|entry| !entry.is_dir)
                                        .map(|entry| entry.path.clone()),
                                    _ => None,
                                };
                                if let Err(e) = browser_mouse::apply_click(browser, vfs, hit, click)
                                {
                                    app.status = Some(format!("cannot open: {e}"));
                                }
                                if let Some(path) = file_to_open {
                                    app.open_default(browser, &path);
                                }
                            }
                            continue;
                        }
                        MouseEventKind::Down(MouseButton::Right) if hit != Hit::Elsewhere => {
                            shell_focused = false;
                            pending_leader = false;
                            shell_chord = None;
                            // A right-click selects what it lands on first, the way a left click
                            // would, so the menu is always about something the user can see lit.
                            let on_entry = match hit {
                                Hit::CurrentRow(index) => {
                                    let clicked = browser
                                        .current_entries()
                                        .get(index)
                                        .map(|entry| entry.path.clone());
                                    // Right-clicking inside a marked set keeps it; anywhere else
                                    // it is replaced by the one entry, as in any file manager.
                                    if clicked.is_some_and(|path| !browser.is_marked(&path)) {
                                        browser.clear_marks();
                                    }
                                    browser.select_index(index);
                                    true
                                }
                                Hit::ParentRow(_) => {
                                    if let Err(e) =
                                        browser_mouse::apply_click(browser, vfs, hit, Click::Single)
                                    {
                                        app.status = Some(format!("cannot open: {e}"));
                                    }
                                    true
                                }
                                Hit::Blank(_) | Hit::Elsewhere => false,
                            };
                            let (target, marked) = match browser.selected_entry() {
                                Some(entry) if on_entry => (
                                    MenuTarget::Entry {
                                        is_dir: entry.is_dir,
                                    },
                                    browser.is_marked(&entry.path),
                                ),
                                _ => (MenuTarget::Blank, false),
                            };
                            let context = MenuContext {
                                target,
                                marked,
                                mark_count: browser.marked_paths().len(),
                                clipboard: app.clipboard.is_some(),
                                hidden_shown: browser.show_hidden(),
                            };
                            let names: Vec<String> = open::open_with_entries(&config.open_with)
                                .into_iter()
                                .map(|choice| choice.name)
                                .collect();
                            menu = Some(ContextMenu::new(
                                pointer,
                                target,
                                context_menu::entries(&context, &names),
                            ));
                            continue;
                        }
                        // The wheel over the preview column scrolls what it shows (text, hex,
                        // an archive's listing); over an image or a folder's children it is a
                        // no-op, since those have nothing to scroll.
                        kind if matches!(hit.pane(), Some(Pane::Preview)) => {
                            if let Some(wheel) = Wheel::of(kind) {
                                previews.text.scroll_rows(browser_mouse::wheel_rows(wheel));
                                continue;
                            }
                        }
                        kind if matches!(hit.pane(), Some(Pane::Parent | Pane::Current)) => {
                            if let Some(wheel) = Wheel::of(kind) {
                                let len = browser.current_entries().len();
                                browser.select_index(browser_mouse::wheel_target(
                                    browser.selected_index(),
                                    len,
                                    wheel,
                                ));
                                continue;
                            }
                        }
                        _ => {}
                    }
                }

                let Some(panes) = shells.as_mut() else {
                    continue;
                };
                let area = shell_area(terminal.size()?.into(), shell_offset, shell_size);
                let right_edge = area.x + area.width.saturating_sub(1);
                let over_box = area.contains(Position::new(mouse.column, mouse.row));
                let alt_held = mouse.modifiers.contains(KeyModifiers::ALT);
                match mouse.kind {
                    // `Alt` + left button anywhere on the box grabs the whole box, exactly like
                    // its title bar does, so it takes priority over every border and divider hit
                    // below — those would otherwise swallow a grab that lands on them. The drag
                    // itself is the ordinary `dragging_shell` one, so it needs no `Alt` to keep
                    // going: releasing the key mid-drag doesn't drop the box.
                    MouseEventKind::Down(MouseButton::Left) if alt_held && over_box => {
                        dragging_shell = Some((mouse.column, mouse.row));
                        if panes.focus_at(area, mouse.column, mouse.row) {
                            shell_focused = true;
                        }
                    }
                    MouseEventKind::Down(MouseButton::Right) if alt_held && over_box => {
                        alt_resizing = Some((mouse.column, mouse.row));
                    }
                    MouseEventKind::Drag(MouseButton::Right) => {
                        if let Some((last_col, last_row)) = alt_resizing {
                            let frame_area: Rect = terminal.size()?.into();
                            (shell_offset, shell_size) = resize_box_corner(
                                frame_area,
                                shell_offset,
                                shell_size,
                                (
                                    mouse.column as i32 - last_col as i32,
                                    mouse.row as i32 - last_row as i32,
                                ),
                            );
                            alt_resizing = Some((mouse.column, mouse.row));
                            panes.resize(shell_area(frame_area, shell_offset, shell_size))?;
                        }
                    }
                    MouseEventKind::Up(MouseButton::Right) => alt_resizing = None,
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(divider) = panes
                            .dividers(area)
                            .into_iter()
                            .find(|d| d.hit(mouse.column, mouse.row))
                        {
                            dragging_divider = Some(divider.id());
                        } else if mouse.column == right_edge
                            && mouse.row >= area.y
                            && mouse.row < area.y + area.height
                        {
                            // The box's own right border, distinct from any internal divider —
                            // grabbing it resizes the box's width, like a floating window's edge.
                            // Checked before the title-bar hit-test below so the top-right corner
                            // (where both would otherwise match) prefers resize over move.
                            dragging_shell_width = Some(mouse.column);
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
                        } else if let Some(last_col) = dragging_shell_width {
                            // The box grows symmetrically from its centered position (see
                            // `shell_area`'s `size_adjust`), so the dragged edge only tracks the
                            // mouse 1:1 if the size delta is double the column delta — the
                            // opposite edge moves the other way by the same amount to keep it
                            // centered.
                            shell_size.0 += 2 * (mouse.column as i32 - last_col as i32);
                            dragging_shell_width = Some(mouse.column);
                            panes.resize(shell_area(
                                terminal.size()?.into(),
                                shell_offset,
                                shell_size,
                            ))?;
                        }
                    }
                    MouseEventKind::Up(MouseButton::Left) => {
                        dragging_divider = None;
                        dragging_shell = None;
                        dragging_shell_width = None;
                    }
                    _ => {}
                }
            }
            Event::Key(key) => {
                // A bare modifier key only shows up when the keyboard protocol is on. Only `Alt`
                // means anything; releasing it with nothing pressed in between is the tap.
                if let KeyCode::Modifier(modifier) = key.code {
                    if matches!(
                        modifier,
                        ModifierKeyCode::LeftAlt | ModifierKeyCode::RightAlt
                    ) {
                        match key.kind {
                            KeyEventKind::Press => alt_tap_armed = true,
                            KeyEventKind::Repeat => {}
                            KeyEventKind::Release => {
                                let tapped = std::mem::take(&mut alt_tap_armed);
                                if tapped && shells.is_some() && app.prompt.is_none() {
                                    shell_focused = !shell_focused;
                                    pending_leader = false;
                                    shell_chord = None;
                                }
                            }
                        }
                    }
                    continue;
                }
                // Releases carry no input, but a held key's repeats do: they are typing.
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                alt_tap_armed = false;
                let key = with_shifted_letter_uppercased(key);

                // `Alt` commands come first and work in every mode — typing in a shell, browsing,
                // mid-chord — because a held `Alt` is what says "this is for the shell box, not
                // the shell inside it". Skipped only under a text prompt, which owns the keyboard.
                if app.prompt.is_none()
                    && let Some(command) = alt_keys::parse(key)
                {
                    pending_leader = false;
                    shell_chord = None;
                    dragging_divider = None;
                    // A new shell is where you'd want to type, same as the leader's split.
                    if command == AltCommand::Split {
                        shell_focused = true;
                    }
                    if let Some(message) = apply_alt_command(
                        command,
                        &mut shells,
                        &mut shell_offset,
                        &mut shell_size,
                        mouse_pos,
                        browser.current_dir(),
                        terminal.size()?.into(),
                    )? {
                        app.status = Some(message);
                    }
                    continue;
                }
                let key = with_caps_lock_applied(key);

                // The disk usage view, the Inspect panel and the menu are modal: they own the
                // keyboard until they close, ahead of the leader, the chords and the shell.
                if let Some(view) = usage.as_mut() {
                    let close = match key.code {
                        KeyCode::Esc => true,
                        KeyCode::Char('a') => {
                            view.toggle_measure();
                            false
                        }
                        KeyCode::Char('r') => {
                            view.rescan();
                            false
                        }
                        KeyCode::PageDown => {
                            view.page(1);
                            false
                        }
                        KeyCode::PageUp => {
                            view.page(-1);
                            false
                        }
                        KeyCode::Home => {
                            view.move_to(0);
                            false
                        }
                        KeyCode::End => {
                            view.move_to(usize::MAX);
                            false
                        }
                        code => match config.keys.resolve(code) {
                            Some(Action::MoveDown) => {
                                view.move_by(1);
                                false
                            }
                            Some(Action::MoveUp) => {
                                view.move_by(-1);
                                false
                            }
                            Some(Action::Enter) => {
                                view.enter();
                                false
                            }
                            Some(Action::Leave) => {
                                view.leave();
                                false
                            }
                            // Quitting from inside the view closes the view, not Minuteman.
                            Some(Action::Quit | Action::QuitToCwd | Action::DiskUsage) => true,
                            _ => false,
                        },
                    };
                    if close {
                        usage = None;
                    }
                    continue;
                }
                if inspect.is_some() {
                    if matches!(
                        key.code,
                        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q' | 'i')
                    ) {
                        inspect = None;
                    }
                    continue;
                }
                if let Some(open) = menu.as_mut() {
                    let nav = match key.code {
                        KeyCode::Up | KeyCode::Char('k') => Some(Nav::Up),
                        KeyCode::Down | KeyCode::Char('j') => Some(Nav::Down),
                        KeyCode::Left | KeyCode::Char('h') => Some(Nav::Left),
                        KeyCode::Right | KeyCode::Char('l') => Some(Nav::Right),
                        KeyCode::Enter => Some(Nav::Enter),
                        _ => None,
                    };
                    let outcome = match (key.code, nav) {
                        (KeyCode::Esc | KeyCode::Char('q'), _) => MenuOutcome::Dismiss,
                        (_, Some(nav)) => open.key(nav),
                        (_, None) => MenuOutcome::Stay,
                    };
                    match outcome {
                        MenuOutcome::Run(command) => {
                            let target = open.target();
                            menu = None;
                            run_menu_command(
                                command,
                                target,
                                browser,
                                app,
                                vfs,
                                config,
                                &mut Panels {
                                    inspect: &mut inspect,
                                    usage: &mut usage,
                                    appearance: &mut appearance,
                                },
                            )?;
                        }
                        MenuOutcome::Dismiss => menu = None,
                        MenuOutcome::Stay => {}
                    }
                    continue;
                }
                if let Some(popup) = settings.as_mut() {
                    match popup.key(key.code) {
                        SettingsOutcome::Stay => {}
                        SettingsOutcome::Close => settings = None,
                        SettingsOutcome::Cycle(row) => {
                            let panels = effective_panels(config, &local_panels);
                            match row {
                                SettingsRow::Columns => {
                                    local_panels.columns = Some(
                                        match panels.columns.cycled() {
                                            ColumnLayout::ThreePane => "three",
                                            ColumnLayout::TwoPane => "two",
                                        }
                                        .into(),
                                    );
                                }
                                SettingsRow::Hud => {
                                    local_panels.show_hud = Some(!panels.show_hud);
                                }
                                SettingsRow::CommandBar => {
                                    local_panels.show_command_bar = Some(!panels.show_command_bar);
                                }
                            }
                            persist_local(
                                &local_theme,
                                &local_ui,
                                &local_panels,
                                &local_custom_themes,
                                &local_active_custom_theme,
                                app,
                            );
                            let panels = effective_panels(config, &local_panels);
                            let view = SettingsView {
                                columns: panels.columns,
                                show_hud: panels.show_hud,
                                show_command_bar: panels.show_command_bar,
                            };
                            app.status =
                                Some(format!("{}: {} — saved", row.label(), view.value(row)));
                        }
                    }
                    continue;
                }

                if let Some(popup) = appearance.as_mut() {
                    let outcome = popup.key(key.code);
                    apply_appearance_outcome(
                        outcome,
                        &mut appearance,
                        &mut local_theme,
                        &mut local_ui,
                        &local_panels,
                        &mut local_custom_themes,
                        &mut local_active_custom_theme,
                        config,
                        app,
                    );
                    continue;
                }

                if pending_leader {
                    pending_leader = false;
                    if shells.is_some() {
                        let area = shell_area(terminal.size()?.into(), shell_offset, shell_size);
                        // The leader pressed twice means "go type in the shell", checked through
                        // the keymap rather than a literal `' '` so a rebound leader still works.
                        if config.keys.resolve(key.code) == Some(Action::Leader) {
                            shell_focused = true;
                        } else if let Some(dir) = leader_focus_dir(key.code) {
                            let moved = shells
                                .as_mut()
                                .expect("`shells.is_some()` checked above")
                                .focus_direction(dir, area);
                            if !moved {
                                app.status = Some("no pane that way".into());
                            }
                        } else {
                            match key.code {
                                // `|` reads as the vertical divider a side-by-side split draws,
                                // `-` as the horizontal one a stacked split draws.
                                KeyCode::Char('|') | KeyCode::Char('-') => {
                                    let direction = if key.code == KeyCode::Char('|') {
                                        SplitDirection::Horizontal
                                    } else {
                                        SplitDirection::Vertical
                                    };
                                    let panes =
                                        shells.take().expect("`shells.is_some()` checked above");
                                    shells = Some(panes.split(
                                        direction,
                                        browser.current_dir(),
                                        area,
                                    )?);
                                    // The new pane is where you'd want to type, so hand it the
                                    // keyboard straight away.
                                    shell_focused = true;
                                }
                                KeyCode::Char('x') => {
                                    let panes =
                                        shells.take().expect("`shells.is_some()` checked above");
                                    let id = panes.focused_id();
                                    let remaining = panes.close(id, area)?;
                                    app.status = Some(if remaining.is_some() {
                                        "pane closed".into()
                                    } else {
                                        "shell closed".into()
                                    });
                                    shells = remaining;
                                }
                                KeyCode::Char('r') => shell_chord = Some(ShellChordMode::Resize),
                                KeyCode::Char('m') => shell_chord = Some(ShellChordMode::Move),
                                // A single immediate action, unlike resize/move — there's nothing
                                // repeatable about it, so it never enters `shell_chord` at all.
                                KeyCode::Char('t') => {
                                    let toggled = shells
                                        .as_mut()
                                        .expect("`shells.is_some()` checked above")
                                        .toggle_focused_orientation(area)?;
                                    app.status = Some(if toggled {
                                        "pane orientation toggled".into()
                                    } else {
                                        "only one pane — nothing to toggle".into()
                                    });
                                }
                                _ => {}
                            }
                        }
                    } else if key.code == KeyCode::Char('t') {
                        // With no shell pane open, `leader` followed by `t` otherwise does
                        // nothing (there is no orientation to flip) — free for the settings
                        // popup instead.
                        settings = Some(SettingsPopup::new());
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

                // Typing mode: every key belongs to the focused shell — `Tab`, `Space`, `o`, `%`,
                // `Esc` and all the rest, since a program inside the pane (vim, fzf, ...) needs
                // `Esc` for itself. The ways out are all `Alt`-layer or mouse: tap `Alt`, click
                // the browser, or `Alt+m` to close the pane. Everything else about the panes is
                // a `Leader` command from browse mode (see `pending_leader` above).
                if shell_focused && let Some(panes) = shells.as_mut() {
                    if let Some(bytes) = popup_shell::encode_key(key) {
                        panes.focused_shell().write_input(&bytes)?;
                    }
                    continue;
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
                    } else if config.keys.resolve(key.code) == Some(Action::Cancel) {
                        app.cancel_all(browser);
                    }
                    continue;
                }

                match config.keys.resolve(key.code) {
                    Some(Action::Quit) => return Ok(()),
                    // Without `--cwd-file` (a bare `minuteman` run) there's nobody to hand the
                    // directory to, so this degrades to a plain quit rather than doing nothing.
                    Some(Action::QuitToCwd) => {
                        if let Some(file) = cwd_file {
                            cli::write_cwd_file(file, browser.current_dir())?;
                        }
                        return Ok(());
                    }
                    Some(Action::MoveDown) => browser.move_down(),
                    Some(Action::MoveUp) => browser.move_up(),
                    Some(Action::Enter) => browser.enter(vfs)?,
                    Some(Action::Leave) => browser.leave(vfs)?,
                    Some(Action::Yank) => app.yank(browser),
                    Some(Action::Cut) => app.cut(browser),
                    Some(Action::Paste) => app.begin_paste(browser),
                    Some(Action::Delete) => app.begin_trash(browser),
                    Some(Action::DeletePermanently) => app.begin_delete_permanently(browser),
                    Some(Action::Rename) => app.begin_rename(browser),
                    Some(Action::Create) => app.begin_create(),
                    Some(Action::Search) => app.begin_search(browser),
                    Some(Action::Command) => app.begin_command(),
                    Some(Action::Cancel) => app.cancel_all(browser),
                    Some(Action::Select) => browser.toggle_mark(),
                    Some(Action::DiskUsage) => {
                        usage = Some(app.begin_disk_usage(browser.current_dir().to_path_buf()));
                    }
                    Some(Action::Appearance) => {
                        appearance = Some(AppearancePopup::new());
                    }
                    Some(Action::PreviewDown) => previews.text.scroll_half_pages(1),
                    Some(Action::PreviewUp) => previews.text.scroll_half_pages(-1),
                    Some(Action::ToggleHidden) => {
                        let shown = browser.toggle_hidden(vfs)?;
                        app.status = Some(
                            if shown {
                                "hidden files shown"
                            } else {
                                "hidden files hidden"
                            }
                            .into(),
                        );
                    }
                    // Starts the resize/move chord for shell panes (see `pending_leader` and
                    // `ShellChordMode`). Stays inert — same as before panes existed to chord
                    // against — whenever no shell is open.
                    Some(Action::Leader) => {
                        if shells.is_some() {
                            // The status bar lists what can follow (see `hud::hints`).
                            pending_leader = true;
                        }
                    }
                    // Guarded explicitly rather than relying on it being unreachable while panes
                    // are already open (the branch above handles that case) — opening a second
                    // tree here would silently drop the running one without closing it. Use a
                    // split instead once a shell is already open.
                    Some(Action::Shell) if shells.is_none() => {
                        match fresh_shell_box(
                            browser.current_dir(),
                            terminal.size()?.into(),
                            &mut shell_offset,
                            &mut shell_size,
                        ) {
                            Ok(panes) => {
                                shells = Some(panes);
                                shell_focused = true;
                            }
                            Err(e) => app.status = Some(format!("failed to start shell: {e}")),
                        }
                    }
                    Some(Action::Shell) => {}
                    None => {
                        app.dispatch_plugin_key(key.code, browser);
                    }
                }
            }
            _ => {}
        }
    }
}

/// What one mouse event does to an open right-click menu.
enum MenuMouse {
    /// The menu stays as it is (it may have highlighted a row).
    Keep,
    Close,
    /// Close it, then treat the event as a right-click that opens a fresh one.
    Reopen,
    Run(MenuCommand),
}

/// Carries out what a chosen menu item asks for, by calling the same `App` and `BrowserState`
/// methods the keys do, so a menu click and its key can never behave differently. `target` says
/// whether "Inspect" and "Copy path" mean the selected entry or the browsed directory.
fn run_menu_command(
    command: MenuCommand,
    target: MenuTarget,
    browser: &mut BrowserState,
    app: &mut App,
    vfs: &LocalVfs,
    config: &Config,
    panels: &mut Panels<'_>,
) -> Result<()> {
    // The keys are all ignored while an operation runs; the ones below would start another
    // operation or a prompt over it.
    if app.is_busy()
        && matches!(
            command,
            MenuCommand::Rename | MenuCommand::Delete | MenuCommand::New
        )
    {
        app.status = Some("an operation is already in progress".into());
        return Ok(());
    }
    let selected = browser
        .selected_entry()
        .map(|entry| (entry.path.clone(), entry.is_dir));
    // The path "Inspect" and "Copy path" are about.
    let subject = match (target, &selected) {
        (MenuTarget::Entry { .. }, Some((path, _))) => path.clone(),
        _ => browser.current_dir().to_path_buf(),
    };
    match (command, selected) {
        (MenuCommand::Open, Some((_, true))) => browser.enter(vfs)?,
        (MenuCommand::Open, Some((path, false))) => app.open_default(browser, &path),
        (MenuCommand::OpenWith(index), Some((path, _))) => {
            if let Some(choice) = open::open_with_entries(&config.open_with).get(index) {
                app.open_with(browser, &choice.command, &path);
            }
        }
        (MenuCommand::PasteInto, Some((path, true))) => app.begin_paste_into(path),
        (MenuCommand::Cut, _) => app.cut(browser),
        (MenuCommand::Copy, _) => app.yank(browser),
        (MenuCommand::Paste, _) => app.begin_paste(browser),
        (MenuCommand::Rename, _) => app.begin_rename(browser),
        (MenuCommand::Delete, _) => app.begin_trash(browser),
        (MenuCommand::New, _) => app.begin_create(),
        (MenuCommand::ToggleMark, _) => browser.toggle_mark(),
        (MenuCommand::CopyPath, _) => {
            execute!(
                io::stdout(),
                Print(osc52::set_clipboard(&subject.to_string_lossy()))
            )?;
            app.status = Some(format!(
                "sent {} to the terminal's clipboard",
                subject.display()
            ));
        }
        (MenuCommand::Inspect, _) => *panels.inspect = app.begin_inspect(&subject),
        (MenuCommand::DiskUsage, _) => {
            *panels.usage = Some(app.begin_disk_usage(subject.clone()));
        }
        (MenuCommand::Appearance, _) => *panels.appearance = Some(AppearancePopup::new()),
        (MenuCommand::ToggleHidden, _) => {
            let shown = browser.toggle_hidden(vfs)?;
            app.status = Some(
                if shown {
                    "hidden files shown"
                } else {
                    "hidden files hidden"
                }
                .into(),
            );
        }
        (MenuCommand::Refresh, _) => {
            browser.reload(vfs)?;
            browser.prune_marks(vfs);
            app.status = Some("refreshed".into());
        }
        // The entry the menu was opened on is gone (deleted by another program while it was
        // open), so there is nothing left for an entry-only item to act on.
        (MenuCommand::Open | MenuCommand::OpenWith(_) | MenuCommand::PasteInto, _) => {
            app.status = Some("that entry is no longer there".into())
        }
    }
    Ok(())
}

/// The pane-focus direction a key means after the leader: `hjkl` or the arrows.
fn leader_focus_dir(code: KeyCode) -> Option<NudgeDir> {
    match code {
        KeyCode::Char('h') | KeyCode::Left => Some(NudgeDir::Left),
        KeyCode::Char('j') | KeyCode::Down => Some(NudgeDir::Down),
        KeyCode::Char('k') | KeyCode::Up => Some(NudgeDir::Up),
        KeyCode::Char('l') | KeyCode::Right => Some(NudgeDir::Right),
        _ => None,
    }
}

fn draw(
    frame: &mut ratatui::Frame<'_>,
    browser: &BrowserState,
    app: &App,
    previews: &mut Previews,
    vfs: &LocalVfs,
    config: &Config,
    overlay: Overlay<'_>,
) {
    let Overlay {
        mode,
        shell,
        current_list: current_state,
        menu,
        inspect,
        usage,
        settings,
        appearance,
        local_theme,
        local_ui,
        local_panels,
        local_custom_themes: _local_custom_themes,
        local_active_custom_theme,
    } = overlay;
    // The settings and appearance popups' changes are saved to `local.toml` (see `run`'s
    // `local_theme`/`local_ui`/`local_panels`), but applying them still has to happen every
    // render, the same as when they were session-only, since `config` itself is never mutated
    // after startup. Cloning once here — rather than threading three more parameters through
    // every `hud`/`overlay_view`/`style` function that already takes `config` — keeps the rest of
    // `draw` (and every function it calls) unchanged; a `Config` is small next to the
    // `Vec<ListItem>`s this function already rebuilds every frame regardless.
    let mut theme = effective_theme(config, local_theme);
    // A color row's in-progress (uncommitted) edit previews live, on top of everything else —
    // in text mode straight from the typed buffer, in picker mode from the HSV sliders' hex form.
    if let Some(popup) = appearance {
        if let (Some(row), Some(buffer)) = (popup.editing_row(), popup.editing_buffer()) {
            theme = appearance_popup::preview(&theme, row, buffer);
        } else if let (Some(row), Some((hsv, _))) = (popup.editing_row(), popup.editing_picker()) {
            theme = appearance_popup::preview(&theme, row, &hsv.to_hex());
        }
    }
    let effective_config = Config {
        panels: effective_panels(config, local_panels),
        theme,
        ui: effective_ui(config, local_ui),
        ..config.clone()
    };
    let config = &effective_config;

    // Header, the three file columns, then the status bar. The mouse handler hit-tests against
    // this same split (see `browser_mouse`), so the two can't disagree about where a row is.
    let layout = BrowserLayout::split(frame.area(), config.panels.columns, config.panels.show_hud);
    let (header_row, status_row) = (layout.header, layout.status);
    let columns = [layout.parent, layout.current, layout.preview];

    // Read once: the status bar wants the selected directory's item count, and the preview pane
    // wants its listing — one `list_dir`, not two, per frame.
    let dir_preview: Option<Vec<DirEntryInfo>> = browser
        .selected_entry()
        .filter(|e| e.is_dir)
        .map(|_| browser.preview_entries(vfs));

    // Background only: the selected row's text color is set per span in `entry_item` (so the
    // accent stripe and file-type colors survive the highlight).
    let selection_style = Style::default().bg(color_from_name(&config.theme.selection_bg));

    // Parent pane — context only, no selection highlight. Skipped entirely in two-pane layout,
    // where `columns[0]` is the zero-width `Rect` `BrowserLayout::split` gives the removed
    // column, rather than rendering an empty list into it.
    if columns[0].width > 0 {
        let parent_items: Vec<ListItem> = browser
            .parent_entries()
            .iter()
            .map(|e| entry_item(e, config, Row::PLAIN, None))
            .collect();
        frame.render_widget(
            List::new(parent_items).block(style::themed_block(config, "..", false)),
            columns[0],
        );
    }

    // Current pane — the active column, with the selection highlighted and marked entries
    // prefixed (Ranger-style) so a pending multi-select is visible before acting on it.
    let has_selection = !browser.current_entries().is_empty();
    let now = SystemTime::now();
    let glyphs = glyphs::of(config);
    let plan = hud::plan_columns(
        columns[1].width.saturating_sub(2) as usize,
        hud::BASE_GUTTER + glyphs.icon_width(),
    );
    let current_items: Vec<ListItem> = browser
        .current_entries()
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let row = Row {
                gutter: true,
                selected: has_selection && i == browser.selected_index(),
                marked: browser.is_marked(&e.path),
            };
            entry_item(e, config, row, Some((plan, now)))
        })
        .collect();
    // `select(None)` also rewinds the scroll offset, so an emptied directory doesn't leave the
    // next one drawn from a stale row.
    current_state.select(has_selection.then(|| browser.selected_index()));
    // The header carries the full path; the frame just names this directory.
    let title = browser
        .current_dir()
        .file_name()
        .map_or_else(|| "/".to_string(), |n| n.to_string_lossy().into_owned());
    frame.render_stateful_widget(
        List::new(current_items)
            .block(style::themed_block(config, &title, true))
            .highlight_style(selection_style),
        columns[1],
        current_state,
    );
    hud::render_scrollbar(
        frame,
        columns[1],
        browser.current_entries().len(),
        browser.selected_index(),
        config,
    );

    // Preview pane — an inline image for image files, rendered text for code/text files,
    // children of a selected directory, or the file's name as a placeholder for anything else.
    let is_selected_image = browser
        .selected_entry()
        .is_some_and(|e| !e.is_dir && preview::is_image(&e.path));
    // Any other file has something to show — text, an archive's listing or a hex dump — unless
    // it is not a regular file (a socket, a pipe), which reads back as `Empty`.
    let is_selected_file = browser.selected_entry().is_some_and(|e| !e.is_dir)
        && previews.text.status() != TextPreviewStatus::Empty;

    if is_selected_image {
        let block = style::themed_block(config, "preview", false);
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
    } else if is_selected_file {
        preview_view::render(frame, columns[2], &mut previews.text, config);
    } else if let Some(children) = &dir_preview {
        let items: Vec<ListItem> = children
            .iter()
            .map(|e| entry_item(e, config, Row::PLAIN, None))
            .collect();
        frame.render_widget(
            List::new(items).block(style::themed_block(config, "preview", false)),
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
                .block(style::themed_block(config, "preview", false)),
            columns[2],
        );
    }

    if config.panels.show_hud {
        hud::render_header(
            frame,
            header_row,
            &hud::HeaderView {
                path: browser.current_dir(),
                home: std::env::var_os("HOME").map(PathBuf::from),
                marks: browser.marked_paths().len(),
                marks_total: app.marked_total(),
                clipboard: app.clipboard.as_ref().map(|c| (c.mode, c.paths.len())),
                progress: app.progress(),
            },
            config,
        );
    }

    let mut message = app.status_line();
    if app.prompt.as_ref().is_some_and(|p| p.is_text_input()) {
        // A visible cursor: prompts only ever append to their buffer.
        message.push_str(glyphs::of(config).cursor);
    }
    // Hiding the command bar only suppresses its idle chrome (the mode pill, the selected file's
    // details, the key hints) — a prompt, a busy/leader/resize/move mode or a transient message
    // still renders the bar in full, exactly as when the setting is on. The row itself always
    // stays reserved (see `BrowserLayout::split`), so this only ever leaves it blank, never
    // reclaims it.
    let show_status_bar =
        config.panels.show_command_bar || mode != hud::Mode::Normal || !message.trim().is_empty();
    if show_status_bar {
        hud::render_status_bar(
            frame,
            status_row,
            &hud::StatusView {
                mode,
                shell_open: shell.is_some(),
                entry: browser.selected_entry(),
                dir_items: dir_preview.as_ref().map(Vec::len),
                position: (
                    if has_selection {
                        browser.selected_index() + 1
                    } else {
                        0
                    },
                    browser.current_entries().len(),
                ),
                message: &message,
                git: app.git_repo(),
                git_entry: app
                    .git_repo()
                    .zip(browser.selected_entry())
                    .and_then(|(repo, entry)| repo.state_of(&entry.path)),
            },
            config,
        );
    }

    // Drawn last, over the browser columns above — the tiled shell panes, in a box that never
    // covers the status bar and can be dragged off-center by its title bar (see `shell_area`).
    if let Some(ShellView {
        panes,
        offset,
        size,
    }) = shell
    {
        panes.render(frame, shell_area(frame.area(), offset, size), config);
    }
    if let Some(view) = usage {
        disk_usage_view::render(frame, view, config);
    }
    if let Some(menu) = menu {
        overlay_view::render_menu(frame, menu, config);
    }
    if let Some(view) = inspect {
        overlay_view::render_inspect(frame, view, config);
    }
    if let Some(popup) = settings {
        overlay_view::render_settings(
            frame,
            popup,
            &SettingsView {
                columns: config.panels.columns,
                show_hud: config.panels.show_hud,
                show_command_bar: config.panels.show_command_bar,
            },
            config,
        );
    }
    if let Some(popup) = appearance {
        // `config` is already the effective one composed above, so its `theme`/`ui.glyphs`
        // already carry `local_theme`/`local_ui` (and any in-progress edit's live preview) —
        // nothing more to layer here.
        let theme_name = appearance_popup::theme_name(local_theme.name.as_deref(), &config.theme);
        overlay_view::render_appearance(
            frame,
            popup,
            &AppearanceView {
                theme: &config.theme,
                glyphs: config.ui.glyphs,
                theme_name,
                active_custom_theme: local_active_custom_theme,
            },
            config,
        );
    }
}

/// How one row of a file list is drawn.
#[derive(Clone, Copy)]
struct Row {
    /// Reserve the two-cell gutter (selection stripe + mark) in front of the name. Only the
    /// current pane has one; the parent and preview lists are context, with nothing selected.
    gutter: bool,
    selected: bool,
    marked: bool,
}

impl Row {
    const PLAIN: Row = Row {
        gutter: false,
        selected: false,
        marked: false,
    };
}

/// `columns` is the current pane's column plan and the time to measure ages against; `None` for
/// the context panes, which show names only.
fn entry_item(
    entry: &DirEntryInfo,
    config: &Config,
    row: Row,
    columns: Option<(hud::Columns, SystemTime)>,
) -> ListItem<'static> {
    let theme = &config.theme;
    let g = glyphs::of(config);
    let accent = Style::default().fg(color_from_name(&theme.accent_fg));
    let styles = &config.styles;
    let kind = style::FileKind::classify(entry);
    let mut kind_style = style::styled(
        Style::default().fg(color_from_name(kind.theme_color(theme))),
        kind.mods(styles),
    );
    if style::is_executable(entry) {
        kind_style = style::styled(kind_style, styles.executable);
    }
    let mut name_style = kind_style;
    if row.selected {
        if let Some(fg) = style::selection_fg(theme) {
            name_style = name_style.fg(fg);
        }
        name_style = style::styled(name_style, styles.selection);
    }
    if row.marked {
        name_style = style::styled(name_style, styles.mark);
    }

    let mut spans = Vec::with_capacity(3);
    if row.gutter {
        spans.push(if row.selected {
            Span::styled(g.stripe, accent)
        } else {
            Span::raw(" ")
        });
        spans.push(if row.marked {
            Span::styled("*", style::styled(accent, styles.mark))
        } else {
            Span::raw(" ")
        });
    }
    // A file-type icon in the kind's color (Nerd glyph set only), before the name.
    if let Some(icons) = &g.icons {
        spans.push(Span::styled(
            format!("{} ", icons.for_kind(kind)),
            kind_style,
        ));
    }
    let label = entry_label(entry);
    match columns {
        Some((plan, now)) => {
            spans.push(Span::styled(
                hud::pad_to(&label, plan.name_width),
                name_style,
            ));
            // Dim, so the eye lands on names first.
            let dim = style::styled(
                Style::default().fg(color_from_name(&theme.status_fg)),
                styles.columns,
            );
            if plan.size {
                spans.push(Span::styled(
                    format!(" {:>5}", hud::size_cell(entry, g.none)),
                    dim,
                ));
            }
            if plan.age {
                let age = match entry.modified {
                    Some(_) => hud::format_age(now, entry.modified),
                    None => g.none.to_string(),
                };
                spans.push(Span::styled(format!(" {age:>3}"), dim));
            }
        }
        None => spans.push(Span::styled(label, name_style)),
    }
    ListItem::new(Line::from(spans))
}

fn entry_label(entry: &DirEntryInfo) -> String {
    if entry.is_dir {
        format!("{}/", entry.name)
    } else {
        entry.name.clone()
    }
}

fn color_from_name(name: &str) -> Color {
    style::color(name)
}

/// Applies Caps Lock to a letter the way every terminal without the keyboard protocol already
/// does: it flips the case, so Caps Lock+`q` is `Q` and Caps Lock+Shift+`q` is `q`. With the
/// protocol on, a terminal reports Caps Lock as a flag and leaves the letter as if it were off,
/// which made `Q` (quit and `cd`) unreachable with Caps Lock on and Caps Lock useless in a
/// mini-shell. Run after `Alt` commands are handled, so those keep working with it on.
fn with_caps_lock_applied(mut key: event::KeyEvent) -> event::KeyEvent {
    if let KeyCode::Char(c) = key.code
        && key.state.contains(KeyEventState::CAPS_LOCK)
        && c.is_ascii_alphabetic()
    {
        key.code = KeyCode::Char(if c.is_ascii_lowercase() {
            c.to_ascii_uppercase()
        } else {
            c.to_ascii_lowercase()
        });
    }
    key
}

/// Turns Shift plus a lowercase letter into the uppercase letter. With the keyboard protocol on,
/// a terminal may report Shift+q as `q` with the Shift flag rather than as `Q`; every binding
/// (`Q` quit-and-`cd`, `S`, ...) and every capital typed into a mini-shell is keyed on the
/// uppercase character, so it is settled once here, before any of them look at the key.
fn with_shifted_letter_uppercased(mut key: event::KeyEvent) -> event::KeyEvent {
    if let KeyCode::Char(c) = key.code
        && key.modifiers.contains(KeyModifiers::SHIFT)
        && c.is_ascii_lowercase()
    {
        key.code = KeyCode::Char(c.to_ascii_uppercase());
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shifted_lowercase_letter_becomes_the_capital() {
        let key = |c, m| event::KeyEvent::new(KeyCode::Char(c), m);
        let shifted = with_shifted_letter_uppercased(key('q', KeyModifiers::SHIFT));
        assert_eq!(shifted.code, KeyCode::Char('Q'));
        // Already a capital, unshifted, or not a letter: left exactly as it was.
        for k in [
            key('Q', KeyModifiers::SHIFT),
            key('q', KeyModifiers::NONE),
            key('1', KeyModifiers::SHIFT),
        ] {
            assert_eq!(with_shifted_letter_uppercased(k), k);
        }
    }

    #[test]
    fn caps_lock_flips_the_case_of_a_letter_and_nothing_else() {
        let key = |c, state| {
            let mut k = event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
            k.state = state;
            k
        };
        let caps = KeyEventState::CAPS_LOCK;
        assert_eq!(
            with_caps_lock_applied(key('q', caps)).code,
            KeyCode::Char('Q')
        );
        // Caps Lock with Shift held is lowercase again, as in every other terminal.
        assert_eq!(
            with_caps_lock_applied(key('Q', caps)).code,
            KeyCode::Char('q')
        );
        for k in [
            key('q', KeyEventState::NONE),
            key('1', caps),
            key(' ', caps),
        ] {
            assert_eq!(with_caps_lock_applied(k), k);
        }
    }

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
        assert_eq!(
            (moved.width, moved.height),
            (centered.width, centered.height)
        );
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

    #[test]
    fn shell_params_for_is_the_exact_inverse_of_shell_area() {
        // Swept rather than sampled: every position and a spread of sizes that fit the bounds,
        // on both an even and an odd-sized screen, since the centering math rounds differently.
        for frame in [Rect::new(0, 0, 100, 40), Rect::new(0, 0, 91, 33)] {
            let bounds = shell_bounds(frame);
            for width in (MIN_SHELL_BOX_WIDTH..=bounds.width).step_by(7) {
                for height in (MIN_SHELL_BOX_HEIGHT..=bounds.height).step_by(5) {
                    for x in (0..=bounds.width - width).step_by(3) {
                        for y in 0..=bounds.height - height {
                            let target = Rect::new(x, y, width, height);
                            let (offset, size) = shell_params_for(frame, target);
                            assert_eq!(shell_area(frame, offset, size), target);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn grow_box_edge_moves_only_the_named_side() {
        let frame = Rect::new(0, 0, 100, 40);
        let before = shell_area(frame, (0, 0), (0, 0));
        let step = SHELL_BOX_RESIZE_STEP as u16;
        let grown = |dir| {
            let (offset, size) = grow_box_edge(frame, (0, 0), (0, 0), dir);
            shell_area(frame, offset, size)
        };

        let left = grown(NudgeDir::Left);
        assert_eq!((left.x, left.width), (before.x - step, before.width + step));
        assert_eq!((left.y, left.height), (before.y, before.height));

        let right = grown(NudgeDir::Right);
        assert_eq!((right.x, right.width), (before.x, before.width + step));

        let up = grown(NudgeDir::Up);
        assert_eq!((up.y, up.height), (before.y - step, before.height + step));
        assert_eq!((up.x, up.width), (before.x, before.width));

        let down = grown(NudgeDir::Down);
        assert_eq!((down.y, down.height), (before.y, before.height + step));
    }

    #[test]
    fn grow_box_edge_stops_at_the_screen_edge() {
        let frame = Rect::new(0, 0, 100, 40);
        let (mut offset, mut size) = ((0, 0), (0, 0));
        for _ in 0..200 {
            (offset, size) = grow_box_edge(frame, offset, size, NudgeDir::Left);
            (offset, size) = grow_box_edge(frame, offset, size, NudgeDir::Up);
        }
        let area = shell_area(frame, offset, size);
        assert_eq!((area.x, area.y), (0, 0));
        // The far sides never moved while the near ones were being grown out.
        assert_eq!(area.x + area.width, 90);
        assert_eq!(area.y + area.height, 33);
    }

    #[test]
    fn resize_box_corner_keeps_the_top_left_and_respects_the_limits() {
        let frame = Rect::new(0, 0, 100, 40);
        let before = shell_area(frame, (0, 0), (0, 0));

        let (offset, size) = resize_box_corner(frame, (0, 0), (0, 0), (-6, -3));
        let smaller = shell_area(frame, offset, size);
        assert_eq!((smaller.x, smaller.y), (before.x, before.y));
        assert_eq!(
            (smaller.width, smaller.height),
            (before.width - 6, before.height - 3)
        );

        let (offset, size) = resize_box_corner(frame, (0, 0), (0, 0), (-10_000, -10_000));
        let floor = shell_area(frame, offset, size);
        assert_eq!(
            (floor.width, floor.height),
            (MIN_SHELL_BOX_WIDTH, MIN_SHELL_BOX_HEIGHT)
        );

        let (offset, size) = resize_box_corner(frame, (0, 0), (0, 0), (10_000, 10_000));
        let ceiling = shell_area(frame, offset, size);
        assert_eq!((ceiling.x, ceiling.y), (before.x, before.y));
        assert_eq!(
            (ceiling.x + ceiling.width, ceiling.y + ceiling.height),
            (100, 39)
        );
    }

    #[test]
    fn snap_box_offset_pins_to_the_top_or_bottom_and_stays_centered() {
        let frame = Rect::new(0, 0, 100, 40);
        let default = shell_area(frame, (0, 0), (0, 0));

        // Starting from an off-center box, so the horizontal re-centering is actually exercised.
        let start = (17, -4);
        let top = shell_area(frame, snap_box_offset(frame, start, (0, 0), true), (0, 0));
        assert_eq!((top.x, top.y), (default.x, 0));
        assert_eq!((top.width, top.height), (default.width, default.height));

        let bottom = shell_area(frame, snap_box_offset(frame, start, (0, 0), false), (0, 0));
        assert_eq!((bottom.x, bottom.y + bottom.height), (default.x, 39));
    }

    #[test]
    fn snapping_leaves_a_small_offset_not_an_inflated_one() {
        let frame = Rect::new(0, 0, 100, 40);
        let offset = snap_box_offset(frame, (0, 0), (0, 0), true);
        // Bounded by the screen, so a single opposite nudge is never lost in accumulated slack.
        assert!(offset.1.abs() <= 40, "offset {offset:?}");
    }
}
