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

//! The appearance popup's state: which row the cursor is on, whether one is being typed into, and
//! what a keystroke or a click means. Like `settings_popup` and `context_menu`, this is pure — no
//! `Frame`, no terminal — so `overlay_view::render_appearance` is the only place that draws it and
//! `main` is the only place that applies what it asks for.
//!
//! Unlike the settings popup (which only ever cycles a small fixed set of values), most of this
//! popup's rows hold a free-form color — a name or `#rrggbb`/`#rgb` hex, exactly what
//! `appearance.toml` already accepts — so they need actual text entry, not just cycling. `key`
//! and `click_row` both funnel into the same `Outcome`s, so a mouse user and a keyboard user can
//! reach the same result. The popup itself never touches `Config`; it only reports what it wants
//! and leaves applying that — and deciding what "the defaults" means to reset to — to `main`.
//! This is session-only, in-memory state, same as the settings popup.

use crossterm::event::KeyCode;
use ratatui::layout::{Margin, Position, Rect};
use theming::{GlyphSet, Hsv, RawTheme, Theme, hex_to_hsv};

/// The built-in palettes the "Theme" row cycles through, in order. `catppuccin-latte` isn't
/// here — it's only reached via `Theme::auto` on a light terminal, or by naming it explicitly in
/// `appearance.toml`; this cycle sticks to dark-background palettes, like `neon`, `dracula` and
/// `nord` already do. Used to be the settings popup's own row; it moved here to sit with the
/// rest of the look it picks a base for.
pub const THEME_NAMES: [&str; 5] = ["neon", "classic", "dracula", "catppuccin", "nord"];

/// One row of the popup, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// Picks the base palette everything else layers on top of.
    Theme,
    Accent,
    BorderFocused,
    Selection,
    Directory,
    StatusBar,
    Danger,
    BorderType,
    Separator,
    Glyphs,
    /// Saves the live look as a named custom theme — updating the one it was loaded from/last
    /// saved as, if it still matches one, or prompting for a new name otherwise. See
    /// `Outcome::WantSaveTheme`.
    SaveTheme,
    /// Clears every override this popup has made this session, in one step.
    Reset,
}

const ROWS: [Row; 12] = [
    Row::Theme,
    Row::Accent,
    Row::BorderFocused,
    Row::Selection,
    Row::Directory,
    Row::StatusBar,
    Row::Danger,
    Row::BorderType,
    Row::Separator,
    Row::Glyphs,
    Row::SaveTheme,
    Row::Reset,
];

/// What kind of value a row holds, and so how a keystroke or a click on it behaves. `Row::Reset`
/// and `Row::SaveTheme` have neither — they are actions, not values — so `Row::kind` returns
/// `None` for them.
#[derive(Debug, Clone, Copy)]
pub enum RowKind {
    /// A free-form name or hex value, edited as text.
    Color,
    /// One of a fixed list, cycled like the settings popup's rows.
    Cycle(&'static [&'static str]),
}

impl Row {
    pub fn label(self) -> &'static str {
        match self {
            Row::Theme => "Theme",
            Row::Accent => "Accent",
            Row::BorderFocused => "Focused border",
            Row::Selection => "Selection",
            Row::Directory => "Directory",
            Row::StatusBar => "Status bar",
            Row::Danger => "Danger",
            Row::BorderType => "Border style",
            Row::Separator => "Separator",
            Row::Glyphs => "Glyphs",
            Row::SaveTheme => "Save theme",
            Row::Reset => "Reset to defaults",
        }
    }

    pub fn kind(self) -> Option<RowKind> {
        match self {
            Row::Theme => Some(RowKind::Cycle(&THEME_NAMES)),
            Row::BorderType => Some(RowKind::Cycle(&["rounded", "plain", "double", "thick"])),
            Row::Separator => Some(RowKind::Cycle(&["flat", "arrow", "auto"])),
            Row::Glyphs => Some(RowKind::Cycle(&["unicode", "nerd", "ascii"])),
            Row::SaveTheme | Row::Reset => None,
            Row::Accent
            | Row::BorderFocused
            | Row::Selection
            | Row::Directory
            | Row::StatusBar
            | Row::Danger => Some(RowKind::Color),
        }
    }
}

/// The current value of every row, for `overlay_view::render_appearance` to draw and for `main`
/// to read the "before" value off of when a color row's edit begins. Built fresh each frame (or
/// each time it's needed) from the session's live theme/glyphs, not stored on the popup itself.
pub struct AppearanceView<'a> {
    pub theme: &'a Theme,
    pub glyphs: GlyphSet,
    /// The Theme row's own value — not derivable from `theme`'s fields alone, since several
    /// named palettes can share a field's value and a palette's colors can themselves be
    /// individually overridden. `main` tracks the picked name directly (`local_theme.name`).
    pub theme_name: &'static str,
    /// The name of the saved custom theme the live look started from (or was last saved as),
    /// if any — shown next to `Row::SaveTheme` so it's clear whether that row will update an
    /// existing theme or start a new one.
    pub active_custom_theme: Option<&'a str>,
}

impl AppearanceView<'_> {
    pub fn value(&self, row: Row) -> String {
        match row {
            Row::Theme => self.theme_name.to_string(),
            Row::Accent => self.theme.accent_fg.clone(),
            Row::BorderFocused => self.theme.border_focused_fg.clone(),
            Row::Selection => self.theme.selection_bg.clone(),
            Row::Directory => self.theme.dir_fg.clone(),
            Row::StatusBar => self.theme.bar_bg.clone(),
            Row::Danger => self.theme.danger_fg.clone(),
            Row::BorderType => self.theme.border_type.clone(),
            Row::Separator => self.theme.separator.clone(),
            Row::Glyphs => glyph_name(self.glyphs).to_string(),
            Row::SaveTheme => match self.active_custom_theme {
                Some(name) => format!("updates '{name}'"),
                None => "new theme".to_string(),
            },
            Row::Reset => String::new(),
        }
    }
}

fn glyph_name(glyphs: GlyphSet) -> &'static str {
    match glyphs {
        GlyphSet::Unicode => "unicode",
        GlyphSet::Nerd => "nerd",
        GlyphSet::Ascii => "ascii",
    }
}

/// `theme` with just `row`'s field replaced by `value` — the live-preview overlay while a color
/// row is mid-edit, before `Enter` commits it. A no-op for a row that isn't a color.
pub fn preview(theme: &Theme, row: Row, value: &str) -> Theme {
    let mut theme = theme.clone();
    match row {
        Row::Accent => theme.accent_fg = value.to_string(),
        Row::BorderFocused => theme.border_focused_fg = value.to_string(),
        Row::Selection => theme.selection_bg = value.to_string(),
        Row::Directory => theme.dir_fg = value.to_string(),
        Row::StatusBar => theme.bar_bg = value.to_string(),
        Row::Danger => theme.danger_fg = value.to_string(),
        Row::Theme
        | Row::BorderType
        | Row::Separator
        | Row::Glyphs
        | Row::SaveTheme
        | Row::Reset => {}
    }
    theme
}

/// Commits `value` into `overrides` for `row`'s field — a confirmed text edit or a cycled value
/// both end up here, including a `Theme` row pick (`RawTheme.name`). `Row::Glyphs` isn't part of
/// `RawTheme`, so `main` sets `local_ui` directly instead of calling this for it; `Row::Reset`
/// carries no value.
pub fn commit(overrides: &mut RawTheme, row: Row, value: String) {
    match row {
        Row::Theme => overrides.name = Some(value),
        Row::Accent => overrides.accent_fg = Some(value),
        Row::BorderFocused => overrides.border_focused_fg = Some(value),
        Row::Selection => overrides.selection_bg = Some(value),
        Row::Directory => overrides.dir_fg = Some(value),
        Row::StatusBar => overrides.bar_bg = Some(value),
        Row::Danger => overrides.danger_fg = Some(value),
        Row::BorderType => overrides.border_type = Some(value),
        Row::Separator => overrides.separator = Some(value),
        Row::Glyphs | Row::SaveTheme | Row::Reset => {}
    }
}

/// The next value in `options` after `current`, wrapping. An unrecognised `current` starts at the
/// first option — the same "a typo falls back to the start" rule `Theme::named` already follows.
pub fn next_in(options: &[&str], current: &str) -> String {
    let next = match options.iter().position(|o| *o == current) {
        Some(i) => (i + 1) % options.len(),
        None => 0,
    };
    options[next].to_string()
}

/// The Theme row's display value: `local_name` (the appearance popup's own pick, if it ever made
/// one this session or a saved one was loaded) when it names one of `THEME_NAMES` exactly; failing
/// that, whichever named palette `theme`'s resolved fields exactly match — so an unmodified
/// `appearance.toml` pin or the adaptive default (see `Theme::auto`) still shows its real name
/// rather than always reading "neon". Falls back to the first name in the list only when neither
/// applies (e.g. a pin plus an inline field override in `appearance.toml`) — cosmetic only; the
/// colors actually shown are unaffected either way.
pub fn theme_name(local_name: Option<&str>, theme: &Theme) -> &'static str {
    local_name
        .and_then(|name| THEME_NAMES.iter().find(|&&t| t == name).copied())
        .or_else(|| {
            THEME_NAMES
                .iter()
                .find(|&&t| Theme::named(t) == *theme)
                .copied()
        })
        .unwrap_or(THEME_NAMES[0])
}

/// What a keystroke or a click asks the caller to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing outside the popup; keep it open.
    Stay,
    /// The row under the cursor holds a color; the caller looks up its current value and calls
    /// `begin_edit` to actually enter text-editing mode.
    WantEdit(Row),
    /// Advance this row to its next fixed value.
    Cycle(Row),
    /// A text edit was confirmed with `Enter`.
    Commit(Row, String),
    /// Clear every override this popup has made this session.
    Reset,
    Close,
    /// `Row::SaveTheme` was activated; the caller compares the live theme against its saved
    /// custom themes, builds a `SaveChoice`, and calls `begin_save` with it — the popup itself
    /// never sees `Config` or `local.toml`, so it can't make that comparison on its own.
    WantSaveTheme,
    /// The save flow `begin_save` started was carried through to a name — either typed fresh or
    /// confirmed as an update.
    SaveTheme(SaveTarget),
}

/// What `Outcome::WantSaveTheme` resolves to, once the caller has compared the live theme
/// against the custom themes it owns copies of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveChoice {
    /// The live look isn't a saved custom theme, or has drifted from the built-in palette it
    /// started from — nothing to offer "update" for, so the popup goes straight to naming it.
    New,
    /// It started from (or was last saved as) `.0`, and has drifted since — the popup asks
    /// whether to update that theme or save a new one.
    UpdateOrNew(String),
    /// It matches `.0` exactly already; `begin_save` is a no-op for this, and the caller should
    /// report that rather than opening the popup's save flow.
    Unchanged(String),
}

/// Where a confirmed save flow should write: a brand new theme, or over one that already exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveTarget {
    New(String),
    Update(String),
}

/// Where a click landed, from `hit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Row(usize),
    /// Inside the popup's border but not on an actionable row (the blank line, the hint line).
    Inert,
    Outside,
}

/// What `pos` is over, given the popup's outer rectangle (from `overlay_view::panel_area`) — the
/// same geometry `overlay_view::render_appearance` draws with, so a click and its drawn row can
/// never disagree.
pub fn hit(area: Rect, pos: Position) -> Hit {
    let inner = area.inner(Margin::new(1, 1));
    if !inner.contains(pos) {
        return Hit::Outside;
    }
    let offset = pos.y - inner.y;
    if offset == 0 {
        // The blank line above the rows.
        return Hit::Inert;
    }
    match usize::from(offset - 1) {
        row if row < ROWS.len() => Hit::Row(row),
        _ => Hit::Inert,
    }
}

/// How much a `Left`/`Right` nudge moves the picker's selected channel — degrees for hue,
/// percentage points for saturation/value. The same step for all three keeps the key feel
/// consistent even though the ranges differ.
const PICKER_STEP: f64 = 5.0;

/// A color row's in-progress edit: either typed as text (a name or `#rrggbb`/`#rgb` hex, exactly
/// what `appearance.toml` accepts) or adjusted as HSV sliders — `Tab` swaps between the two,
/// converting the value across the swap so neither loses what the other typed/set.
#[derive(Debug, Clone, PartialEq)]
enum Edit {
    Text(String),
    Picker {
        hsv: Hsv,
        /// Which of H/S/V (0/1/2) `Up`/`Down` and `Left`/`Right` act on.
        channel: u8,
    },
}

/// The save flow `Outcome::WantSaveTheme` / `begin_save` drive: first an update-or-new choice
/// (skipped when there's nothing to offer "update" for), then typing a name.
#[derive(Debug, Clone, PartialEq)]
enum SaveState {
    /// Offers "update `.0`" (`u`) or "save as new" (`n`); `.0` is the theme that would be
    /// updated.
    Choice(String),
    /// Typing the name a new custom theme will be saved under.
    Name(String),
}

pub struct AppearancePopup {
    cursor: usize,
    /// `Some(_)` while the row under the cursor is being edited, in either mode.
    editing: Option<Edit>,
    /// `Some(_)` while the "Save theme" flow is asking for a choice or a name.
    save: Option<SaveState>,
}

impl AppearancePopup {
    pub fn new() -> Self {
        Self {
            cursor: 0,
            editing: None,
            save: None,
        }
    }

    pub fn rows(&self) -> &'static [Row] {
        &ROWS
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The row being edited, if any — in either text or picker mode.
    pub fn editing_row(&self) -> Option<Row> {
        self.editing.is_some().then(|| ROWS[self.cursor])
    }

    /// The in-progress text buffer, if the row under the cursor is being edited as text. `None`
    /// while it's in picker mode instead — see `editing_picker`.
    pub fn editing_buffer(&self) -> Option<&str> {
        match &self.editing {
            Some(Edit::Text(buffer)) => Some(buffer),
            _ => None,
        }
    }

    /// The in-progress HSV value and which channel is selected, if the row under the cursor is
    /// being edited as a color picker.
    pub fn editing_picker(&self) -> Option<(Hsv, u8)> {
        match self.editing {
            Some(Edit::Picker { hsv, channel }) => Some((hsv, channel)),
            _ => None,
        }
    }

    /// Starts editing the row under the cursor as text, pre-filled with `initial` (its current
    /// value) — called after a `WantEdit` outcome, once the caller has looked that value up.
    pub fn begin_edit(&mut self, initial: String) {
        self.editing = Some(Edit::Text(initial));
    }

    /// What the "Save theme" flow should ask, from `Outcome::WantSaveTheme` — a no-op for
    /// `SaveChoice::Unchanged`, since there's nothing to save.
    pub fn begin_save(&mut self, choice: SaveChoice) {
        self.save = match choice {
            SaveChoice::New => Some(SaveState::Name(String::new())),
            SaveChoice::UpdateOrNew(name) => Some(SaveState::Choice(name)),
            SaveChoice::Unchanged(_) => None,
        };
    }

    /// What the save flow is currently showing, for `overlay_view::render_appearance` — the name
    /// it would update, or the buffer being typed for a new one.
    pub fn save_view(&self) -> Option<SaveView<'_>> {
        match &self.save {
            Some(SaveState::Choice(name)) => Some(SaveView::Choice(name)),
            Some(SaveState::Name(buffer)) => Some(SaveView::Name(buffer)),
            None => None,
        }
    }

    /// `j`/`k`/the arrows move the cursor with wraparound; `h`/`l`/enter/left/right act on the row
    /// under it (edit a color, cycle a fixed value, save the theme, or run Reset); `Esc`/`q`
    /// closes the popup. While a row is being edited or the save flow is open, keys are routed to
    /// that instead — see `key_editing`/`key_saving`.
    pub fn key(&mut self, code: KeyCode) -> Outcome {
        if self.save.is_some() {
            return self.key_saving(code);
        }
        if self.editing.is_some() {
            return self.key_editing(code);
        }
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.cursor = (self.cursor + 1) % ROWS.len();
                Outcome::Stay
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor = (self.cursor + ROWS.len() - 1) % ROWS.len();
                Outcome::Stay
            }
            KeyCode::Char('h' | 'l') | KeyCode::Left | KeyCode::Right
                if matches!(ROWS[self.cursor].kind(), Some(RowKind::Cycle(_))) =>
            {
                Outcome::Cycle(ROWS[self.cursor])
            }
            KeyCode::Enter => self.activate(),
            KeyCode::Esc | KeyCode::Char('q') => Outcome::Close,
            _ => Outcome::Stay,
        }
    }

    /// Text typing (`Edit::Text`) and slider nudging (`Edit::Picker`) both live here; `Tab`
    /// converts the in-progress value across the two. `Enter` commits (converting a picker's HSV
    /// to hex first); `Esc` cancels back to whatever the row held before editing began.
    fn key_editing(&mut self, code: KeyCode) -> Outcome {
        match self.editing.take().expect("checked by the caller") {
            Edit::Text(mut buffer) => match code {
                KeyCode::Esc => Outcome::Stay,
                KeyCode::Enter => Outcome::Commit(ROWS[self.cursor], buffer),
                KeyCode::Tab => {
                    // An unparsable buffer (a color name, or a half-typed hex) starts the picker
                    // at white rather than losing the row's edit entirely.
                    let hsv = hex_to_hsv(&buffer).unwrap_or(Hsv {
                        h: 0.0,
                        s: 0.0,
                        v: 100.0,
                    });
                    self.editing = Some(Edit::Picker { hsv, channel: 0 });
                    Outcome::Stay
                }
                KeyCode::Backspace => {
                    buffer.pop();
                    self.editing = Some(Edit::Text(buffer));
                    Outcome::Stay
                }
                KeyCode::Char(c) => {
                    buffer.push(c);
                    self.editing = Some(Edit::Text(buffer));
                    Outcome::Stay
                }
                _ => {
                    self.editing = Some(Edit::Text(buffer));
                    Outcome::Stay
                }
            },
            Edit::Picker { hsv, channel } => match code {
                KeyCode::Esc => Outcome::Stay,
                KeyCode::Enter => Outcome::Commit(ROWS[self.cursor], hsv.to_hex()),
                KeyCode::Tab => {
                    self.editing = Some(Edit::Text(hsv.to_hex()));
                    Outcome::Stay
                }
                KeyCode::Up => {
                    self.editing = Some(Edit::Picker {
                        hsv,
                        channel: (channel + 2) % 3,
                    });
                    Outcome::Stay
                }
                KeyCode::Down => {
                    self.editing = Some(Edit::Picker {
                        hsv,
                        channel: (channel + 1) % 3,
                    });
                    Outcome::Stay
                }
                KeyCode::Left => {
                    self.editing = Some(Edit::Picker {
                        hsv: nudge(hsv, channel, -PICKER_STEP),
                        channel,
                    });
                    Outcome::Stay
                }
                KeyCode::Right => {
                    self.editing = Some(Edit::Picker {
                        hsv: nudge(hsv, channel, PICKER_STEP),
                        channel,
                    });
                    Outcome::Stay
                }
                _ => {
                    self.editing = Some(Edit::Picker { hsv, channel });
                    Outcome::Stay
                }
            },
        }
    }

    /// The update-or-new choice and the name buffer both live here. `Esc` at either step cancels
    /// the whole flow without saving anything.
    fn key_saving(&mut self, code: KeyCode) -> Outcome {
        match self.save.take().expect("checked by the caller") {
            SaveState::Choice(name) => match code {
                KeyCode::Char('u') => Outcome::SaveTheme(SaveTarget::Update(name)),
                KeyCode::Char('n') => {
                    self.save = Some(SaveState::Name(String::new()));
                    Outcome::Stay
                }
                KeyCode::Esc => Outcome::Stay,
                _ => {
                    self.save = Some(SaveState::Choice(name));
                    Outcome::Stay
                }
            },
            SaveState::Name(mut buffer) => match code {
                KeyCode::Esc => Outcome::Stay,
                KeyCode::Enter if !buffer.trim().is_empty() => {
                    Outcome::SaveTheme(SaveTarget::New(buffer.trim().to_string()))
                }
                KeyCode::Backspace => {
                    buffer.pop();
                    self.save = Some(SaveState::Name(buffer));
                    Outcome::Stay
                }
                KeyCode::Char(c) => {
                    buffer.push(c);
                    self.save = Some(SaveState::Name(buffer));
                    Outcome::Stay
                }
                _ => {
                    self.save = Some(SaveState::Name(buffer));
                    Outcome::Stay
                }
            },
        }
    }

    /// A left click on `row_index` (from `hit`), which acts on that row exactly like `Enter`
    /// would after moving the cursor there — a mouse user and a keyboard user reach the same
    /// outcome. Abandons any in-progress edit or save flow on a different row without committing
    /// it.
    pub fn click_row(&mut self, row_index: usize) -> Outcome {
        if row_index >= ROWS.len() {
            return Outcome::Stay;
        }
        self.cursor = row_index;
        self.editing = None;
        self.save = None;
        self.activate()
    }

    fn activate(&self) -> Outcome {
        match ROWS[self.cursor] {
            Row::Reset => Outcome::Reset,
            Row::SaveTheme => Outcome::WantSaveTheme,
            row => match row.kind().expect("every other row has a kind") {
                RowKind::Color => Outcome::WantEdit(row),
                RowKind::Cycle(_) => Outcome::Cycle(row),
            },
        }
    }
}

/// `hsv` with `channel`'s field (0 = hue, 1 = saturation, 2 = value) nudged by `delta` and
/// wrapped/clamped back into range.
fn nudge(hsv: Hsv, channel: u8, delta: f64) -> Hsv {
    match channel {
        0 => Hsv {
            h: hsv.h + delta,
            ..hsv
        },
        1 => Hsv {
            s: hsv.s + delta,
            ..hsv
        },
        _ => Hsv {
            v: hsv.v + delta,
            ..hsv
        },
    }
    .clamped()
}

/// What the save flow is showing, for `overlay_view::render_appearance` to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveView<'a> {
    /// Offers "update `.0`" or "save as new".
    Choice(&'a str),
    /// The buffer being typed for a new theme's name.
    Name(&'a str),
}

impl Default for AppearancePopup {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use theming::Theme;

    use super::*;

    #[test]
    fn j_and_k_move_the_cursor_with_wraparound() {
        let mut popup = AppearancePopup::new();
        assert_eq!(popup.cursor(), 0);
        assert_eq!(popup.key(KeyCode::Char('j')), Outcome::Stay);
        assert_eq!(popup.cursor(), 1);
        assert_eq!(popup.key(KeyCode::Char('k')), Outcome::Stay);
        assert_eq!(popup.cursor(), 0);
        assert_eq!(popup.key(KeyCode::Char('k')), Outcome::Stay);
        assert_eq!(
            popup.cursor(),
            ROWS.len() - 1,
            "up from the top wraps to the bottom"
        );
    }

    #[test]
    fn enter_on_the_theme_row_cycles_it_but_on_a_color_row_asks_to_edit_it() {
        let mut popup = AppearancePopup::new();
        assert_eq!(popup.key(KeyCode::Enter), Outcome::Cycle(Row::Theme));
        popup.key(KeyCode::Char('j')); // Accent
        assert_eq!(popup.key(KeyCode::Enter), Outcome::WantEdit(Row::Accent));
        assert_eq!(popup.editing_row(), None, "not editing until begin_edit");
    }

    #[test]
    fn h_and_l_cycle_a_cycle_row_but_do_nothing_on_a_color_row() {
        let mut popup = AppearancePopup::new();
        popup.key(KeyCode::Char('j')); // Accent, a color row
        assert_eq!(
            popup.key(KeyCode::Char('l')),
            Outcome::Stay,
            "Accent is a color row; h/l are not its edit key"
        );
        for _ in 0..6 {
            popup.key(KeyCode::Char('j'));
        }
        assert_eq!(popup.cursor(), 7); // BorderType
        assert_eq!(
            popup.key(KeyCode::Char('l')),
            Outcome::Cycle(Row::BorderType)
        );
        assert_eq!(
            popup.key(KeyCode::Char('h')),
            Outcome::Cycle(Row::BorderType)
        );
    }

    #[test]
    fn editing_types_backspaces_commits_and_cancels() {
        let mut popup = AppearancePopup::new();
        popup.key(KeyCode::Char('j')); // Accent
        popup.begin_edit("#ff2bd6".into());
        assert_eq!(popup.editing_row(), Some(Row::Accent));
        assert_eq!(popup.editing_buffer(), Some("#ff2bd6"));

        assert_eq!(popup.key(KeyCode::Backspace), Outcome::Stay);
        assert_eq!(popup.editing_buffer(), Some("#ff2bd"));
        assert_eq!(popup.key(KeyCode::Char('0')), Outcome::Stay);
        assert_eq!(popup.editing_buffer(), Some("#ff2bd0"));
        assert_eq!(
            popup.key(KeyCode::Enter),
            Outcome::Commit(Row::Accent, "#ff2bd0".into())
        );
        assert_eq!(popup.editing_row(), None, "Enter consumed the edit");

        popup.begin_edit("#ff2bd6".into());
        assert_eq!(popup.key(KeyCode::Esc), Outcome::Stay);
        assert_eq!(popup.editing_row(), None, "Esc cancels without committing");
    }

    #[test]
    fn moving_or_navigating_while_not_editing_never_touches_a_buffer() {
        let mut popup = AppearancePopup::new();
        assert_eq!(popup.key(KeyCode::Char('j')), Outcome::Stay);
        assert_eq!(popup.editing_row(), None);
    }

    #[test]
    fn esc_and_q_close_it_when_nothing_is_being_edited() {
        let mut popup = AppearancePopup::new();
        assert_eq!(popup.key(KeyCode::Esc), Outcome::Close);
        assert_eq!(popup.key(KeyCode::Char('q')), Outcome::Close);
    }

    #[test]
    fn the_reset_row_is_an_action_not_a_color_or_a_cycle() {
        assert!(Row::Reset.kind().is_none());
        let mut popup = AppearancePopup::new();
        for _ in 0..11 {
            popup.key(KeyCode::Char('j'));
        }
        assert_eq!(popup.cursor(), 11); // Reset
        assert_eq!(popup.key(KeyCode::Enter), Outcome::Reset);
    }

    #[test]
    fn the_save_theme_row_is_an_action_that_wants_a_save() {
        assert!(Row::SaveTheme.kind().is_none());
        let mut popup = AppearancePopup::new();
        for _ in 0..10 {
            popup.key(KeyCode::Char('j'));
        }
        assert_eq!(popup.cursor(), 10); // SaveTheme
        assert_eq!(popup.key(KeyCode::Enter), Outcome::WantSaveTheme);
    }

    #[test]
    fn click_row_moves_the_cursor_and_acts_like_enter_there() {
        let mut popup = AppearancePopup::new();
        assert_eq!(popup.click_row(0), Outcome::Cycle(Row::Theme));
        assert_eq!(popup.click_row(7), Outcome::Cycle(Row::BorderType));
        assert_eq!(popup.cursor(), 7);
        assert_eq!(
            popup.click_row(20),
            Outcome::Stay,
            "out of range is ignored"
        );
    }

    #[test]
    fn clicking_another_row_abandons_an_in_progress_edit() {
        let mut popup = AppearancePopup::new();
        popup.key(KeyCode::Char('j')); // Accent
        popup.begin_edit("#ff2bd6".into());
        popup.click_row(2);
        assert_eq!(popup.editing_row(), None);
        assert_eq!(popup.cursor(), 2);
    }

    #[test]
    fn tab_toggles_between_text_and_picker_preserving_the_color() {
        let mut popup = AppearancePopup::new();
        popup.key(KeyCode::Char('j')); // Accent
        popup.begin_edit("#ff0000".into());

        assert_eq!(popup.key(KeyCode::Tab), Outcome::Stay);
        assert_eq!(
            popup.editing_buffer(),
            None,
            "picker mode has no text buffer"
        );
        let (hsv, channel) = popup.editing_picker().expect("now in picker mode");
        assert_eq!(channel, 0);
        assert!((hsv.h - 0.0).abs() < 1.0, "red's hue");

        assert_eq!(popup.key(KeyCode::Tab), Outcome::Stay);
        assert_eq!(
            popup.editing_buffer(),
            Some("#ff0000"),
            "back to text with the same color"
        );
        assert_eq!(popup.editing_picker(), None);
    }

    #[test]
    fn an_unparsable_buffer_starts_the_picker_at_white_instead_of_losing_the_edit() {
        let mut popup = AppearancePopup::new();
        popup.key(KeyCode::Char('j')); // Accent
        popup.begin_edit("cyan".into()); // a name, not hex — can't be inverted to HSV
        popup.key(KeyCode::Tab);
        let (hsv, _) = popup.editing_picker().expect("still enters picker mode");
        assert_eq!((hsv.h, hsv.s, hsv.v), (0.0, 0.0, 100.0));
    }

    #[test]
    fn picker_up_down_switch_channel_and_left_right_nudge_it_with_wraparound() {
        let mut popup = AppearancePopup::new();
        popup.key(KeyCode::Char('j')); // Accent
        popup.begin_edit("#000000".into());
        popup.key(KeyCode::Tab);
        assert_eq!(popup.editing_picker().unwrap().1, 0); // hue selected

        popup.key(KeyCode::Down);
        assert_eq!(popup.editing_picker().unwrap().1, 1); // saturation
        popup.key(KeyCode::Down);
        assert_eq!(popup.editing_picker().unwrap().1, 2); // value
        popup.key(KeyCode::Down);
        assert_eq!(popup.editing_picker().unwrap().1, 0, "wraps back to hue");
        popup.key(KeyCode::Up);
        assert_eq!(
            popup.editing_picker().unwrap().1,
            2,
            "up from hue wraps to value"
        );

        // Black's value is 0; nudging it left must clamp at 0, not go negative.
        popup.key(KeyCode::Left);
        assert_eq!(popup.editing_picker().unwrap().0.v, 0.0);
        popup.key(KeyCode::Right);
        assert_eq!(popup.editing_picker().unwrap().0.v, PICKER_STEP);

        popup.key(KeyCode::Up); // back to hue
        for _ in 0..80 {
            popup.key(KeyCode::Left);
        }
        let hue = popup.editing_picker().unwrap().0.h;
        assert!((0.0..360.0).contains(&hue), "hue wraps rather than clamps");
    }

    #[test]
    fn enter_in_picker_mode_commits_the_hex_form() {
        let mut popup = AppearancePopup::new();
        popup.key(KeyCode::Char('j')); // Accent
        popup.begin_edit("#ff0000".into());
        popup.key(KeyCode::Tab);
        popup.key(KeyCode::Down); // saturation
        popup.key(KeyCode::Left); // drop it a bit
        let outcome = popup.key(KeyCode::Enter);
        let Outcome::Commit(Row::Accent, hex) = outcome else {
            panic!("expected a Commit, got {outcome:?}");
        };
        assert_eq!(hex.len(), 7);
        assert_ne!(hex, "#ff0000", "the saturation nudge changed the color");
        assert_eq!(popup.editing_row(), None, "Enter consumed the edit");
    }

    #[test]
    fn esc_in_picker_mode_cancels_without_committing() {
        let mut popup = AppearancePopup::new();
        popup.key(KeyCode::Char('j')); // Accent
        popup.begin_edit("#ff0000".into());
        popup.key(KeyCode::Tab);
        assert_eq!(popup.key(KeyCode::Esc), Outcome::Stay);
        assert_eq!(popup.editing_row(), None);
        assert_eq!(popup.editing_picker(), None);
    }

    #[test]
    fn save_theme_offers_update_or_new_and_update_reports_the_existing_name() {
        let mut popup = AppearancePopup::new();
        popup.begin_save(SaveChoice::UpdateOrNew("sunset".into()));
        assert!(matches!(popup.save_view(), Some(SaveView::Choice(n)) if n == "sunset"));
        assert_eq!(
            popup.key(KeyCode::Char('u')),
            Outcome::SaveTheme(SaveTarget::Update("sunset".into()))
        );
        assert_eq!(popup.save_view(), None, "the outcome ends the flow");
    }

    #[test]
    fn save_theme_choosing_new_from_an_update_prompt_asks_for_a_name() {
        let mut popup = AppearancePopup::new();
        popup.begin_save(SaveChoice::UpdateOrNew("sunset".into()));
        assert_eq!(popup.key(KeyCode::Char('n')), Outcome::Stay);
        assert!(matches!(popup.save_view(), Some(SaveView::Name(n)) if n.is_empty()));

        for c in "midnight".chars() {
            popup.key(KeyCode::Char(c));
        }
        assert_eq!(
            popup.key(KeyCode::Enter),
            Outcome::SaveTheme(SaveTarget::New("midnight".into()))
        );
    }

    #[test]
    fn save_theme_new_choice_skips_straight_to_naming() {
        let mut popup = AppearancePopup::new();
        popup.begin_save(SaveChoice::New);
        assert!(matches!(popup.save_view(), Some(SaveView::Name(n)) if n.is_empty()));
        popup.key(KeyCode::Char('x'));
        assert_eq!(
            popup.key(KeyCode::Enter),
            Outcome::SaveTheme(SaveTarget::New("x".into()))
        );
    }

    #[test]
    fn save_theme_unchanged_choice_never_opens_the_flow() {
        let mut popup = AppearancePopup::new();
        popup.begin_save(SaveChoice::Unchanged("sunset".into()));
        assert_eq!(popup.save_view(), None);
    }

    #[test]
    fn enter_does_not_save_an_all_whitespace_name() {
        let mut popup = AppearancePopup::new();
        popup.begin_save(SaveChoice::New);
        popup.key(KeyCode::Char(' '));
        assert_eq!(popup.key(KeyCode::Enter), Outcome::Stay);
    }

    #[test]
    fn esc_cancels_the_save_flow_at_either_step() {
        let mut popup = AppearancePopup::new();
        popup.begin_save(SaveChoice::UpdateOrNew("sunset".into()));
        assert_eq!(popup.key(KeyCode::Esc), Outcome::Stay);
        assert_eq!(popup.save_view(), None);

        popup.begin_save(SaveChoice::New);
        popup.key(KeyCode::Char('x'));
        assert_eq!(popup.key(KeyCode::Esc), Outcome::Stay);
        assert_eq!(popup.save_view(), None);
    }

    #[test]
    fn clicking_a_row_abandons_an_in_progress_save_flow() {
        let mut popup = AppearancePopup::new();
        popup.begin_save(SaveChoice::New);
        popup.click_row(1);
        assert_eq!(popup.save_view(), None);
        assert_eq!(popup.cursor(), 1);
    }

    #[test]
    fn next_in_wraps_and_an_unrecognised_current_starts_at_the_first_option() {
        let options = ["rounded", "plain", "double", "thick"];
        assert_eq!(next_in(&options, "rounded"), "plain");
        assert_eq!(next_in(&options, "thick"), "rounded");
        assert_eq!(next_in(&options, "nonexistent"), "rounded");
    }

    #[test]
    fn theme_name_prefers_an_exact_local_pick_then_a_resolved_match_then_the_first_name() {
        let dracula = Theme::named("dracula");
        assert_eq!(
            theme_name(Some("nord"), &dracula),
            "nord",
            "local pick wins"
        );
        assert_eq!(
            theme_name(None, &dracula),
            "dracula",
            "no local pick, but the resolved theme matches dracula exactly"
        );
        let customized = Theme {
            accent_fg: "#abcdef".into(),
            ..dracula
        };
        assert_eq!(
            theme_name(None, &customized),
            THEME_NAMES[0],
            "no local pick and no exact match falls back to the first name"
        );
    }

    #[test]
    fn view_reads_the_right_field_per_row() {
        let theme = Theme::named("dracula");
        let view = AppearanceView {
            theme: &theme,
            glyphs: GlyphSet::Nerd,
            theme_name: "dracula",
            active_custom_theme: None,
        };
        assert_eq!(view.value(Row::Theme), "dracula");
        assert_eq!(view.value(Row::Accent), theme.accent_fg);
        assert_eq!(view.value(Row::BorderFocused), theme.border_focused_fg);
        assert_eq!(view.value(Row::Selection), theme.selection_bg);
        assert_eq!(view.value(Row::Directory), theme.dir_fg);
        assert_eq!(view.value(Row::StatusBar), theme.bar_bg);
        assert_eq!(view.value(Row::Danger), theme.danger_fg);
        assert_eq!(view.value(Row::BorderType), theme.border_type);
        assert_eq!(view.value(Row::Separator), theme.separator);
        assert_eq!(view.value(Row::Glyphs), "nerd");
        assert_eq!(view.value(Row::SaveTheme), "new theme");

        let named = AppearanceView {
            active_custom_theme: Some("sunset"),
            ..view
        };
        assert_eq!(named.value(Row::SaveTheme), "updates 'sunset'");
    }

    #[test]
    fn preview_replaces_only_the_edited_field() {
        let theme = Theme::named("dracula");
        let previewed = preview(&theme, Row::Accent, "#123456");
        assert_eq!(previewed.accent_fg, "#123456");
        assert_eq!(previewed.border_focused_fg, theme.border_focused_fg);

        // A cycle/action row is a no-op for `preview` — nothing in `Theme` to replace with free text.
        assert_eq!(preview(&theme, Row::BorderType, "whatever"), theme);
    }

    #[test]
    fn commit_writes_only_the_named_field() {
        let mut overrides = RawTheme::default();
        commit(&mut overrides, Row::Danger, "#ff0000".into());
        assert_eq!(overrides.danger_fg.as_deref(), Some("#ff0000"));
        assert_eq!(overrides.accent_fg, None);

        commit(&mut overrides, Row::Separator, "arrow".into());
        assert_eq!(overrides.separator.as_deref(), Some("arrow"));

        commit(&mut overrides, Row::Theme, "nord".into());
        assert_eq!(overrides.name.as_deref(), Some("nord"));
    }

    #[test]
    fn hit_finds_the_row_under_the_blank_line_and_outside_is_outside() {
        let area = Rect::new(10, 5, 40, 17); // border + blank + 12 rows + blank + hint + border
        let inner = area.inner(Margin::new(1, 1));
        assert_eq!(hit(area, Position::new(5, 5)), Hit::Outside);
        assert_eq!(hit(area, Position::new(inner.x, inner.y)), Hit::Inert);
        assert_eq!(hit(area, Position::new(inner.x, inner.y + 1)), Hit::Row(0));
        assert_eq!(
            hit(area, Position::new(inner.x, inner.y + 12)),
            Hit::Row(11)
        );
    }

    proptest::proptest! {
        /// Whatever sequence of moves the cursor sees, it always names one of the popup's rows.
        #[test]
        fn the_cursor_always_stays_in_bounds(moves in proptest::collection::vec(0u8..2, 0..200)) {
            let mut popup = AppearancePopup::new();
            for m in moves {
                let code = if m == 0 { KeyCode::Char('j') } else { KeyCode::Char('k') };
                popup.key(code);
                proptest::prop_assert!(popup.cursor() < ROWS.len());
            }
        }

        /// However `hit` is queried, a `Hit::Row` it returns always names an existing row.
        #[test]
        fn hit_never_names_a_missing_row(x in 0u16..80, y in 0u16..40) {
            let area = Rect::new(10, 5, 40, 14);
            if let Hit::Row(row) = hit(area, Position::new(x, y)) {
                proptest::prop_assert!(row < ROWS.len());
            }
        }
    }
}
