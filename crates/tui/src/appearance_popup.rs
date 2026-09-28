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
//! The popup is two levels deep: a root screen listing five categories (`Category`) plus "Save
//! theme"/"Reset to defaults", and — once a category is entered — a flat list of that category's
//! own rows, with a "‹ Back" row at the top. `View` tracks which level is showing; `rows()` builds
//! the current level's list fresh each time rather than storing a `'static` slice, since which
//! rows exist now depends on where the cursor drilled into.
//!
//! A color row is edited either as free-form text (a name or `#rrggbb`/`#rgb` hex, exactly what
//! `appearance.toml` accepts) or, since `Tab` toggles into it, as a real two-axis picker: a
//! saturation/value gradient square plus a separate hue strip, the same shape a desktop color
//! dialog uses. `picker_areas` and the `hsv_from_*_click` functions are pure geometry — the same
//! rectangles `overlay_view::render_appearance` paints the gradient into are what a click is
//! hit-tested against, so the two can never disagree about where a color landed. A "Text styles"
//! row (a per-element modifier list) and the two "Font" rows are edited as plain text with no
//! picker, reusing the same buffer machinery.

use crossterm::event::KeyCode;
use ratatui::layout::{Margin, Position, Rect};
use theming::{
    CustomTheme, Font, GlyphSet, Hsv, RawTheme, STYLE_ELEMENTS, Styles, Theme, hex_to_hsv,
};

/// The built-in palettes the "Theme" row cycles through, in order. `catppuccin-latte` isn't
/// here — it's only reached via `Theme::auto` on a light terminal, or by naming it explicitly in
/// `appearance.toml`; this cycle sticks to dark-background palettes, like `neon`, `dracula` and
/// `nord` already do.
pub const THEME_NAMES: [&str; 5] = ["neon", "classic", "dracula", "catppuccin", "nord"];

/// One of the appearance popup's root-level groups — entering one replaces the row list with that
/// group's own rows (see `AppearancePopup::rows`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    ThemeColors,
    BorderSeparator,
    Glyphs,
    Styles,
    Font,
    /// The custom themes saved from "Save theme": apply, rename or delete one.
    SavedThemes,
}

impl Category {
    /// The root row's own label (with its trailing "›"). `overlay_view::render_appearance` also
    /// uses this, trimmed, for the popup's border title while a category is open.
    pub fn label(self) -> &'static str {
        match self {
            Category::ThemeColors => "Theme & colors ›",
            Category::BorderSeparator => "Border & separator ›",
            Category::Glyphs => "Glyphs ›",
            Category::Styles => "Text styles ›",
            Category::Font => "Font ›",
            Category::SavedThemes => "Saved themes ›",
        }
    }
}

/// One row of the popup, in display order. Which rows exist at a given moment depends on
/// `AppearancePopup`'s current `View` — see `AppearancePopup::rows`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A root-level row that drills into that category.
    Category(Category),
    /// The first row of every category: drills back out to the root.
    Back,
    /// Picks the base palette everything else layers on top of. Lives in `Category::ThemeColors`.
    Theme,
    Selection,
    SelectionText,
    Border,
    BorderFocused,
    Title,
    Accent,
    Directory,
    File,
    Source,
    ConfigFile,
    Doc,
    Archive,
    Media,
    SyntaxKeyword,
    SyntaxString,
    SyntaxComment,
    SyntaxNumber,
    SyntaxFunction,
    SyntaxType,
    StatusText,
    StatusBar,
    Danger,
    /// Lives in `Category::BorderSeparator`.
    BorderType,
    /// Lives in `Category::BorderSeparator`.
    Separator,
    /// Lives in `Category::Glyphs`.
    Glyphs,
    /// An index into `STYLE_ELEMENTS`. Lives in `Category::Styles`.
    StyleElement(usize),
    /// Lives in `Category::Font`.
    FontFamily,
    /// Lives in `Category::Font`.
    FontSize,
    /// An index into the saved custom themes (see `AppearancePopup::sync_saved`). Lives in
    /// `Category::SavedThemes`; its label is the theme's own name, which `Row::label` can't return
    /// (it is `'static`), so `overlay_view` asks `AppearancePopup::saved_name` instead.
    SavedTheme(usize),
    /// Saves the live look as a named custom theme — updating the one it was loaded from/last
    /// saved as, if it still matches one, or prompting for a new name otherwise. Root-level.
    SaveTheme,
    /// Clears every override this popup has made this session, in one step. Root-level.
    Reset,
}

/// `Category::ThemeColors`'s 22 color rows, in `Theme`'s own field order (after `Theme` itself,
/// which is a `Cycle` row rather than a `Color` one).
const COLOR_ROWS: [Row; 22] = [
    Row::Selection,
    Row::SelectionText,
    Row::Border,
    Row::BorderFocused,
    Row::Title,
    Row::Accent,
    Row::Directory,
    Row::File,
    Row::Source,
    Row::ConfigFile,
    Row::Doc,
    Row::Archive,
    Row::Media,
    Row::SyntaxKeyword,
    Row::SyntaxString,
    Row::SyntaxComment,
    Row::SyntaxNumber,
    Row::SyntaxFunction,
    Row::SyntaxType,
    Row::StatusText,
    Row::StatusBar,
    Row::Danger,
];

/// What kind of value a row holds, and so how a keystroke or a click on it behaves. `Row::Reset`,
/// `Row::SaveTheme`, `Row::Category`, and `Row::Back` have none — they are actions or navigation,
/// not values — so `Row::kind` returns `None` for them.
#[derive(Debug, Clone, Copy)]
pub enum RowKind {
    /// A free-form name or hex value, edited as text or as the gradient/hue picker.
    Color,
    /// One of a fixed list, cycled like the settings popup's rows.
    Cycle(&'static [&'static str]),
    /// Free-form text with no picker: a "Text styles" element's modifier list, or a font field.
    Text,
}

impl Row {
    pub fn label(self) -> &'static str {
        match self {
            Row::Category(cat) => cat.label(),
            Row::Back => "‹ Back",
            Row::Theme => "Theme",
            Row::Selection => "Selection",
            Row::SelectionText => "Selection text",
            Row::Border => "Border",
            Row::BorderFocused => "Focused border",
            Row::Title => "Title",
            Row::Accent => "Accent",
            Row::Directory => "Directory",
            Row::File => "File",
            Row::Source => "Source file",
            Row::ConfigFile => "Config file",
            Row::Doc => "Document",
            Row::Archive => "Archive",
            Row::Media => "Media",
            Row::SyntaxKeyword => "Syntax: keyword",
            Row::SyntaxString => "Syntax: string",
            Row::SyntaxComment => "Syntax: comment",
            Row::SyntaxNumber => "Syntax: number",
            Row::SyntaxFunction => "Syntax: function",
            Row::SyntaxType => "Syntax: type",
            Row::StatusText => "Status text",
            Row::StatusBar => "Status bar",
            Row::Danger => "Danger",
            Row::BorderType => "Border style",
            Row::Separator => "Separator",
            Row::Glyphs => "Glyphs",
            Row::StyleElement(i) => STYLE_ELEMENTS.get(i).copied().unwrap_or("?"),
            Row::FontFamily => "Font family",
            Row::FontSize => "Font size",
            Row::SavedTheme(_) => "(saved theme)",
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
            Row::StyleElement(_) | Row::FontFamily | Row::FontSize => Some(RowKind::Text),
            Row::Category(_) | Row::Back | Row::SavedTheme(_) | Row::SaveTheme | Row::Reset => None,
            Row::Selection
            | Row::SelectionText
            | Row::Border
            | Row::BorderFocused
            | Row::Title
            | Row::Accent
            | Row::Directory
            | Row::File
            | Row::Source
            | Row::ConfigFile
            | Row::Doc
            | Row::Archive
            | Row::Media
            | Row::SyntaxKeyword
            | Row::SyntaxString
            | Row::SyntaxComment
            | Row::SyntaxNumber
            | Row::SyntaxFunction
            | Row::SyntaxType
            | Row::StatusText
            | Row::StatusBar
            | Row::Danger => Some(RowKind::Color),
        }
    }
}

/// Which level of the popup is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Root,
    Category(Category),
}

/// The current value of every row, for `overlay_view::render_appearance` to draw and for `main`
/// to read the "before" value off of when a row's edit begins. Built fresh each frame (or each
/// time it's needed) from the session's live theme/glyphs/styles/font, not stored on the popup.
pub struct AppearanceView<'a> {
    pub theme: &'a Theme,
    pub glyphs: GlyphSet,
    pub styles: &'a Styles,
    pub font: &'a Font,
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
            Row::Category(_) | Row::Back | Row::SavedTheme(_) => String::new(),
            Row::Theme => self.theme_name.to_string(),
            Row::Selection => self.theme.selection_bg.clone(),
            Row::SelectionText => self.theme.selection_fg.clone(),
            Row::Border => self.theme.border_fg.clone(),
            Row::BorderFocused => self.theme.border_focused_fg.clone(),
            Row::Title => self.theme.title_fg.clone(),
            Row::Accent => self.theme.accent_fg.clone(),
            Row::Directory => self.theme.dir_fg.clone(),
            Row::File => self.theme.file_fg.clone(),
            Row::Source => self.theme.source_fg.clone(),
            Row::ConfigFile => self.theme.config_fg.clone(),
            Row::Doc => self.theme.doc_fg.clone(),
            Row::Archive => self.theme.archive_fg.clone(),
            Row::Media => self.theme.media_fg.clone(),
            Row::SyntaxKeyword => self.theme.syntax_keyword_fg.clone(),
            Row::SyntaxString => self.theme.syntax_string_fg.clone(),
            Row::SyntaxComment => self.theme.syntax_comment_fg.clone(),
            Row::SyntaxNumber => self.theme.syntax_number_fg.clone(),
            Row::SyntaxFunction => self.theme.syntax_function_fg.clone(),
            Row::SyntaxType => self.theme.syntax_type_fg.clone(),
            Row::StatusText => self.theme.status_fg.clone(),
            Row::StatusBar => self.theme.bar_bg.clone(),
            Row::Danger => self.theme.danger_fg.clone(),
            Row::BorderType => self.theme.border_type.clone(),
            Row::Separator => self.theme.separator.clone(),
            Row::Glyphs => glyph_name(self.glyphs).to_string(),
            Row::StyleElement(i) => self
                .styles
                .by_index(i)
                .map(|m| m.names().join(", "))
                .unwrap_or_default(),
            Row::FontFamily => self.font.family.clone(),
            Row::FontSize => format!("{:.1}", self.font.size),
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
/// row is mid-edit, before `Enter` commits it. A no-op for a row that isn't a theme color.
pub fn preview(theme: &Theme, row: Row, value: &str) -> Theme {
    let mut theme = theme.clone();
    match row {
        Row::Selection => theme.selection_bg = value.to_string(),
        Row::SelectionText => theme.selection_fg = value.to_string(),
        Row::Border => theme.border_fg = value.to_string(),
        Row::BorderFocused => theme.border_focused_fg = value.to_string(),
        Row::Title => theme.title_fg = value.to_string(),
        Row::Accent => theme.accent_fg = value.to_string(),
        Row::Directory => theme.dir_fg = value.to_string(),
        Row::File => theme.file_fg = value.to_string(),
        Row::Source => theme.source_fg = value.to_string(),
        Row::ConfigFile => theme.config_fg = value.to_string(),
        Row::Doc => theme.doc_fg = value.to_string(),
        Row::Archive => theme.archive_fg = value.to_string(),
        Row::Media => theme.media_fg = value.to_string(),
        Row::SyntaxKeyword => theme.syntax_keyword_fg = value.to_string(),
        Row::SyntaxString => theme.syntax_string_fg = value.to_string(),
        Row::SyntaxComment => theme.syntax_comment_fg = value.to_string(),
        Row::SyntaxNumber => theme.syntax_number_fg = value.to_string(),
        Row::SyntaxFunction => theme.syntax_function_fg = value.to_string(),
        Row::SyntaxType => theme.syntax_type_fg = value.to_string(),
        Row::StatusText => theme.status_fg = value.to_string(),
        Row::StatusBar => theme.bar_bg = value.to_string(),
        Row::Danger => theme.danger_fg = value.to_string(),
        Row::Category(_)
        | Row::Back
        | Row::Theme
        | Row::BorderType
        | Row::Separator
        | Row::Glyphs
        | Row::StyleElement(_)
        | Row::FontFamily
        | Row::FontSize
        | Row::SavedTheme(_)
        | Row::SaveTheme
        | Row::Reset => {}
    }
    theme
}

/// `styles` with the element at `index` replaced by whatever `buffer` parses to — the "Text
/// styles" category's live-preview equivalent of `preview`. An unparsable or empty buffer just
/// means "no modifiers," the same as an empty list in `appearance.toml`.
pub fn preview_style(styles: Styles, index: usize, buffer: &str) -> Styles {
    styles.with_index(index, theming::Mods::parse(&mod_names(buffer)))
}

/// Splits a comma-separated modifier list (`"bold, italic"`) into trimmed, non-empty names —
/// shared by `preview_style` and `commit_style` so a mid-edit preview and its eventual commit can
/// never disagree about how the buffer is parsed.
fn mod_names(buffer: &str) -> Vec<&str> {
    buffer
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Commits `value` into `overrides` for `row`'s field — a confirmed text edit or a cycled value
/// both end up here, including a `Theme` row pick (`RawTheme.name`). Rows that live outside
/// `RawTheme` (glyphs, a style element, a font field) are committed elsewhere — see
/// `commit_style` and `apply_appearance_outcome` in `main`.
pub fn commit(overrides: &mut RawTheme, row: Row, value: String) {
    match row {
        Row::Theme => overrides.name = Some(value),
        Row::Selection => overrides.selection_bg = Some(value),
        Row::SelectionText => overrides.selection_fg = Some(value),
        Row::Border => overrides.border_fg = Some(value),
        Row::BorderFocused => overrides.border_focused_fg = Some(value),
        Row::Title => overrides.title_fg = Some(value),
        Row::Accent => overrides.accent_fg = Some(value),
        Row::Directory => overrides.dir_fg = Some(value),
        Row::File => overrides.file_fg = Some(value),
        Row::Source => overrides.source_fg = Some(value),
        Row::ConfigFile => overrides.config_fg = Some(value),
        Row::Doc => overrides.doc_fg = Some(value),
        Row::Archive => overrides.archive_fg = Some(value),
        Row::Media => overrides.media_fg = Some(value),
        Row::SyntaxKeyword => overrides.syntax_keyword_fg = Some(value),
        Row::SyntaxString => overrides.syntax_string_fg = Some(value),
        Row::SyntaxComment => overrides.syntax_comment_fg = Some(value),
        Row::SyntaxNumber => overrides.syntax_number_fg = Some(value),
        Row::SyntaxFunction => overrides.syntax_function_fg = Some(value),
        Row::SyntaxType => overrides.syntax_type_fg = Some(value),
        Row::StatusText => overrides.status_fg = Some(value),
        Row::StatusBar => overrides.bar_bg = Some(value),
        Row::Danger => overrides.danger_fg = Some(value),
        Row::BorderType => overrides.border_type = Some(value),
        Row::Separator => overrides.separator = Some(value),
        Row::Category(_)
        | Row::Back
        | Row::Glyphs
        | Row::StyleElement(_)
        | Row::FontFamily
        | Row::FontSize
        | Row::SavedTheme(_)
        | Row::SaveTheme
        | Row::Reset => {}
    }
}

/// Commits a "Text styles" row's buffer into `overrides` at `STYLE_ELEMENTS[index]` — `commit`'s
/// counterpart for `RawStyles` instead of `RawTheme`. A no-op for an out-of-range `index`, the
/// same as `RawStyles::set_by_index` itself.
pub fn commit_style(overrides: &mut theming::RawStyles, index: usize, buffer: &str) {
    let names = mod_names(buffer)
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    overrides.set_by_index(index, Some(names));
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
    /// The row under the cursor holds a color or text value; the caller looks up its current
    /// value and calls `begin_edit` to actually enter text-editing mode.
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
    /// Make saved theme `.0` (an index into the list `sync_saved` last gave the popup) the live
    /// look.
    ApplyTheme(usize),
    /// Rename saved theme `.0` to `.1`, which the popup has already checked is non-empty and not
    /// another saved theme's name.
    RenameTheme(usize, String),
    /// Delete saved theme `.0`, after the popup's own confirmation.
    DeleteTheme(usize),
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
    /// Inside the popup's border but not on an actionable row (the blank line, the hint line, or
    /// — while a picker is open — the gradient/hue area, which `hit` doesn't cover; see
    /// `picker_areas` for that).
    Inert,
    Outside,
}

/// What `pos` is over, given the popup's outer rectangle (from `overlay_view::panel_area`, sized
/// with `row_count` rows) — the same geometry `overlay_view::render_appearance` draws with, so a
/// click and its drawn row can never disagree. `row_count` is `AppearancePopup::rows().len()` for
/// whichever view is currently showing.
pub fn hit(area: Rect, row_count: usize, pos: Position) -> Hit {
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
        row if row < row_count => Hit::Row(row),
        _ => Hit::Inert,
    }
}

/// How much a keyboard nudge moves the picker's hue (degrees) or saturation/value (percentage
/// points) per press. The same step for all three keeps the key feel consistent even though the
/// ranges differ.
const PICKER_STEP: f64 = 5.0;

/// The gradient square's size in cells — wide enough to place a color precisely, short enough
/// (terminal cells run roughly twice as tall as wide) that the square doesn't read as a stretched
/// rectangle.
pub const SV_BOX_WIDTH: u16 = 30;
pub const SV_BOX_HEIGHT: u16 = 8;
/// The hue strip is one cell tall — it only ever needs to place a position along a single axis.
pub const HUE_STRIP_HEIGHT: u16 = 1;
/// How many extra content lines the popup's panel needs, beyond one per row, while a color
/// picker is open: the gradient square, a blank line, the hue strip, a blank line, and the
/// H/S/V + hex readout line — see `overlay_view::panel_area`'s caller in `main`.
pub const PICKER_EXTRA_LINES: u16 = SV_BOX_HEIGHT + 1 + HUE_STRIP_HEIGHT + 1 + 1;

/// Where the picker's gradient square and hue strip sit within `area` (the popup's own outer
/// rect, as passed to `hit`), given `row_count` rows listed above them — the picker is drawn
/// below the row list rather than disturbing any row's own line, so `hit`'s row math never needs
/// to account for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickerAreas {
    pub sv_box: Rect,
    pub hue_strip: Rect,
}

pub fn picker_areas(area: Rect, row_count: usize) -> PickerAreas {
    let inner = area.inner(Margin::new(1, 1));
    // One blank line above the rows, `row_count` rows, then a blank line before the square.
    let top = inner
        .y
        .saturating_add(2 + row_count as u16)
        .min(inner.bottom());
    let width = SV_BOX_WIDTH.min(inner.width);
    let sv_box = Rect::new(
        inner.x,
        top,
        width,
        SV_BOX_HEIGHT.min(inner.bottom().saturating_sub(top)),
    );
    let hue_y = sv_box
        .y
        .saturating_add(sv_box.height)
        .saturating_add(1)
        .min(inner.bottom());
    let hue_strip = Rect::new(
        inner.x,
        hue_y,
        width,
        HUE_STRIP_HEIGHT.min(inner.bottom().saturating_sub(hue_y)),
    );
    PickerAreas { sv_box, hue_strip }
}

/// The `Hsv` a click at `pos` inside the gradient square picks, keeping `hue` fixed — `None` if
/// `pos` isn't over the square at all.
pub fn hsv_from_sv_click(area: Rect, row_count: usize, pos: Position, hue: f64) -> Option<Hsv> {
    let areas = picker_areas(area, row_count);
    let sv = areas.sv_box;
    if sv.width == 0 || sv.height == 0 || !sv.contains(pos) {
        return None;
    }
    let s = (f64::from(pos.x - sv.x) + 0.5) / f64::from(sv.width) * 100.0;
    let v = 100.0 - (f64::from(pos.y - sv.y) + 0.5) / f64::from(sv.height) * 100.0;
    Some(Hsv { h: hue, s, v }.clamped())
}

/// The `Hsv` a click at `pos` inside the hue strip picks, keeping `s`/`v` fixed — `None` if `pos`
/// isn't over the strip at all.
pub fn hue_from_strip_click(
    area: Rect,
    row_count: usize,
    pos: Position,
    s: f64,
    v: f64,
) -> Option<Hsv> {
    let areas = picker_areas(area, row_count);
    let strip = areas.hue_strip;
    if strip.width == 0 || strip.height == 0 || !strip.contains(pos) {
        return None;
    }
    let h = (f64::from(pos.x - strip.x) + 0.5) / f64::from(strip.width) * 360.0;
    Some(Hsv { h, s, v }.clamped())
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

/// The "Saved themes" category's rename and delete flows — like `SaveState`, a small modal step
/// over the row list rather than a row of its own.
#[derive(Debug, Clone, PartialEq)]
enum ManageState {
    /// Typing a new name for saved theme `index`; starts as its current name.
    Rename { index: usize, buffer: String },
    /// Asking `y`/`n` before deleting saved theme `.0`.
    ConfirmDelete(usize),
}

pub struct AppearancePopup {
    cursor: usize,
    view: View,
    /// `Some(_)` while the row under the cursor is being edited, in either mode.
    editing: Option<EditState>,
    /// `Some(_)` while the "Save theme" flow is asking for a choice or a name.
    save: Option<SaveState>,
    /// `Some(_)` while a saved theme is being renamed or is awaiting delete confirmation.
    manage: Option<ManageState>,
    /// The saved custom themes' names, in `local.toml` order. The popup never sees `Config` or
    /// `local.toml` (see `Outcome::WantSaveTheme`), so `main` hands it this list through
    /// `sync_saved` instead; it is what gives `Category::SavedThemes` its rows.
    saved: Vec<String>,
}

/// A color row's in-progress edit: either typed as text (a name or `#rrggbb`/`#rgb` hex, exactly
/// what `appearance.toml` accepts) or adjusted as the gradient/hue picker — `Tab` swaps between
/// the two, converting the value across the swap so neither loses what the other set. A "Text
/// styles"/font row only ever uses `Text` — see `Row::kind`.
#[derive(Debug, Clone, PartialEq)]
enum EditState {
    Text(String),
    Picker(Hsv),
}

impl AppearancePopup {
    pub fn new() -> Self {
        Self {
            cursor: 0,
            view: View::Root,
            editing: None,
            save: None,
            manage: None,
            saved: Vec::new(),
        }
    }

    /// The rows the current view lists, built fresh from `self.view` — not stored, since which
    /// rows exist depends on whether a category has been entered.
    pub fn rows(&self) -> Vec<Row> {
        match self.view {
            View::Root => vec![
                Row::Category(Category::ThemeColors),
                Row::Category(Category::BorderSeparator),
                Row::Category(Category::Glyphs),
                Row::Category(Category::Styles),
                Row::Category(Category::Font),
                Row::Category(Category::SavedThemes),
                Row::SaveTheme,
                Row::Reset,
            ],
            View::Category(Category::ThemeColors) => {
                let mut rows = vec![Row::Back, Row::Theme];
                rows.extend(COLOR_ROWS);
                rows
            }
            View::Category(Category::BorderSeparator) => {
                vec![Row::Back, Row::BorderType, Row::Separator]
            }
            View::Category(Category::Glyphs) => vec![Row::Back, Row::Glyphs],
            View::Category(Category::Styles) => {
                let mut rows = vec![Row::Back];
                rows.extend((0..STYLE_ELEMENTS.len()).map(Row::StyleElement));
                rows
            }
            View::Category(Category::Font) => vec![Row::Back, Row::FontFamily, Row::FontSize],
            View::Category(Category::SavedThemes) => {
                let mut rows = vec![Row::Back];
                rows.extend((0..self.saved.len()).map(Row::SavedTheme));
                rows
            }
        }
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn view(&self) -> View {
        self.view
    }

    /// The row being edited, if any — in either text or picker mode.
    pub fn editing_row(&self) -> Option<Row> {
        self.editing.is_some().then(|| self.rows()[self.cursor])
    }

    /// The in-progress text buffer, if the row under the cursor is being edited as text. `None`
    /// while it's in picker mode instead — see `editing_picker`.
    pub fn editing_buffer(&self) -> Option<&str> {
        match &self.editing {
            Some(EditState::Text(buffer)) => Some(buffer),
            _ => None,
        }
    }

    /// The in-progress `Hsv`, if the row under the cursor is being edited as the gradient/hue
    /// picker.
    pub fn editing_picker(&self) -> Option<Hsv> {
        match self.editing {
            Some(EditState::Picker(hsv)) => Some(hsv),
            _ => None,
        }
    }

    /// Starts editing the row under the cursor as text, pre-filled with `initial` (its current
    /// value) — called after a `WantEdit` outcome, once the caller has looked that value up.
    pub fn begin_edit(&mut self, initial: String) {
        self.editing = Some(EditState::Text(initial));
    }

    /// Replaces the popup's copy of the saved custom themes' names with `themes`' — called by
    /// `main` whenever the list may have changed (a save, a rename, a delete, or the popup just
    /// opening). Any rename or delete in flight is dropped if the theme it targeted is gone, and
    /// the cursor is pulled back onto a row that still exists.
    pub fn sync_saved(&mut self, themes: &[CustomTheme]) {
        let names: Vec<String> = themes.iter().map(|t| t.name.clone()).collect();
        if names == self.saved {
            return;
        }
        self.saved = names;
        let stale = match &self.manage {
            Some(ManageState::Rename { index, .. }) | Some(ManageState::ConfirmDelete(index)) => {
                *index >= self.saved.len()
            }
            None => false,
        };
        if stale {
            self.manage = None;
        }
        self.cursor = self.cursor.min(self.rows().len() - 1);
    }

    /// The name of saved theme `index`, for `overlay_view` to use as that row's label.
    pub fn saved_name(&self, index: usize) -> Option<&str> {
        self.saved.get(index).map(String::as_str)
    }

    /// What the rename/delete flow is currently showing, for `overlay_view::render_appearance`.
    pub fn manage_view(&self) -> Option<ManageView<'_>> {
        match &self.manage {
            Some(ManageState::Rename { buffer, .. }) => Some(ManageView::Rename(buffer)),
            Some(ManageState::ConfirmDelete(index)) => {
                self.saved_name(*index).map(ManageView::ConfirmDelete)
            }
            None => None,
        }
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
    /// under it (edit a color/text value, cycle a fixed value, drill into a category, save the
    /// theme, or run Reset); `Esc` steps back out of a category, or closes the popup from the
    /// root; `q` always closes it outright. While a row is being edited or the save flow is open,
    /// keys are routed to that instead — see `key_editing`/`key_saving`.
    pub fn key(&mut self, code: KeyCode) -> Outcome {
        if self.save.is_some() {
            return self.key_saving(code);
        }
        if self.manage.is_some() {
            return self.key_managing(code);
        }
        if self.editing.is_some() {
            return self.key_editing(code);
        }
        let rows = self.rows();
        let saved_index = match rows[self.cursor] {
            Row::SavedTheme(index) => Some(index),
            _ => None,
        };
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.cursor = (self.cursor + 1) % rows.len();
                Outcome::Stay
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor = (self.cursor + rows.len() - 1) % rows.len();
                Outcome::Stay
            }
            KeyCode::Char('h' | 'l') | KeyCode::Left | KeyCode::Right
                if matches!(rows[self.cursor].kind(), Some(RowKind::Cycle(_))) =>
            {
                Outcome::Cycle(rows[self.cursor])
            }
            KeyCode::Char('r') if saved_index.is_some() => {
                let index = saved_index.expect("checked by the guard");
                let buffer = self.saved[index].clone();
                self.manage = Some(ManageState::Rename { index, buffer });
                Outcome::Stay
            }
            KeyCode::Char('d') if saved_index.is_some() => {
                self.manage = saved_index.map(ManageState::ConfirmDelete);
                Outcome::Stay
            }
            KeyCode::Enter => self.activate(),
            KeyCode::Esc => match self.view {
                View::Root => Outcome::Close,
                View::Category(_) => {
                    self.view = View::Root;
                    self.cursor = 0;
                    Outcome::Stay
                }
            },
            KeyCode::Char('q') => Outcome::Close,
            _ => Outcome::Stay,
        }
    }

    /// Text typing and the gradient/hue picker both live here; `Tab` converts the in-progress
    /// value across the two, only for a `RowKind::Color` row — a "Text styles"/font row has no
    /// picker to convert into. `Enter` commits (converting a picker's HSV to hex first); `Esc`
    /// cancels back to whatever the row held before editing began.
    fn key_editing(&mut self, code: KeyCode) -> Outcome {
        let row = self.rows()[self.cursor];
        let is_color = matches!(row.kind(), Some(RowKind::Color));
        match self.editing.take().expect("checked by the caller") {
            EditState::Text(mut buffer) => match code {
                KeyCode::Esc => Outcome::Stay,
                KeyCode::Enter => Outcome::Commit(row, buffer),
                KeyCode::Tab if is_color => {
                    // An unparsable buffer (a color name, or a half-typed hex) starts the picker
                    // at white rather than losing the row's edit entirely.
                    let hsv = hex_to_hsv(&buffer).unwrap_or(Hsv {
                        h: 0.0,
                        s: 0.0,
                        v: 100.0,
                    });
                    self.editing = Some(EditState::Picker(hsv));
                    Outcome::Stay
                }
                KeyCode::Backspace => {
                    buffer.pop();
                    self.editing = Some(EditState::Text(buffer));
                    Outcome::Stay
                }
                KeyCode::Char(c) => {
                    buffer.push(c);
                    self.editing = Some(EditState::Text(buffer));
                    Outcome::Stay
                }
                _ => {
                    self.editing = Some(EditState::Text(buffer));
                    Outcome::Stay
                }
            },
            EditState::Picker(hsv) => match code {
                KeyCode::Esc => Outcome::Stay,
                KeyCode::Enter => Outcome::Commit(row, hsv.to_hex()),
                KeyCode::Tab => {
                    self.editing = Some(EditState::Text(hsv.to_hex()));
                    Outcome::Stay
                }
                KeyCode::Up => {
                    self.editing = Some(EditState::Picker(nudge(hsv, 0.0, 0.0, PICKER_STEP)));
                    Outcome::Stay
                }
                KeyCode::Down => {
                    self.editing = Some(EditState::Picker(nudge(hsv, 0.0, 0.0, -PICKER_STEP)));
                    Outcome::Stay
                }
                KeyCode::Left => {
                    self.editing = Some(EditState::Picker(nudge(hsv, 0.0, -PICKER_STEP, 0.0)));
                    Outcome::Stay
                }
                KeyCode::Right => {
                    self.editing = Some(EditState::Picker(nudge(hsv, 0.0, PICKER_STEP, 0.0)));
                    Outcome::Stay
                }
                KeyCode::Char('[') => {
                    self.editing = Some(EditState::Picker(nudge(hsv, -PICKER_STEP, 0.0, 0.0)));
                    Outcome::Stay
                }
                KeyCode::Char(']') => {
                    self.editing = Some(EditState::Picker(nudge(hsv, PICKER_STEP, 0.0, 0.0)));
                    Outcome::Stay
                }
                _ => {
                    self.editing = Some(EditState::Picker(hsv));
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

    /// The rename buffer and the delete confirmation both live here. `Esc` cancels either without
    /// changing anything; a rename to an empty name, or to another saved theme's name, is ignored
    /// rather than committed, the same way an empty new name is in `key_saving`.
    fn key_managing(&mut self, code: KeyCode) -> Outcome {
        match self.manage.take().expect("checked by the caller") {
            ManageState::ConfirmDelete(index) => match code {
                KeyCode::Char('y') => Outcome::DeleteTheme(index),
                KeyCode::Char('n') | KeyCode::Esc => Outcome::Stay,
                _ => {
                    self.manage = Some(ManageState::ConfirmDelete(index));
                    Outcome::Stay
                }
            },
            ManageState::Rename { index, mut buffer } => match code {
                KeyCode::Esc => Outcome::Stay,
                KeyCode::Enter => {
                    let name = buffer.trim().to_string();
                    let clashes = self
                        .saved
                        .iter()
                        .enumerate()
                        .any(|(i, saved)| i != index && *saved == name);
                    if name.is_empty() || clashes {
                        self.manage = Some(ManageState::Rename { index, buffer });
                        Outcome::Stay
                    } else if self.saved.get(index) == Some(&name) {
                        // Unchanged: nothing to write.
                        Outcome::Stay
                    } else {
                        Outcome::RenameTheme(index, name)
                    }
                }
                KeyCode::Backspace => {
                    buffer.pop();
                    self.manage = Some(ManageState::Rename { index, buffer });
                    Outcome::Stay
                }
                KeyCode::Char(c) => {
                    buffer.push(c);
                    self.manage = Some(ManageState::Rename { index, buffer });
                    Outcome::Stay
                }
                _ => {
                    self.manage = Some(ManageState::Rename { index, buffer });
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
        if row_index >= self.rows().len() {
            return Outcome::Stay;
        }
        self.cursor = row_index;
        self.editing = None;
        self.save = None;
        self.manage = None;
        self.activate()
    }

    /// A left click at `pos` inside the gradient square or the hue strip, while a color row is
    /// being edited in picker mode — `false` if `pos` lands on neither (e.g. a click that missed
    /// while fine-tuning), in which case the caller does nothing rather than closing the popup.
    /// The same function drives a drag: called again on every `Drag` event with the pointer's
    /// current position.
    pub fn click_picker(&mut self, area: Rect, pos: Position) -> bool {
        let Some(EditState::Picker(hsv)) = self.editing else {
            return false;
        };
        let row_count = self.rows().len();
        if let Some(new_hsv) = hsv_from_sv_click(area, row_count, pos, hsv.h) {
            self.editing = Some(EditState::Picker(new_hsv));
            true
        } else if let Some(new_hsv) = hue_from_strip_click(area, row_count, pos, hsv.s, hsv.v) {
            self.editing = Some(EditState::Picker(new_hsv));
            true
        } else {
            false
        }
    }

    fn activate(&mut self) -> Outcome {
        let rows = self.rows();
        match rows[self.cursor] {
            Row::Back => {
                self.view = View::Root;
                self.cursor = 0;
                Outcome::Stay
            }
            Row::Category(cat) => {
                self.view = View::Category(cat);
                self.cursor = 0;
                Outcome::Stay
            }
            Row::Reset => Outcome::Reset,
            Row::SaveTheme => Outcome::WantSaveTheme,
            Row::SavedTheme(index) => Outcome::ApplyTheme(index),
            row => match row.kind().expect("every other row has a kind") {
                RowKind::Color | RowKind::Text => Outcome::WantEdit(row),
                RowKind::Cycle(_) => Outcome::Cycle(row),
            },
        }
    }
}

/// `hsv` with `dh`/`ds`/`dv` added to each channel and wrapped/clamped back into range.
fn nudge(hsv: Hsv, dh: f64, ds: f64, dv: f64) -> Hsv {
    Hsv {
        h: hsv.h + dh,
        s: hsv.s + ds,
        v: hsv.v + dv,
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

/// What the rename/delete flow is showing, for `overlay_view::render_appearance` to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManageView<'a> {
    /// The buffer being typed as a saved theme's new name.
    Rename(&'a str),
    /// The name of the saved theme awaiting a `y`/`n` delete confirmation.
    ConfirmDelete(&'a str),
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
            popup.rows().len() - 1,
            "up from the top wraps to the bottom"
        );
    }

    #[test]
    fn enter_on_a_category_drills_in_and_back_returns_to_the_root() {
        let mut popup = AppearancePopup::new();
        assert_eq!(popup.key(KeyCode::Enter), Outcome::Stay);
        assert_eq!(popup.view, View::Category(Category::ThemeColors));
        assert_eq!(popup.cursor(), 0);
        assert_eq!(popup.rows()[0], Row::Back);

        assert_eq!(popup.key(KeyCode::Enter), Outcome::Stay, "Back drills out");
        assert_eq!(popup.view, View::Root);
        assert_eq!(popup.cursor(), 0);
    }

    #[test]
    fn esc_steps_back_from_a_category_but_closes_from_the_root() {
        let mut popup = AppearancePopup::new();
        popup.key(KeyCode::Enter); // into Theme & colors
        assert_eq!(popup.key(KeyCode::Esc), Outcome::Stay);
        assert_eq!(popup.view, View::Root);
        assert_eq!(popup.key(KeyCode::Esc), Outcome::Close);
    }

    #[test]
    fn q_closes_even_from_inside_a_category() {
        let mut popup = AppearancePopup::new();
        popup.key(KeyCode::Enter);
        assert_eq!(popup.key(KeyCode::Char('q')), Outcome::Close);
    }

    #[test]
    fn a_color_row_wants_an_edit_and_a_cycle_row_cycles() {
        let mut popup = AppearancePopup::new();
        popup.key(KeyCode::Enter); // Theme & colors
        popup.key(KeyCode::Char('j')); // Theme -> Accent... first row after Back is Theme
        assert_eq!(popup.rows()[popup.cursor()], Row::Theme);
        assert_eq!(popup.key(KeyCode::Enter), Outcome::Cycle(Row::Theme));
        popup.key(KeyCode::Char('j')); // Selection
        assert_eq!(popup.rows()[popup.cursor()], Row::Selection);
        assert_eq!(popup.key(KeyCode::Enter), Outcome::WantEdit(Row::Selection));
        assert_eq!(popup.editing_row(), None, "not editing until begin_edit");
    }

    #[test]
    fn a_style_element_row_wants_an_edit_not_a_cycle() {
        let mut popup = AppearancePopup::new();
        popup.view = View::Category(Category::Styles);
        popup.key(KeyCode::Char('j')); // first style element after Back
        assert_eq!(
            popup.key(KeyCode::Enter),
            Outcome::WantEdit(Row::StyleElement(0))
        );
    }

    #[test]
    fn h_and_l_cycle_a_cycle_row_but_do_nothing_on_a_color_row() {
        let mut popup = AppearancePopup::new();
        popup.view = View::Category(Category::ThemeColors);
        popup.key(KeyCode::Char('j')); // Theme
        popup.key(KeyCode::Char('j')); // Selection, a color row
        assert_eq!(
            popup.key(KeyCode::Char('l')),
            Outcome::Stay,
            "Selection is a color row; h/l are not its edit key"
        );

        popup.view = View::Category(Category::BorderSeparator);
        popup.cursor = 0;
        popup.key(KeyCode::Char('j')); // BorderType
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
        popup.view = View::Category(Category::ThemeColors);
        popup.key(KeyCode::Char('j')); // Theme
        popup.key(KeyCode::Char('j')); // Selection
        popup.begin_edit("#ff2bd6".into());
        assert_eq!(popup.editing_row(), Some(Row::Selection));
        assert_eq!(popup.editing_buffer(), Some("#ff2bd6"));

        assert_eq!(popup.key(KeyCode::Backspace), Outcome::Stay);
        assert_eq!(popup.editing_buffer(), Some("#ff2bd"));
        assert_eq!(popup.key(KeyCode::Char('0')), Outcome::Stay);
        assert_eq!(popup.editing_buffer(), Some("#ff2bd0"));
        assert_eq!(
            popup.key(KeyCode::Enter),
            Outcome::Commit(Row::Selection, "#ff2bd0".into())
        );
        assert_eq!(popup.editing_row(), None, "Enter consumed the edit");

        popup.begin_edit("#ff2bd6".into());
        assert_eq!(popup.key(KeyCode::Esc), Outcome::Stay);
        assert_eq!(popup.editing_row(), None, "Esc cancels without committing");
    }

    #[test]
    fn tab_does_nothing_special_for_a_text_only_row() {
        let mut popup = AppearancePopup::new();
        popup.view = View::Category(Category::Font);
        popup.key(KeyCode::Char('j')); // Font family
        popup.begin_edit("Iosevka".into());
        assert_eq!(popup.key(KeyCode::Tab), Outcome::Stay);
        assert_eq!(
            popup.editing_buffer(),
            Some("Iosevka"),
            "Tab never opens a picker for a non-color row"
        );
        assert_eq!(popup.editing_picker(), None);
    }

    #[test]
    fn moving_or_navigating_while_not_editing_never_touches_a_buffer() {
        let mut popup = AppearancePopup::new();
        assert_eq!(popup.key(KeyCode::Char('j')), Outcome::Stay);
        assert_eq!(popup.editing_row(), None);
    }

    #[test]
    fn the_reset_row_is_an_action_not_a_color_or_a_cycle() {
        assert!(Row::Reset.kind().is_none());
        let mut popup = AppearancePopup::new();
        // Root: [ThemeColors, BorderSeparator, Glyphs, Styles, Font, SavedThemes, SaveTheme,
        // Reset]
        for _ in 0..7 {
            popup.key(KeyCode::Char('j'));
        }
        assert_eq!(popup.rows()[popup.cursor()], Row::Reset);
        assert_eq!(popup.key(KeyCode::Enter), Outcome::Reset);
    }

    #[test]
    fn the_save_theme_row_is_an_action_that_wants_a_save() {
        assert!(Row::SaveTheme.kind().is_none());
        let mut popup = AppearancePopup::new();
        for _ in 0..6 {
            popup.key(KeyCode::Char('j'));
        }
        assert_eq!(popup.rows()[popup.cursor()], Row::SaveTheme);
        assert_eq!(popup.key(KeyCode::Enter), Outcome::WantSaveTheme);
    }

    #[test]
    fn click_row_moves_the_cursor_and_acts_like_enter_there() {
        let mut popup = AppearancePopup::new();
        assert_eq!(
            popup.click_row(0),
            Outcome::Stay,
            "row 0 is a category; clicking it drills in"
        );
        assert_eq!(popup.view, View::Category(Category::ThemeColors));
        assert_eq!(
            popup.click_row(100),
            Outcome::Stay,
            "out of range is ignored"
        );
    }

    #[test]
    fn clicking_another_row_abandons_an_in_progress_edit() {
        let mut popup = AppearancePopup::new();
        popup.view = View::Category(Category::ThemeColors);
        popup.key(KeyCode::Char('j'));
        popup.key(KeyCode::Char('j')); // Selection
        popup.begin_edit("#ff2bd6".into());
        popup.click_row(0); // Back
        assert_eq!(popup.editing_row(), None);
        assert_eq!(popup.view, View::Root);
    }

    #[test]
    fn tab_toggles_between_text_and_picker_preserving_the_color() {
        let mut popup = AppearancePopup::new();
        popup.view = View::Category(Category::ThemeColors);
        popup.key(KeyCode::Char('j'));
        popup.key(KeyCode::Char('j')); // Selection
        popup.begin_edit("#ff0000".into());

        assert_eq!(popup.key(KeyCode::Tab), Outcome::Stay);
        assert_eq!(
            popup.editing_buffer(),
            None,
            "picker mode has no text buffer"
        );
        let hsv = popup.editing_picker().expect("now in picker mode");
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
        popup.view = View::Category(Category::ThemeColors);
        popup.key(KeyCode::Char('j'));
        popup.key(KeyCode::Char('j')); // Selection
        popup.begin_edit("cyan".into()); // a name, not hex — can't be inverted to HSV
        popup.key(KeyCode::Tab);
        let hsv = popup.editing_picker().expect("still enters picker mode");
        assert_eq!((hsv.h, hsv.s, hsv.v), (0.0, 0.0, 100.0));
    }

    #[test]
    fn arrows_nudge_saturation_and_value_and_brackets_nudge_hue() {
        let mut popup = AppearancePopup::new();
        popup.view = View::Category(Category::ThemeColors);
        popup.key(KeyCode::Char('j'));
        popup.key(KeyCode::Char('j')); // Selection
        popup.begin_edit("#000000".into());
        popup.key(KeyCode::Tab);

        // Black's value is 0; nudging it down must clamp at 0, not go negative.
        popup.key(KeyCode::Down);
        assert_eq!(popup.editing_picker().unwrap().v, 0.0);
        popup.key(KeyCode::Up);
        assert_eq!(popup.editing_picker().unwrap().v, PICKER_STEP);

        popup.key(KeyCode::Right);
        assert_eq!(popup.editing_picker().unwrap().s, PICKER_STEP);
        popup.key(KeyCode::Left);
        popup.key(KeyCode::Left);
        assert_eq!(
            popup.editing_picker().unwrap().s,
            0.0,
            "clamped, not negative"
        );

        for _ in 0..80 {
            popup.key(KeyCode::Char(']'));
        }
        let hue = popup.editing_picker().unwrap().h;
        assert!((0.0..360.0).contains(&hue), "hue wraps rather than clamps");
    }

    #[test]
    fn enter_in_picker_mode_commits_the_hex_form() {
        let mut popup = AppearancePopup::new();
        popup.view = View::Category(Category::ThemeColors);
        popup.key(KeyCode::Char('j'));
        popup.key(KeyCode::Char('j')); // Selection
        popup.begin_edit("#ff0000".into());
        popup.key(KeyCode::Tab);
        popup.key(KeyCode::Down); // drop the value a bit
        let outcome = popup.key(KeyCode::Enter);
        let Outcome::Commit(Row::Selection, hex) = outcome else {
            panic!("expected a Commit, got {outcome:?}");
        };
        assert_eq!(hex.len(), 7);
        assert_ne!(hex, "#ff0000", "the value nudge changed the color");
        assert_eq!(popup.editing_row(), None, "Enter consumed the edit");
    }

    #[test]
    fn esc_in_picker_mode_cancels_without_committing() {
        let mut popup = AppearancePopup::new();
        popup.view = View::Category(Category::ThemeColors);
        popup.key(KeyCode::Char('j'));
        popup.key(KeyCode::Char('j')); // Selection
        popup.begin_edit("#ff0000".into());
        popup.key(KeyCode::Tab);
        assert_eq!(popup.key(KeyCode::Esc), Outcome::Stay);
        assert_eq!(popup.editing_row(), None);
        assert_eq!(popup.editing_picker(), None);
    }

    #[test]
    fn click_picker_sets_saturation_value_from_the_square_and_hue_from_the_strip() {
        let mut popup = AppearancePopup::new();
        popup.view = View::Category(Category::ThemeColors);
        popup.key(KeyCode::Char('j'));
        popup.key(KeyCode::Char('j')); // Selection
        popup.begin_edit("#000000".into());
        popup.key(KeyCode::Tab);

        // Deliberately generous: big enough that the square and strip render at their full,
        // unclamped size regardless of exactly how tall a real popup panel computes itself.
        let area = Rect::new(0, 0, 76, 100);
        let row_count = popup.rows().len();
        let areas = picker_areas(area, row_count);

        // Top-left of the square: saturation rises left-to-right and value top-to-bottom, so the
        // top-left corner is near-zero saturation at near-full value.
        let top_left = Position::new(areas.sv_box.x, areas.sv_box.y);
        assert!(popup.click_picker(area, top_left));
        let hsv = popup.editing_picker().unwrap();
        assert!(hsv.s < 10.0 && hsv.v > 90.0);

        // A click on the hue strip changes hue but keeps s/v.
        let strip_right = Position::new(areas.hue_strip.right() - 1, areas.hue_strip.y);
        assert!(popup.click_picker(area, strip_right));
        let hsv2 = popup.editing_picker().unwrap();
        assert!(hsv2.h > 300.0);
        assert!((hsv2.s - hsv.s).abs() < 1.0 && (hsv2.v - hsv.v).abs() < 1.0);

        // A click outside both areas is reported as not handled.
        assert!(!popup.click_picker(area, Position::new(0, 0)));
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
        let styles = Styles::default();
        let font = Font::default();
        let view = AppearanceView {
            theme: &theme,
            glyphs: GlyphSet::Nerd,
            styles: &styles,
            font: &font,
            theme_name: "dracula",
            active_custom_theme: None,
        };
        assert_eq!(view.value(Row::Theme), "dracula");
        assert_eq!(view.value(Row::Selection), theme.selection_bg);
        assert_eq!(view.value(Row::Border), theme.border_fg);
        assert_eq!(view.value(Row::BorderFocused), theme.border_focused_fg);
        assert_eq!(view.value(Row::Directory), theme.dir_fg);
        assert_eq!(view.value(Row::StatusBar), theme.bar_bg);
        assert_eq!(view.value(Row::Danger), theme.danger_fg);
        assert_eq!(view.value(Row::BorderType), theme.border_type);
        assert_eq!(view.value(Row::Separator), theme.separator);
        assert_eq!(view.value(Row::Glyphs), "nerd");
        assert_eq!(view.value(Row::SaveTheme), "new theme");
        assert_eq!(
            view.value(Row::StyleElement(0)),
            "bold",
            "dir is bold by default"
        );
        assert_eq!(view.value(Row::FontFamily), font.family);
        assert_eq!(view.value(Row::FontSize), format!("{:.1}", font.size));
        assert_eq!(view.value(Row::Category(Category::ThemeColors)), "");
        assert_eq!(view.value(Row::Back), "");

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

        // A cycle/action/non-theme row is a no-op for `preview`.
        assert_eq!(preview(&theme, Row::BorderType, "whatever"), theme);
        assert_eq!(preview(&theme, Row::StyleElement(0), "whatever"), theme);
    }

    #[test]
    fn preview_style_replaces_only_the_edited_element() {
        let styles = Styles::default();
        let previewed = preview_style(styles, 0, "italic"); // index 0 is "dir"
        assert!(previewed.dir.italic && !previewed.dir.bold);
        assert_eq!(previewed.selection, styles.selection);
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

        // A non-`RawTheme` row is a no-op here.
        commit(&mut overrides, Row::FontFamily, "Iosevka".into());
        assert_eq!(
            overrides,
            RawTheme {
                danger_fg: Some("#ff0000".into()),
                separator: Some("arrow".into()),
                name: Some("nord".into()),
                ..RawTheme::default()
            }
        );
    }

    #[test]
    fn commit_style_writes_only_the_named_element() {
        let mut overrides = theming::RawStyles::default();
        commit_style(&mut overrides, 0, "bold, italic"); // index 0 is "dir"
        let resolved: Styles = overrides.into();
        assert!(resolved.dir.bold && resolved.dir.italic);
        assert_eq!(resolved.selection, Styles::default().selection);
    }

    #[test]
    fn hit_finds_the_row_under_the_blank_line_and_outside_is_outside() {
        let area = Rect::new(10, 5, 40, 9); // border + blank + 5 rows + blank + hint + border
        let inner = area.inner(Margin::new(1, 1));
        assert_eq!(hit(area, 5, Position::new(5, 5)), Hit::Outside);
        assert_eq!(hit(area, 5, Position::new(inner.x, inner.y)), Hit::Inert);
        assert_eq!(
            hit(area, 5, Position::new(inner.x, inner.y + 1)),
            Hit::Row(0)
        );
        assert_eq!(
            hit(area, 5, Position::new(inner.x, inner.y + 5)),
            Hit::Row(4)
        );
    }

    #[test]
    fn picker_areas_sit_below_the_rows_and_shrink_to_fit_a_small_frame() {
        let area = Rect::new(0, 0, 76, 7 + PICKER_EXTRA_LINES + 5);
        let areas = picker_areas(area, 7);
        assert!(areas.sv_box.height > 0 && areas.hue_strip.height > 0);
        assert!(areas.hue_strip.y > areas.sv_box.bottom());

        let tiny = picker_areas(Rect::new(0, 0, 20, 6), 7);
        assert_eq!(tiny.sv_box.height, 0, "no room left for the square");
    }

    proptest::proptest! {
        /// Whatever sequence of moves the cursor sees, it always names one of the current view's
        /// rows.
        #[test]
        fn the_cursor_always_stays_in_bounds(moves in proptest::collection::vec(0u8..3, 0..200)) {
            let mut popup = AppearancePopup::new();
            for m in moves {
                let code = match m {
                    0 => KeyCode::Char('j'),
                    1 => KeyCode::Char('k'),
                    _ => KeyCode::Enter,
                };
                popup.key(code);
                proptest::prop_assert!(popup.cursor() < popup.rows().len());
            }
        }

        /// However `hit` is queried, a `Hit::Row` it returns is always within `row_count`.
        #[test]
        fn hit_never_names_a_missing_row(x in 0u16..80, y in 0u16..40, row_count in 1usize..25) {
            let area = Rect::new(10, 5, 40, 14);
            if let Hit::Row(row) = hit(area, row_count, Position::new(x, y)) {
                proptest::prop_assert!(row < row_count);
            }
        }

        /// A click anywhere inside the gradient square always yields a saturation/value pair
        /// within range, and a click inside the hue strip always yields an in-range hue.
        #[test]
        fn picker_clicks_always_produce_in_range_values(
            x in 0u16..76, y in 0u16..40, row_count in 0usize..25,
        ) {
            let area = Rect::new(0, 0, 76, 40);
            let pos = Position::new(x, y);
            if let Some(hsv) = hsv_from_sv_click(area, row_count, pos, 180.0) {
                proptest::prop_assert!((0.0..=100.0).contains(&hsv.s));
                proptest::prop_assert!((0.0..=100.0).contains(&hsv.v));
            }
            if let Some(hsv) = hue_from_strip_click(area, row_count, pos, 50.0, 50.0) {
                proptest::prop_assert!((0.0..360.0).contains(&hsv.h));
            }
        }
    }

    fn themes(names: &[&str]) -> Vec<CustomTheme> {
        names
            .iter()
            .map(|name| CustomTheme {
                name: (*name).to_string(),
                theme: RawTheme::default(),
            })
            .collect()
    }

    /// A popup already inside "Saved themes" with `names` synced, cursor on the first saved theme.
    fn saved_themes_popup(names: &[&str]) -> AppearancePopup {
        let mut popup = AppearancePopup::new();
        popup.sync_saved(&themes(names));
        popup.view = View::Category(Category::SavedThemes);
        popup.cursor = 1;
        popup
    }

    #[test]
    fn saved_themes_category_lists_one_row_per_saved_theme_after_back() {
        let mut popup = AppearancePopup::new();
        popup.view = View::Category(Category::SavedThemes);
        assert_eq!(popup.rows(), vec![Row::Back], "nothing saved yet");
        popup.sync_saved(&themes(&["sunset", "midnight"]));
        assert_eq!(
            popup.rows(),
            vec![Row::Back, Row::SavedTheme(0), Row::SavedTheme(1)]
        );
        assert_eq!(popup.saved_name(1), Some("midnight"));
        assert_eq!(popup.saved_name(2), None);
    }

    #[test]
    fn enter_on_a_saved_theme_asks_to_apply_it() {
        let mut popup = saved_themes_popup(&["sunset", "midnight"]);
        popup.key(KeyCode::Char('j'));
        assert_eq!(popup.key(KeyCode::Enter), Outcome::ApplyTheme(1));
        assert_eq!(
            popup.click_row(1),
            Outcome::ApplyTheme(0),
            "a click acts like enter on that row"
        );
    }

    #[test]
    fn rename_starts_from_the_current_name_and_commits_the_edit() {
        let mut popup = saved_themes_popup(&["sunset", "midnight"]);
        assert_eq!(popup.key(KeyCode::Char('r')), Outcome::Stay);
        assert_eq!(popup.manage_view(), Some(ManageView::Rename("sunset")));
        popup.key(KeyCode::Backspace);
        popup.key(KeyCode::Char('!'));
        assert_eq!(popup.manage_view(), Some(ManageView::Rename("sunse!")));
        assert_eq!(
            popup.key(KeyCode::Enter),
            Outcome::RenameTheme(0, "sunse!".into())
        );
        assert_eq!(popup.manage_view(), None, "the flow ends on commit");
    }

    #[test]
    fn rename_ignores_an_empty_or_clashing_name_and_esc_cancels() {
        let mut popup = saved_themes_popup(&["sunset", "midnight"]);
        popup.key(KeyCode::Char('r'));
        for _ in 0.."sunset".len() {
            popup.key(KeyCode::Backspace);
        }
        assert_eq!(popup.key(KeyCode::Enter), Outcome::Stay, "empty is ignored");
        assert_eq!(popup.manage_view(), Some(ManageView::Rename("")));

        for c in "midnight".chars() {
            popup.key(KeyCode::Char(c));
        }
        assert_eq!(
            popup.key(KeyCode::Enter),
            Outcome::Stay,
            "a clash is ignored"
        );
        assert_eq!(popup.manage_view(), Some(ManageView::Rename("midnight")));

        assert_eq!(popup.key(KeyCode::Esc), Outcome::Stay);
        assert_eq!(popup.manage_view(), None);
    }

    #[test]
    fn renaming_to_the_unchanged_name_writes_nothing() {
        let mut popup = saved_themes_popup(&["sunset"]);
        popup.key(KeyCode::Char('r'));
        assert_eq!(popup.key(KeyCode::Enter), Outcome::Stay);
        assert_eq!(popup.manage_view(), None);
    }

    #[test]
    fn delete_needs_a_y_and_anything_but_y_or_n_keeps_asking() {
        let mut popup = saved_themes_popup(&["sunset", "midnight"]);
        popup.key(KeyCode::Char('d'));
        assert_eq!(
            popup.manage_view(),
            Some(ManageView::ConfirmDelete("sunset"))
        );
        assert_eq!(popup.key(KeyCode::Char('x')), Outcome::Stay);
        assert_eq!(
            popup.manage_view(),
            Some(ManageView::ConfirmDelete("sunset")),
            "an unrelated key doesn't dismiss the question"
        );
        assert_eq!(popup.key(KeyCode::Char('n')), Outcome::Stay);
        assert_eq!(popup.manage_view(), None, "n cancels");

        popup.key(KeyCode::Char('d'));
        assert_eq!(popup.key(KeyCode::Char('y')), Outcome::DeleteTheme(0));
    }

    #[test]
    fn r_and_d_do_nothing_off_a_saved_theme_row() {
        let mut popup = saved_themes_popup(&["sunset"]);
        popup.cursor = 0; // Back
        assert_eq!(popup.key(KeyCode::Char('r')), Outcome::Stay);
        assert_eq!(popup.key(KeyCode::Char('d')), Outcome::Stay);
        assert_eq!(popup.manage_view(), None);
    }

    #[test]
    fn sync_saved_pulls_the_cursor_back_and_drops_a_flow_on_a_vanished_theme() {
        let mut popup = saved_themes_popup(&["sunset", "midnight"]);
        popup.key(KeyCode::Char('j')); // midnight, the last row
        popup.key(KeyCode::Char('d'));
        popup.sync_saved(&themes(&["sunset"]));
        assert_eq!(popup.manage_view(), None, "its target no longer exists");
        assert_eq!(popup.cursor(), 1, "still on a row that exists");
        popup.sync_saved(&themes(&[]));
        assert_eq!(popup.cursor(), 0, "only Back is left");
    }

    #[test]
    fn a_saved_theme_row_is_an_action_with_no_value() {
        assert!(Row::SavedTheme(0).kind().is_none());
        let theme = Theme::default();
        let styles = Styles::default();
        let font = Font::default();
        let view = AppearanceView {
            theme: &theme,
            glyphs: GlyphSet::Unicode,
            styles: &styles,
            font: &font,
            theme_name: "neon",
            active_custom_theme: None,
        };
        assert_eq!(view.value(Row::SavedTheme(0)), "");
    }
}
