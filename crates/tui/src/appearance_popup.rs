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

//! The appearance popup's state: which page and row the cursor is on, whether one is being typed
//! into, and what a keystroke or a click means. Like `settings_popup` and `context_menu`, this is
//! pure — no `Frame`, no terminal — so `overlay_view::render_appearance` is the only place that
//! draws it and `main` is the only place that applies what it asks for.
//!
//! The popup is laid out like a desktop settings window: the pages (`Page`) are listed down the
//! left, and the chosen page's rows fill the right. `Tab`/`Shift+Tab` or the page's number switch
//! pages, and a click on one does too. The Theme page lists the built-in palettes and the themes
//! saved from "Save current look"; a saved theme is a snapshot of the colors and the shape. The
//! Shape page's numbers step with `h`/`l` (by five with `H`/`L`), its switches flip with `Enter`,
//! and every change applies at once.
//!
//! A color row is edited either as free-form text (a name or `#rrggbb`/`#rgb` hex, exactly what
//! `appearance.toml` accepts) or, since `Tab` toggles into it, as a real two-axis picker: a
//! saturation/value gradient square plus a separate hue strip, the same shape a desktop color
//! dialog uses. `areas`, `picker_areas` and the `hsv_from_*_click` functions are pure geometry —
//! the same rectangles `overlay_view::render_appearance` paints into are what a click is
//! hit-tested against, so the two can never disagree about where a click landed. A "Text" row (a
//! per-element modifier list) and the two font rows are edited as plain text with no picker,
//! reusing the same buffer machinery.

use crossterm::event::KeyCode;
use ratatui::layout::{Margin, Position, Rect};
use theming::{
    BORDER_STYLES, CustomTheme, Font, GlyphSet, Hsv, RawTheme, STYLE_ELEMENTS, Shape, Styles,
    Theme, TitleAlign, hex_to_hsv,
};

/// The built-in palettes the Theme page lists, in order. `catppuccin-latte` isn't here — it's
/// only reached via `Theme::auto` on a light terminal, or by naming it explicitly in
/// `appearance.toml`; this list sticks to dark-background palettes, like `neon`, `dracula` and
/// `nord` already do. `terminal` (built from the terminal's own colors) goes last so that, where
/// the terminal reports nothing and it equals `neon`, a neon theme is still shown as `neon`.
pub const THEME_NAMES: [&str; 7] = [
    "default",
    "neon",
    "classic",
    "dracula",
    "catppuccin",
    "nord",
    "terminal",
];

/// One page of the popup, listed down its left side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// The built-in palettes, the saved themes, saving the live look, and Reset.
    Theme,
    Colors,
    /// Frame style, title position, gaps and effects.
    Shape,
    /// Bold, italic and friends per element.
    Text,
    /// The glyph set, and the font `init-terminal` prints.
    Font,
}

impl Page {
    /// Every page, in the order they are listed and numbered (`1` is the first).
    pub const ALL: [Page; 5] = [
        Page::Theme,
        Page::Colors,
        Page::Shape,
        Page::Text,
        Page::Font,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Page::Theme => "Theme",
            Page::Colors => "Colors",
            Page::Shape => "Shape",
            Page::Text => "Text",
            Page::Font => "Font",
        }
    }

    fn index(self) -> usize {
        Page::ALL.iter().position(|&p| p == self).unwrap_or(0)
    }
}

/// One row of a page, in display order. Which rows exist depends on the page — see
/// `AppearancePopup::rows`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A built-in palette, by index into `THEME_NAMES`. Lives on `Page::Theme`.
    Preset(usize),
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
    /// The frame style. Lives on `Page::Shape`, like the rest of the shape rows below.
    BorderType,
    TitleAlign,
    GapsIn,
    GapsOut,
    Shadow,
    DimInactive,
    /// The status bar's segment edges.
    Separator,
    /// Lives on `Page::Font`.
    Glyphs,
    /// An index into `STYLE_ELEMENTS`. Lives on `Page::Text`.
    StyleElement(usize),
    /// Lives on `Page::Font`.
    FontFamily,
    /// Lives on `Page::Font`.
    FontSize,
    /// An index into the saved custom themes (see `AppearancePopup::sync_saved`). Lives on
    /// `Page::Theme`; its label is the theme's own name, which `Row::label` can't return (it is
    /// `'static`), so `overlay_view` asks `AppearancePopup::saved_name` instead.
    SavedTheme(usize),
    /// Saves the live look as a named custom theme — updating the one it was loaded from/last
    /// saved as, if it still matches one, or prompting for a new name otherwise.
    SaveTheme,
    /// Clears every override this popup has made, in one step.
    Reset,
}

/// `Page::Colors`' 22 rows, in `Theme`'s own field order.
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

/// `Page::Shape`'s rows: the frame and its title, then the spacing, then the effects.
const SHAPE_ROWS: [Row; 7] = [
    Row::BorderType,
    Row::TitleAlign,
    Row::GapsIn,
    Row::GapsOut,
    Row::Shadow,
    Row::DimInactive,
    Row::Separator,
];

/// What kind of value a row holds, and so how a keystroke or a click on it behaves. The Theme
/// page's rows have none — they are actions, not values — so `Row::kind` returns `None` for them.
#[derive(Debug, Clone, Copy)]
pub enum RowKind {
    /// A free-form name or hex value, edited as text or as the gradient/hue picker.
    Color,
    /// One of a fixed list, cycled like the settings popup's rows.
    Cycle(&'static [&'static str]),
    /// Free-form text with no picker: a "Text" element's modifier list, or a font field.
    Text,
    /// A number of cells, stepped up and down within its range.
    Step,
    /// On or off.
    Toggle,
}

impl Row {
    pub fn label(self) -> &'static str {
        match self {
            Row::Preset(i) => THEME_NAMES.get(i).copied().unwrap_or("?"),
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
            Row::TitleAlign => "Title position",
            Row::GapsIn => "Gap between panes",
            Row::GapsOut => "Gap to the edge",
            Row::Shadow => "Popup shadows",
            Row::DimInactive => "Dim unfocused panes",
            Row::Separator => "Bar separator",
            Row::Glyphs => "Glyphs",
            Row::StyleElement(i) => STYLE_ELEMENTS.get(i).copied().unwrap_or("?"),
            Row::FontFamily => "Font family",
            Row::FontSize => "Font size",
            Row::SavedTheme(_) => "(saved theme)",
            Row::SaveTheme => "Save current look",
            Row::Reset => "Reset to defaults",
        }
    }

    pub fn kind(self) -> Option<RowKind> {
        match self {
            Row::BorderType => Some(RowKind::Cycle(&BORDER_STYLES)),
            Row::TitleAlign => Some(RowKind::Cycle(&TitleAlign::NAMES)),
            Row::Separator => Some(RowKind::Cycle(&["flat", "arrow", "auto"])),
            Row::Glyphs => Some(RowKind::Cycle(&["unicode", "nerd", "ascii"])),
            Row::GapsIn | Row::GapsOut => Some(RowKind::Step),
            Row::Shadow | Row::DimInactive => Some(RowKind::Toggle),
            Row::StyleElement(_) | Row::FontFamily | Row::FontSize => Some(RowKind::Text),
            Row::Preset(_) | Row::SavedTheme(_) | Row::SaveTheme | Row::Reset => None,
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

/// The current value of every row, for `overlay_view::render_appearance` to draw and for `main`
/// to read the "before" value off of when a row's edit begins. Built fresh each frame (or each
/// time it's needed) from the session's live theme/glyphs/styles/font, not stored on the popup.
pub struct AppearanceView<'a> {
    pub theme: &'a Theme,
    pub glyphs: GlyphSet,
    pub styles: &'a Styles,
    pub font: &'a Font,
    /// The built-in palette the live colors are, if they are one exactly (see `active_preset`).
    pub active_preset: Option<&'static str>,
    /// The name of the saved custom theme the live look started from (or was last saved as),
    /// if any — shown next to `Row::SaveTheme` so it's clear whether that row will update an
    /// existing theme or start a new one.
    pub active_custom_theme: Option<&'a str>,
}

impl AppearanceView<'_> {
    pub fn value(&self, row: Row) -> String {
        let shape = &self.theme.shape;
        match row {
            Row::SavedTheme(_) | Row::Reset => String::new(),
            Row::Preset(i) => {
                if self.active_preset == THEME_NAMES.get(i).copied() {
                    "built-in, active".to_string()
                } else {
                    "built-in".to_string()
                }
            }
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
            Row::TitleAlign => shape.title_align.name().to_string(),
            Row::GapsIn => cells(shape.gaps_in.cells()),
            Row::GapsOut => cells(shape.gaps_out.cells()),
            Row::Shadow => on_off(shape.shadow),
            Row::DimInactive => on_off(shape.dim_inactive),
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
        }
    }
}

fn cells(n: u16) -> String {
    if n == 1 {
        "1 cell".to_string()
    } else {
        format!("{n} cells")
    }
}

fn on_off(on: bool) -> String {
    if on { "on" } else { "off" }.to_string()
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
        Row::Preset(_)
        | Row::BorderType
        | Row::TitleAlign
        | Row::GapsIn
        | Row::GapsOut
        | Row::Shadow
        | Row::DimInactive
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

/// `styles` with the element at `index` replaced by whatever `buffer` parses to — the "Text"
/// page's live-preview equivalent of `preview`. An unparsable or empty buffer just means "no
/// modifiers," the same as an empty list in `appearance.toml`.
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
/// both end up here. Rows that live outside `RawTheme` (glyphs, a style element, a font field)
/// are committed elsewhere — see `commit_style` and `apply_appearance_outcome` in `main`; the
/// shape's numbers and switches go through `step` and `toggle`.
pub fn commit(overrides: &mut RawTheme, row: Row, value: String) {
    match row {
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
        Row::TitleAlign => overrides.shape.title_align = Some(value),
        Row::Separator => overrides.separator = Some(value),
        Row::Preset(_)
        | Row::GapsIn
        | Row::GapsOut
        | Row::Shadow
        | Row::DimInactive
        | Row::Glyphs
        | Row::StyleElement(_)
        | Row::FontFamily
        | Row::FontSize
        | Row::SavedTheme(_)
        | Row::SaveTheme
        | Row::Reset => {}
    }
}

/// Moves a gap row `delta` cells from `live` (the shape in effect), stopping at either end of its
/// range, and records the result in `overrides`. A no-op for any other row.
pub fn step(overrides: &mut RawTheme, row: Row, live: &Shape, delta: i64) {
    match row {
        Row::GapsIn => {
            overrides.shape.gaps_in = Some(i64::from(live.gaps_in.stepped(delta).cells()));
        }
        Row::GapsOut => {
            overrides.shape.gaps_out = Some(i64::from(live.gaps_out.stepped(delta).cells()));
        }
        _ => {}
    }
}

/// Flips a switch row from its value in `live` (the shape in effect) and records the result in
/// `overrides`. A no-op for any other row.
pub fn toggle(overrides: &mut RawTheme, row: Row, live: &Shape) {
    match row {
        Row::Shadow => overrides.shape.shadow = Some(!live.shadow),
        Row::DimInactive => overrides.shape.dim_inactive = Some(!live.dim_inactive),
        _ => {}
    }
}

/// The overrides that show the built-in palette `name` as it is: its colors replace every color
/// override, while the frame style, separator and shape — the Shape page's — stay as they were.
pub fn apply_preset(overrides: &RawTheme, name: &str) -> RawTheme {
    RawTheme {
        name: Some(name.to_string()),
        border_type: overrides.border_type.clone(),
        separator: overrides.separator.clone(),
        shape: overrides.shape.clone(),
        ..RawTheme::default()
    }
}

/// Commits a "Text" row's buffer into `overrides` at `STYLE_ELEMENTS[index]` — `commit`'s
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

/// The built-in palette `theme`'s colors are exactly, if any: `local_name` (the popup's own pick)
/// when it still matches, else the first listed palette that does. Only the colors are compared —
/// the frame style, separator and shape are the Shape page's and never make a palette inactive.
/// `None` once any color has been changed by hand.
pub fn active_preset(local_name: Option<&str>, theme: &Theme) -> Option<&'static str> {
    let same_colors = |name: &str| {
        Theme {
            border_type: theme.border_type.clone(),
            separator: theme.separator.clone(),
            shape: theme.shape,
            ..Theme::named(name)
        } == *theme
    };
    local_name
        .and_then(|name| THEME_NAMES.iter().copied().find(|&t| t == name))
        .filter(|name| same_colors(name))
        .or_else(|| THEME_NAMES.iter().copied().find(|name| same_colors(name)))
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
    /// Flip this switch row.
    Toggle(Row),
    /// Move this number row by `.1` steps.
    Step(Row, i64),
    /// A text edit was confirmed with `Enter`.
    Commit(Row, String),
    /// Show this built-in palette's colors (see `apply_preset`).
    ApplyPreset(&'static str),
    /// Clear every override this popup has made.
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
    /// A page in the list down the left, by index into `Page::ALL`.
    Page(usize),
    Row(usize),
    /// Inside the popup's border but not on a page or an actionable row (a blank line, the hint
    /// line, the divider, or — while a picker is open — the gradient/hue area, which `hit`
    /// doesn't cover; see `picker_areas` for that).
    Inert,
    Outside,
}

/// The width of the page list, `" 1 Colors "` being its longest entry.
pub const SIDEBAR_WIDTH: u16 = 10;

/// Where the page list and the chosen page's rows are drawn inside the popup's outer rectangle.
/// Both start with a blank line; the last line of the popup is the hint, under both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupAreas {
    pub sidebar: Rect,
    /// The column between the two, drawn as a line.
    pub divider: Rect,
    pub content: Rect,
    pub hint: Rect,
}

/// The popup's inner layout, for `area` (from `overlay_view::panel_area`). `render_appearance`
/// draws into exactly these rectangles and `hit` tests against them.
pub fn areas(area: Rect) -> PopupAreas {
    let inner = area.inner(Margin::new(1, 1));
    // One cell of padding inside the frame, on both sides.
    let padded = inner.inner(Margin::new(1, 0));
    let body_height = padded.height.saturating_sub(1);
    let side = SIDEBAR_WIDTH.min(padded.width);
    let sidebar = Rect::new(padded.x, padded.y, side, body_height);
    let divider_x = sidebar.right().min(padded.right());
    let divider = Rect::new(
        divider_x,
        padded.y,
        u16::from(divider_x < padded.right()),
        body_height,
    );
    // A blank column after the divider, so the labels don't touch it.
    let content_x = divider.right().saturating_add(1).min(padded.right());
    let content = Rect::new(content_x, padded.y, padded.right() - content_x, body_height);
    let hint = Rect::new(
        padded.x,
        padded.bottom().saturating_sub(1),
        padded.width,
        u16::from(padded.height > 0),
    );
    PopupAreas {
        sidebar,
        divider,
        content,
        hint,
    }
}

/// What `pos` is over, given the popup's outer rectangle (from `overlay_view::panel_area`) and
/// how many rows the chosen page has — the same geometry `overlay_view::render_appearance` draws
/// with, so a click and its drawn row can never disagree.
pub fn hit(area: Rect, row_count: usize, pos: Position) -> Hit {
    if !area.inner(Margin::new(1, 1)).contains(pos) {
        return Hit::Outside;
    }
    let areas = areas(area);
    // The first line of both columns is blank.
    let line = |rect: Rect| usize::from(pos.y - rect.y).checked_sub(1);
    if areas.sidebar.contains(pos) {
        return match line(areas.sidebar) {
            Some(page) if page < Page::ALL.len() => Hit::Page(page),
            _ => Hit::Inert,
        };
    }
    if areas.content.contains(pos) {
        return match line(areas.content) {
            Some(row) if row < row_count => Hit::Row(row),
            _ => Hit::Inert,
        };
    }
    Hit::Inert
}

/// How much a keyboard nudge moves the picker's hue (degrees) or saturation/value (percentage
/// points) per press. The same step for all three keeps the key feel consistent even though the
/// ranges differ.
const PICKER_STEP: f64 = 5.0;

/// How far `H`/`L` move a number row, against `h`/`l`'s one step.
const BIG_STEP: i64 = 5;

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
    let content = areas(area).content;
    // One blank line above the rows, `row_count` rows, then a blank line before the square.
    let top = content
        .y
        .saturating_add(2 + row_count as u16)
        .min(content.bottom());
    let width = SV_BOX_WIDTH.min(content.width);
    let sv_box = Rect::new(
        content.x,
        top,
        width,
        SV_BOX_HEIGHT.min(content.bottom().saturating_sub(top)),
    );
    let hue_y = sv_box
        .y
        .saturating_add(sv_box.height)
        .saturating_add(1)
        .min(content.bottom());
    let hue_strip = Rect::new(
        content.x,
        hue_y,
        width,
        HUE_STRIP_HEIGHT.min(content.bottom().saturating_sub(hue_y)),
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

/// The saved themes' rename and delete flows — like `SaveState`, a small modal step over the row
/// list rather than a row of its own.
#[derive(Debug, Clone, PartialEq)]
enum ManageState {
    /// Typing a new name for saved theme `index`; starts as its current name.
    Rename { index: usize, buffer: String },
    /// Asking `y`/`n` before deleting saved theme `.0`.
    ConfirmDelete(usize),
}

pub struct AppearancePopup {
    cursor: usize,
    page: Page,
    /// `Some(_)` while the row under the cursor is being edited, in either mode.
    editing: Option<EditState>,
    /// `Some(_)` while the "Save current look" flow is asking for a choice or a name.
    save: Option<SaveState>,
    /// `Some(_)` while a saved theme is being renamed or is awaiting delete confirmation.
    manage: Option<ManageState>,
    /// The saved custom themes' names, in `local.toml` order. The popup never sees `Config` or
    /// `local.toml` (see `Outcome::WantSaveTheme`), so `main` hands it this list through
    /// `sync_saved` instead; it is what gives the Theme page its saved-theme rows.
    saved: Vec<String>,
}

/// A color row's in-progress edit: either typed as text (a name or `#rrggbb`/`#rgb` hex, exactly
/// what `appearance.toml` accepts) or adjusted as the gradient/hue picker — `Tab` swaps between
/// the two, converting the value across the swap so neither loses what the other set. A "Text"/
/// font row only ever uses `Text` — see `Row::kind`.
#[derive(Debug, Clone, PartialEq)]
enum EditState {
    Text(String),
    Picker(Hsv),
}

impl AppearancePopup {
    pub fn new() -> Self {
        Self {
            cursor: 0,
            page: Page::Theme,
            editing: None,
            save: None,
            manage: None,
            saved: Vec::new(),
        }
    }

    /// The rows `page` lists, built fresh — the Theme page's depend on the saved themes.
    fn rows_of(&self, page: Page) -> Vec<Row> {
        match page {
            Page::Theme => (0..THEME_NAMES.len())
                .map(Row::Preset)
                .chain((0..self.saved.len()).map(Row::SavedTheme))
                .chain([Row::SaveTheme, Row::Reset])
                .collect(),
            Page::Colors => COLOR_ROWS.to_vec(),
            Page::Shape => SHAPE_ROWS.to_vec(),
            Page::Text => (0..STYLE_ELEMENTS.len()).map(Row::StyleElement).collect(),
            Page::Font => vec![Row::Glyphs, Row::FontFamily, Row::FontSize],
        }
    }

    /// The rows the chosen page lists.
    pub fn rows(&self) -> Vec<Row> {
        self.rows_of(self.page)
    }

    /// How many rows the popup is sized for: the longest page's, so it keeps one size while the
    /// pages are switched, the way a settings window does.
    pub fn panel_rows(&self) -> usize {
        Page::ALL
            .iter()
            .map(|&page| self.rows_of(page).len())
            .max()
            .unwrap_or(0)
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn page(&self) -> Page {
        self.page
    }

    /// Shows `page` from its first row, dropping any edit or flow left open on the old one.
    pub fn set_page(&mut self, page: Page) {
        self.page = page;
        self.cursor = 0;
        self.editing = None;
        self.save = None;
        self.manage = None;
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

    /// What the "Save current look" flow should ask, from `Outcome::WantSaveTheme` — a no-op for
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

    /// `j`/`k`/the arrows move the cursor with wraparound; `Tab`/`Shift+Tab` or a page's number
    /// switch pages; `h`/`l`/the side arrows change the row under the cursor (`H`/`L` by five for
    /// a number), and `Enter` acts on it; `Esc` and `q` close the popup. While a row is being
    /// edited or a save/rename/delete flow is open, keys are routed to that instead — see
    /// `key_editing`/`key_saving`/`key_managing`.
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
        let row = rows[self.cursor];
        let saved_index = match row {
            Row::SavedTheme(index) => Some(index),
            _ => None,
        };
        let page_count = Page::ALL.len();
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.cursor = (self.cursor + 1) % rows.len();
                Outcome::Stay
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor = (self.cursor + rows.len() - 1) % rows.len();
                Outcome::Stay
            }
            KeyCode::Tab => {
                self.set_page(Page::ALL[(self.page.index() + 1) % page_count]);
                Outcome::Stay
            }
            KeyCode::BackTab => {
                self.set_page(Page::ALL[(self.page.index() + page_count - 1) % page_count]);
                Outcome::Stay
            }
            KeyCode::Char(c @ '1'..='9') => {
                let index = usize::from(c as u8 - b'1');
                if let Some(&page) = Page::ALL.get(index) {
                    self.set_page(page);
                }
                Outcome::Stay
            }
            KeyCode::Char('h' | 'l' | 'H' | 'L') | KeyCode::Left | KeyCode::Right => {
                let forward = matches!(code, KeyCode::Char('l' | 'L') | KeyCode::Right);
                let size = if matches!(code, KeyCode::Char('H' | 'L')) {
                    BIG_STEP
                } else {
                    1
                };
                match row.kind() {
                    Some(RowKind::Cycle(_)) => Outcome::Cycle(row),
                    Some(RowKind::Toggle) => Outcome::Toggle(row),
                    Some(RowKind::Step) => Outcome::Step(row, if forward { size } else { -size }),
                    _ => Outcome::Stay,
                }
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
            KeyCode::Esc | KeyCode::Char('q') => Outcome::Close,
            _ => Outcome::Stay,
        }
    }

    /// Text typing and the gradient/hue picker both live here; `Tab` converts the in-progress
    /// value across the two, only for a `RowKind::Color` row — a "Text"/font row has no picker to
    /// convert into. `Enter` commits (converting a picker's HSV to hex first); `Esc` cancels back
    /// to whatever the row held before editing began.
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

    /// A left click on page `index` in the list (from `hit`): the same as pressing its number.
    pub fn click_page(&mut self, index: usize) {
        if let Some(&page) = Page::ALL.get(index) {
            self.set_page(page);
        }
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
        let row = self.rows()[self.cursor];
        match row {
            Row::Preset(index) => THEME_NAMES
                .get(index)
                .map_or(Outcome::Stay, |&name| Outcome::ApplyPreset(name)),
            Row::Reset => Outcome::Reset,
            Row::SaveTheme => Outcome::WantSaveTheme,
            Row::SavedTheme(index) => Outcome::ApplyTheme(index),
            row => match row.kind().expect("every other row has a kind") {
                RowKind::Color | RowKind::Text => Outcome::WantEdit(row),
                RowKind::Cycle(_) => Outcome::Cycle(row),
                RowKind::Toggle => Outcome::Toggle(row),
                // A number changes with the side keys; Enter has nothing to confirm.
                RowKind::Step => Outcome::Stay,
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
    use theming::{Gap, Theme};

    use super::*;

    /// A popup showing `page`, cursor on its first row.
    fn on(page: Page) -> AppearancePopup {
        let mut popup = AppearancePopup::new();
        popup.set_page(page);
        popup
    }

    /// A popup on the Colors page with the cursor on Selection, its first row.
    fn on_selection() -> AppearancePopup {
        let popup = on(Page::Colors);
        assert_eq!(popup.rows()[popup.cursor()], Row::Selection);
        popup
    }

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
    fn tab_numbers_and_shift_tab_switch_pages_from_their_first_row() {
        let mut popup = AppearancePopup::new();
        assert_eq!(popup.page(), Page::Theme);
        popup.key(KeyCode::Char('j'));
        assert_eq!(popup.key(KeyCode::Tab), Outcome::Stay);
        assert_eq!(popup.page(), Page::Colors);
        assert_eq!(popup.cursor(), 0, "a new page starts at its first row");
        popup.key(KeyCode::Char('3'));
        assert_eq!(popup.page(), Page::Shape);
        popup.key(KeyCode::BackTab);
        assert_eq!(popup.page(), Page::Colors);
        popup.key(KeyCode::Char('1'));
        popup.key(KeyCode::BackTab);
        assert_eq!(popup.page(), Page::Font, "Shift+Tab wraps to the last page");
        popup.key(KeyCode::Char('9'));
        assert_eq!(
            popup.page(),
            Page::Font,
            "a number with no page does nothing"
        );
    }

    #[test]
    fn esc_and_q_close_from_any_page() {
        let mut popup = on(Page::Shape);
        assert_eq!(popup.key(KeyCode::Esc), Outcome::Close);
        assert_eq!(popup.key(KeyCode::Char('q')), Outcome::Close);
    }

    #[test]
    fn the_theme_page_lists_presets_then_saved_themes_then_save_and_reset() {
        let mut popup = AppearancePopup::new();
        popup.sync_saved(&themes(&["sunset"]));
        let rows = popup.rows();
        assert_eq!(rows.len(), THEME_NAMES.len() + 3);
        assert_eq!(rows[0], Row::Preset(0));
        assert_eq!(rows[THEME_NAMES.len()], Row::SavedTheme(0));
        assert_eq!(rows[rows.len() - 2..], [Row::SaveTheme, Row::Reset]);
    }

    #[test]
    fn enter_on_a_preset_applies_that_palette() {
        let mut popup = AppearancePopup::new();
        popup.key(KeyCode::Char('j'));
        assert_eq!(
            popup.key(KeyCode::Enter),
            Outcome::ApplyPreset(THEME_NAMES[1])
        );
        assert_eq!(
            popup.key(KeyCode::Char('l')),
            Outcome::Stay,
            "a preset is an action, not a value to change"
        );
    }

    #[test]
    fn a_color_row_wants_an_edit_and_a_cycle_row_cycles() {
        let mut popup = on_selection();
        assert_eq!(popup.key(KeyCode::Enter), Outcome::WantEdit(Row::Selection));
        assert_eq!(popup.editing_row(), None, "not editing until begin_edit");
        assert_eq!(
            popup.key(KeyCode::Char('l')),
            Outcome::Stay,
            "h/l are not a color row's edit key"
        );

        let mut popup = on(Page::Shape);
        assert_eq!(popup.rows()[0], Row::BorderType);
        assert_eq!(popup.key(KeyCode::Enter), Outcome::Cycle(Row::BorderType));
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
    fn number_rows_step_by_one_or_five_and_enter_leaves_them_alone() {
        let mut popup = on(Page::Shape);
        let at = SHAPE_ROWS.iter().position(|&r| r == Row::GapsIn).unwrap();
        for _ in 0..at {
            popup.key(KeyCode::Char('j'));
        }
        assert_eq!(popup.key(KeyCode::Char('l')), Outcome::Step(Row::GapsIn, 1));
        assert_eq!(popup.key(KeyCode::Left), Outcome::Step(Row::GapsIn, -1));
        assert_eq!(
            popup.key(KeyCode::Char('L')),
            Outcome::Step(Row::GapsIn, BIG_STEP)
        );
        assert_eq!(
            popup.key(KeyCode::Char('H')),
            Outcome::Step(Row::GapsIn, -BIG_STEP)
        );
        assert_eq!(popup.key(KeyCode::Enter), Outcome::Stay);
    }

    #[test]
    fn switch_rows_flip_with_enter_or_the_side_keys() {
        let mut popup = on(Page::Shape);
        let at = SHAPE_ROWS.iter().position(|&r| r == Row::Shadow).unwrap();
        assert_eq!(popup.click_row(at), Outcome::Toggle(Row::Shadow));
        assert_eq!(popup.key(KeyCode::Char('h')), Outcome::Toggle(Row::Shadow));
        assert_eq!(popup.key(KeyCode::Enter), Outcome::Toggle(Row::Shadow));
    }

    #[test]
    fn a_style_element_row_wants_an_edit_not_a_cycle() {
        let mut popup = on(Page::Text);
        assert_eq!(
            popup.key(KeyCode::Enter),
            Outcome::WantEdit(Row::StyleElement(0))
        );
    }

    #[test]
    fn editing_types_backspaces_commits_and_cancels() {
        let mut popup = on_selection();
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
    fn keys_typed_into_an_edit_never_switch_pages() {
        let mut popup = on(Page::Font);
        popup.key(KeyCode::Char('j')); // Font family
        popup.begin_edit("Iosevka".into());
        popup.key(KeyCode::Char('2'));
        assert_eq!(popup.page(), Page::Font);
        assert_eq!(popup.editing_buffer(), Some("Iosevka2"));
        assert_eq!(popup.key(KeyCode::Tab), Outcome::Stay);
        assert_eq!(popup.page(), Page::Font, "Tab in a text edit stays put");
        assert_eq!(
            popup.editing_buffer(),
            Some("Iosevka2"),
            "Tab never opens a picker for a non-color row"
        );
        assert_eq!(popup.editing_picker(), None);
    }

    #[test]
    fn moving_or_navigating_while_not_editing_never_touches_a_buffer() {
        let mut popup = AppearancePopup::new();
        assert_eq!(popup.key(KeyCode::Char('j')), Outcome::Stay);
        assert_eq!(popup.key(KeyCode::Tab), Outcome::Stay);
        assert_eq!(popup.editing_row(), None);
    }

    #[test]
    fn the_reset_and_save_rows_are_actions_not_values() {
        assert!(Row::Reset.kind().is_none());
        assert!(Row::SaveTheme.kind().is_none());
        let mut popup = AppearancePopup::new();
        popup.key(KeyCode::Char('k')); // wraps to the last row
        assert_eq!(popup.rows()[popup.cursor()], Row::Reset);
        assert_eq!(popup.key(KeyCode::Enter), Outcome::Reset);
        popup.key(KeyCode::Char('k'));
        assert_eq!(popup.rows()[popup.cursor()], Row::SaveTheme);
        assert_eq!(popup.key(KeyCode::Enter), Outcome::WantSaveTheme);
    }

    #[test]
    fn click_row_moves_the_cursor_and_acts_like_enter_there() {
        let mut popup = on(Page::Colors);
        assert_eq!(popup.click_row(2), Outcome::WantEdit(Row::Border));
        assert_eq!(popup.cursor(), 2);
        assert_eq!(
            popup.click_row(100),
            Outcome::Stay,
            "out of range is ignored"
        );
    }

    #[test]
    fn clicking_another_page_abandons_an_in_progress_edit() {
        let mut popup = on_selection();
        popup.begin_edit("#ff2bd6".into());
        popup.click_page(2);
        assert_eq!(popup.editing_row(), None);
        assert_eq!(popup.page(), Page::Shape);
        popup.click_page(99);
        assert_eq!(popup.page(), Page::Shape, "out of range is ignored");
    }

    #[test]
    fn tab_toggles_between_text_and_picker_preserving_the_color() {
        let mut popup = on_selection();
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
        assert_eq!(
            popup.page(),
            Page::Colors,
            "the edit's Tab never switches pages"
        );
    }

    #[test]
    fn an_unparsable_buffer_starts_the_picker_at_white_instead_of_losing_the_edit() {
        let mut popup = on_selection();
        popup.begin_edit("cyan".into()); // a name, not hex — can't be inverted to HSV
        popup.key(KeyCode::Tab);
        let hsv = popup.editing_picker().expect("still enters picker mode");
        assert_eq!((hsv.h, hsv.s, hsv.v), (0.0, 0.0, 100.0));
    }

    #[test]
    fn arrows_nudge_saturation_and_value_and_brackets_nudge_hue() {
        let mut popup = on_selection();
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
        let mut popup = on_selection();
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
        let mut popup = on_selection();
        popup.begin_edit("#ff0000".into());
        popup.key(KeyCode::Tab);
        assert_eq!(popup.key(KeyCode::Esc), Outcome::Stay);
        assert_eq!(popup.editing_row(), None);
        assert_eq!(popup.editing_picker(), None);
    }

    #[test]
    fn click_picker_sets_saturation_value_from_the_square_and_hue_from_the_strip() {
        let mut popup = on_selection();
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
    fn active_preset_compares_colors_only_and_prefers_the_local_pick() {
        let dracula = Theme::named("dracula");
        assert_eq!(active_preset(None, &dracula), Some("dracula"));
        assert_eq!(
            active_preset(Some("nord"), &dracula),
            Some("dracula"),
            "a stale local pick doesn't win over the colors actually shown"
        );
        let reshaped = Theme {
            border_type: "none".into(),
            separator: "arrow".into(),
            shape: Shape {
                gaps_in: Gap::clamped(3),
                shadow: true,
                ..Shape::default()
            },
            ..dracula.clone()
        };
        assert_eq!(
            active_preset(None, &reshaped),
            Some("dracula"),
            "the Shape page's settings never make a palette inactive"
        );
        let recolored = Theme {
            accent_fg: "#abcdef".into(),
            ..dracula
        };
        assert_eq!(active_preset(None, &recolored), None);
    }

    fn view_of<'a>(theme: &'a Theme, styles: &'a Styles, font: &'a Font) -> AppearanceView<'a> {
        AppearanceView {
            theme,
            glyphs: GlyphSet::Nerd,
            styles,
            font,
            active_preset: Some("dracula"),
            active_custom_theme: None,
        }
    }

    #[test]
    fn view_reads_the_right_field_per_row() {
        let mut theme = Theme::named("dracula");
        theme.shape.gaps_in = Gap::clamped(1);
        theme.shape.gaps_out = Gap::clamped(4);
        theme.shape.dim_inactive = true;
        let styles = Styles::default();
        let font = Font::default();
        let view = view_of(&theme, &styles, &font);
        let dracula = THEME_NAMES.iter().position(|&t| t == "dracula").unwrap();
        assert_eq!(view.value(Row::Preset(dracula)), "built-in, active");
        assert_eq!(view.value(Row::Preset(0)), "built-in");
        assert_eq!(view.value(Row::Selection), theme.selection_bg);
        assert_eq!(view.value(Row::Border), theme.border_fg);
        assert_eq!(view.value(Row::BorderFocused), theme.border_focused_fg);
        assert_eq!(view.value(Row::Directory), theme.dir_fg);
        assert_eq!(view.value(Row::StatusBar), theme.bar_bg);
        assert_eq!(view.value(Row::Danger), theme.danger_fg);
        assert_eq!(view.value(Row::BorderType), theme.border_type);
        assert_eq!(view.value(Row::Separator), theme.separator);
        assert_eq!(view.value(Row::TitleAlign), "left");
        assert_eq!(view.value(Row::GapsIn), "1 cell");
        assert_eq!(view.value(Row::GapsOut), "4 cells");
        assert_eq!(view.value(Row::Shadow), "off");
        assert_eq!(view.value(Row::DimInactive), "on");
        assert_eq!(view.value(Row::Glyphs), "nerd");
        assert_eq!(view.value(Row::SaveTheme), "new theme");
        assert_eq!(
            view.value(Row::StyleElement(0)),
            "bold",
            "dir is bold by default"
        );
        assert_eq!(view.value(Row::FontFamily), font.family);
        assert_eq!(view.value(Row::FontSize), format!("{:.1}", font.size));
        assert_eq!(view.value(Row::SavedTheme(0)), "");

        let named = AppearanceView {
            active_custom_theme: Some("sunset"),
            ..view
        };
        assert_eq!(named.value(Row::SaveTheme), "updates 'sunset'");
    }

    #[test]
    fn every_shape_row_cycles_through_values_its_view_shows() {
        let theme = Theme::default();
        let styles = Styles::default();
        let font = Font::default();
        let view = view_of(&theme, &styles, &font);
        for row in SHAPE_ROWS {
            if let Some(RowKind::Cycle(options)) = row.kind() {
                assert!(
                    options.contains(&view.value(row).as_str()),
                    "{row:?} shows {:?}, not one of its options",
                    view.value(row)
                );
            }
        }
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
        commit(&mut overrides, Row::TitleAlign, "center".into());

        // A non-`RawTheme` row is a no-op here.
        commit(&mut overrides, Row::FontFamily, "Iosevka".into());
        let mut expected = RawTheme {
            danger_fg: Some("#ff0000".into()),
            separator: Some("arrow".into()),
            ..RawTheme::default()
        };
        expected.shape.title_align = Some("center".into());
        assert_eq!(overrides, expected);
    }

    #[test]
    fn step_and_toggle_start_from_the_live_shape_and_stay_in_range() {
        let live = Shape {
            gaps_in: Gap::clamped(3),
            shadow: true,
            ..Shape::default()
        };
        let mut overrides = RawTheme::default();
        step(&mut overrides, Row::GapsIn, &live, BIG_STEP);
        assert_eq!(
            overrides.shape.gaps_in,
            Some(i64::from(theming::shape::MAX_GAPS_IN)),
            "stops at the top of the range"
        );
        step(&mut overrides, Row::GapsOut, &live, -1);
        assert_eq!(overrides.shape.gaps_out, Some(0), "stops at zero");
        toggle(&mut overrides, Row::Shadow, &live);
        assert_eq!(overrides.shape.shadow, Some(false));
        toggle(&mut overrides, Row::DimInactive, &live);
        assert_eq!(overrides.shape.dim_inactive, Some(true));

        let before = overrides.clone();
        step(&mut overrides, Row::Shadow, &live, 1);
        toggle(&mut overrides, Row::GapsIn, &live);
        assert_eq!(overrides, before, "each only acts on its own kind of row");
    }

    #[test]
    fn a_preset_replaces_the_colors_but_keeps_the_shape() {
        let mut overrides = RawTheme {
            name: Some("nord".into()),
            accent_fg: Some("#123456".into()),
            border_type: Some("thick".into()),
            separator: Some("arrow".into()),
            ..RawTheme::default()
        };
        overrides.shape.gaps_in = Some(2);
        let applied = apply_preset(&overrides, "dracula");
        assert_eq!(applied.name.as_deref(), Some("dracula"));
        assert_eq!(applied.accent_fg, None, "a hand-picked color goes");
        assert_eq!(applied.border_type.as_deref(), Some("thick"));
        assert_eq!(applied.separator.as_deref(), Some("arrow"));
        assert_eq!(applied.shape, overrides.shape);
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
    fn hit_finds_pages_on_the_left_and_rows_on_the_right() {
        let area = Rect::new(10, 5, 60, 12);
        let a = areas(area);
        assert_eq!(hit(area, 5, Position::new(5, 5)), Hit::Outside);
        assert_eq!(
            hit(area, 5, Position::new(area.x, area.y + 3)),
            Hit::Outside,
            "the frame"
        );
        assert_eq!(
            hit(area, 5, Position::new(a.sidebar.x, a.sidebar.y)),
            Hit::Inert
        );
        assert_eq!(
            hit(area, 5, Position::new(a.sidebar.x, a.sidebar.y + 1)),
            Hit::Page(0)
        );
        assert_eq!(
            hit(area, 5, Position::new(a.sidebar.x + 3, a.sidebar.y + 5)),
            Hit::Page(4)
        );
        assert_eq!(
            hit(area, 5, Position::new(a.sidebar.x, a.sidebar.y + 6)),
            Hit::Inert
        );
        assert_eq!(
            hit(area, 5, Position::new(a.content.x, a.content.y + 1)),
            Hit::Row(0)
        );
        assert_eq!(
            hit(area, 5, Position::new(a.content.x + 9, a.content.y + 5)),
            Hit::Row(4)
        );
        assert_eq!(
            hit(area, 5, Position::new(a.content.x, a.content.y + 6)),
            Hit::Inert
        );
        assert_eq!(
            hit(area, 5, Position::new(a.divider.x, a.divider.y + 2)),
            Hit::Inert
        );
        assert_eq!(
            hit(area, 5, Position::new(a.hint.x + 20, a.hint.y)),
            Hit::Inert
        );
    }

    #[test]
    fn picker_areas_sit_below_the_rows_and_shrink_to_fit_a_small_frame() {
        let area = Rect::new(0, 0, 76, 7 + PICKER_EXTRA_LINES + 5);
        let areas_ = picker_areas(area, 7);
        assert!(areas_.sv_box.height > 0 && areas_.hue_strip.height > 0);
        assert!(areas_.hue_strip.y > areas_.sv_box.bottom());
        assert!(areas(area).content.contains(areas_.sv_box.as_position()));

        let tiny = picker_areas(Rect::new(0, 0, 20, 6), 7);
        assert_eq!(tiny.sv_box.height, 0, "no room left for the square");
    }

    #[test]
    fn the_panel_is_sized_for_the_longest_page_so_switching_never_resizes_it() {
        let mut popup = AppearancePopup::new();
        let rows = popup.panel_rows();
        assert_eq!(rows, COLOR_ROWS.len().max(STYLE_ELEMENTS.len()));
        for page in Page::ALL {
            popup.set_page(page);
            assert!(popup.rows().len() <= rows);
            assert_eq!(popup.panel_rows(), rows);
        }
        let many: Vec<String> = (0..30).map(|i| format!("t{i}")).collect();
        let names: Vec<&str> = many.iter().map(String::as_str).collect();
        popup.sync_saved(&themes(&names));
        assert_eq!(popup.panel_rows(), THEME_NAMES.len() + 30 + 2);
    }

    proptest::proptest! {
        /// Whatever keys the popup sees, the cursor always names one of the shown page's rows.
        #[test]
        fn the_cursor_always_stays_in_bounds(moves in proptest::collection::vec(0u8..6, 0..200)) {
            let mut popup = AppearancePopup::new();
            popup.sync_saved(&themes(&["a", "b"]));
            for m in moves {
                let code = match m {
                    0 => KeyCode::Char('j'),
                    1 => KeyCode::Char('k'),
                    2 => KeyCode::Tab,
                    3 => KeyCode::BackTab,
                    4 => KeyCode::Char('4'),
                    _ => KeyCode::Esc,
                };
                popup.key(code);
                proptest::prop_assert!(popup.cursor() < popup.rows().len());
            }
        }

        /// However `hit` is queried, a page or row it returns always exists.
        #[test]
        fn hit_never_names_a_missing_page_or_row(
            x in 0u16..100, y in 0u16..40, w in 0u16..90, h in 0u16..30, row_count in 1usize..25,
        ) {
            let area = Rect::new(10, 5, w, h);
            match hit(area, row_count, Position::new(x, y)) {
                Hit::Row(row) => proptest::prop_assert!(row < row_count),
                Hit::Page(page) => proptest::prop_assert!(page < Page::ALL.len()),
                Hit::Inert | Hit::Outside => {}
            }
        }

        /// The page list, divider, rows and hint never overlap and never leave the frame.
        #[test]
        fn the_popup_areas_tile_its_inside(w in 0u16..120, h in 0u16..50) {
            let area = Rect::new(3, 2, w, h);
            let inner = area.inner(Margin::new(1, 1));
            let a = areas(area);
            let parts = [a.sidebar, a.divider, a.content, a.hint];
            for part in parts {
                proptest::prop_assert!(part.area() == 0 || inner.contains(part.as_position()));
                proptest::prop_assert!(part.area() == 0 || (part.right() <= inner.right() && part.bottom() <= inner.bottom()));
            }
            for (i, p) in parts.iter().enumerate() {
                for q in &parts[i + 1..] {
                    proptest::prop_assert!(p.area() == 0 || q.area() == 0 || !p.intersects(*q));
                }
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

    /// A popup on the Theme page with `names` synced, cursor on the first saved theme.
    fn saved_themes_popup(names: &[&str]) -> AppearancePopup {
        let mut popup = AppearancePopup::new();
        popup.sync_saved(&themes(names));
        popup.cursor = THEME_NAMES.len();
        popup
    }

    #[test]
    fn saved_themes_follow_the_presets_one_row_each() {
        let mut popup = AppearancePopup::new();
        assert!(!popup.rows().iter().any(|r| matches!(r, Row::SavedTheme(_))));
        popup.sync_saved(&themes(&["sunset", "midnight"]));
        let first = THEME_NAMES.len();
        assert_eq!(
            popup.rows()[first..first + 2],
            [Row::SavedTheme(0), Row::SavedTheme(1)]
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
            popup.click_row(THEME_NAMES.len()),
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
        popup.cursor = 0; // a preset
        assert_eq!(popup.key(KeyCode::Char('r')), Outcome::Stay);
        assert_eq!(popup.key(KeyCode::Char('d')), Outcome::Stay);
        assert_eq!(popup.manage_view(), None);
    }

    #[test]
    fn sync_saved_pulls_the_cursor_back_and_drops_a_flow_on_a_vanished_theme() {
        let mut popup = saved_themes_popup(&["sunset", "midnight"]);
        popup.cursor = THEME_NAMES.len() + 1; // midnight
        popup.key(KeyCode::Char('d'));
        popup.sync_saved(&themes(&["sunset"]));
        assert_eq!(popup.manage_view(), None, "its target no longer exists");
        popup.cursor = popup.rows().len() - 1; // Reset, the last row
        popup.sync_saved(&themes(&[]));
        assert_eq!(
            popup.rows()[popup.cursor()],
            Row::Reset,
            "still on a row that exists"
        );
    }
}
