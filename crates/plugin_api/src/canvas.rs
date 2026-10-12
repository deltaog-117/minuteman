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

//! What a plugin that draws a whole-body view exchanges with the host: a canvas. The atlas and the
//! wiring diagram are the first two. The interface tells the plugin when its view opens, how many
//! cells it has, and what the user did in it (a key it claimed, a click, the wheel); the plugin
//! answers with a canvas (`set_canvas`): rectangles, frames, text and orthogonal paths on a grid
//! of cells, each painted by meaning (a tone, a hue number, a mark) and never by color. The
//! interface owns the palette, the border style, the glyph set and the animation clock, so the
//! plugin cannot draw anything the user's theme would not, and a view that moves out of the core
//! looks the same as it did in it. What a view asks of the browser (go to a folder, say something)
//! is a `canvas_note`.
//!
//! Keys cannot be offered to a plugin and passed back, because the answer would arrive a frame
//! late. Instead every canvas names the keys it `claims` right now; the interface forwards those
//! and keeps the rest (`V`, `q`, `/`, `:`) for the browser, so what the atlas does with `Esc` at
//! its top level (nothing, so the browser closes the view) is a claim list that omits it.
//!
//! A plugin may be wrong or hostile and its text ends up in the user's terminal, so
//! [`SetCanvas::checked`] cleans and bounds a canvas before the host passes it on, and refuses one
//! whose parts lie outside it.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::view::{Tone, is_unsafe};

/// The notification that gives a view its canvas: `set_canvas`.
pub const METHOD_SET_CANVAS: &str = "set_canvas";
/// The notification that says what a view asks of the browser: `canvas_note`.
pub const METHOD_CANVAS_NOTE: &str = "canvas_note";
/// The event name a canvas plugin gets from the interface.
pub const EVENT_CANVAS: &str = "canvas";

/// What a plugin lists under `provides` to draw the atlas.
pub const PROVIDES_CANVAS_ATLAS: &str = "canvas-atlas";
/// What a plugin lists under `provides` to draw the wiring diagram.
pub const PROVIDES_CANVAS_WIRING: &str = "canvas-wiring";

/// Most cells a canvas is wide or tall; a terminal larger than this is not a thing to lay out.
pub const MAX_CANVAS_SIDE: u16 = 1000;
/// Most parts a canvas holds.
pub const MAX_CANVAS_ITEMS: usize = 4000;
/// Most characters of any one text, of a note, or of a claimed key's name.
pub const MAX_CANVAS_TEXT_CHARS: usize = 256;
/// Most characters of text in all the parts of a canvas.
pub const MAX_CANVAS_CHARS: usize = 64 * 1024;
/// Most corners (and ends) of a path.
pub const MAX_PATH_POINTS: usize = 64;
/// Most keys a canvas claims.
pub const MAX_CLAIMS: usize = 64;
/// How many hues a canvas may name; the interface derives them from the theme, and wraps none.
pub const HUES: u8 = 16;

/// A view a plugin can draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// The zoomable map of the storage.
    Atlas,
    /// The diagram of how the storage is wired.
    Wiring,
}

impl Surface {
    /// What a plugin lists under `provides` to draw this view.
    pub fn provides(self) -> &'static str {
        match self {
            Self::Atlas => PROVIDES_CANVAS_ATLAS,
            Self::Wiring => PROVIDES_CANVAS_WIRING,
        }
    }

    /// The surface `provides` names, or `None` for a word that is not one of the above.
    pub fn from_provides(word: &str) -> Option<Self> {
        [Self::Atlas, Self::Wiring]
            .into_iter()
            .find(|surface| surface.provides() == word)
    }
}

/// A place on the canvas: the cell of its top left corner and its size in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cells {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Cells {
    /// Whether the rectangle has area and lies inside a canvas of `width` by `height`.
    fn fits(self, width: u16, height: u16) -> bool {
        self.w > 0
            && self.h > 0
            && u32::from(self.x) + u32::from(self.w) <= u32::from(width)
            && u32::from(self.y) + u32::from(self.h) <= u32::from(height)
    }
}

/// What a part stands for among its neighbours, drawn however the theme draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    #[default]
    None,
    /// The part the cursor is on.
    Chosen,
    /// The part holding the folder the browser is in ("you are here").
    Here,
}

/// How a part looks, in meaning: the interface turns it into colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Paint {
    #[serde(default)]
    pub tone: Tone,
    /// A categorical color, below [`HUES`], for telling kinds apart (a folder from a file, a disk
    /// from a partition). `None` takes the tone's own color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hue: Option<u8>,
    #[serde(default)]
    pub mark: Mark,
}

/// One part of a canvas. Parts are drawn in order, so a later one covers an earlier one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "part", rename_all = "snake_case")]
pub enum Item {
    /// A filled rectangle. `level` is how much of it, from the bottom, is the paint's own color
    /// rather than dimmed (a volume's block, filled by how full the volume is): 0 to 100.
    Block {
        at: Cells,
        paint: Paint,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level: Option<u8>,
    },
    /// A frame, drawn in the theme's border style, with an optional title in its top edge.
    Frame {
        at: Cells,
        paint: Paint,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    /// One line of text starting at `x`, cut to `width` cells.
    Text {
        x: u16,
        y: u16,
        width: u16,
        text: String,
        paint: Paint,
        #[serde(default)]
        bold: bool,
    },
    /// Straight runs joined at right angles through `points`. `flow` asks for a mark that travels
    /// along it, which the interface animates (data moving between two layers).
    Path {
        points: Vec<(u16, u16)>,
        paint: Paint,
        #[serde(default)]
        flow: bool,
    },
}

/// Everything one view shows, laid out for `width` by `height` cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Canvas {
    /// The size it was laid out for. The interface draws it at the top left of the view's body
    /// and shows nothing where the view is smaller, until the plugin lays out again for the size
    /// it was sent (`resize`).
    pub width: u16,
    pub height: u16,
    pub items: Vec<Item>,
    /// The keys, spelled as the config spells them, that the plugin wants while this canvas
    /// shows. Any other key is the browser's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claims: Vec<String>,
    /// A line for the status bar's hint area while the view shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// The parameters of `set_canvas`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetCanvas {
    pub surface: Surface,
    /// What now fills the view; `null` or absent says the view has nothing yet (it shows as
    /// loading).
    #[serde(default)]
    pub canvas: Option<Canvas>,
}

/// Why a canvas was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanvasError {
    /// Zero cells wide or tall, or more than [`MAX_CANVAS_SIDE`].
    BadSize,
    TooManyItems,
    TooMuchText,
    /// A part with no area, or lying outside the canvas (the index of the part).
    OutsideCanvas(usize),
    /// A path of fewer than two points, more than [`MAX_PATH_POINTS`], or with a slanted run.
    BadPath(usize),
    /// A hue number at or above [`HUES`], or a level above 100.
    BadPaint(usize),
    TooManyClaims,
}

impl fmt::Display for CanvasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSize => write!(f, "a canvas is 1 to {MAX_CANVAS_SIDE} cells on each side"),
            Self::TooManyItems => write!(f, "a canvas holds at most {MAX_CANVAS_ITEMS} parts"),
            Self::TooMuchText => write!(f, "a canvas holds at most {MAX_CANVAS_CHARS} characters"),
            Self::OutsideCanvas(i) => write!(f, "part {i} has no area or lies outside the canvas"),
            Self::BadPath(i) => write!(
                f,
                "part {i} is not a path of 2 to {MAX_PATH_POINTS} points joined at right angles"
            ),
            Self::BadPaint(i) => write!(f, "part {i} names a hue or a level that does not exist"),
            Self::TooManyClaims => write!(f, "a canvas claims at most {MAX_CLAIMS} keys"),
        }
    }
}

impl std::error::Error for CanvasError {}

/// Which button a click was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Left,
    Middle,
    Right,
}

/// The modifier keys held during a key or a click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Mods {
    #[serde(default, skip_serializing_if = "is_false")]
    pub ctrl: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub alt: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub shift: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// What the interface tells a canvas plugin, in the `canvas` field of a `canvas` event. Positions
/// are cells of the canvas, counted from its top left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "input", rename_all = "snake_case")]
pub enum CanvasInput {
    /// The view opened with this many cells (and again, with the same, if it is reopened). The
    /// plugin should look at the machine only from now until `close`. `here` is the folder the
    /// browser is in, for a view that marks where the user is.
    Open {
        width: u16,
        height: u16,
        #[serde(default)]
        here: PathBuf,
    },
    /// The window changed: lay out again for this size.
    Resize { width: u16, height: u16 },
    /// The view closed.
    Close,
    /// A key the canvas claimed.
    Key {
        key: String,
        #[serde(default)]
        mods: Mods,
    },
    Click {
        x: u16,
        y: u16,
        button: Button,
        #[serde(default)]
        double: bool,
        #[serde(default)]
        mods: Mods,
    },
    /// The wheel turned over a cell: `down` is towards the user.
    Wheel { x: u16, y: u16, down: bool },
}

/// The event the interface sends: which view, and what happened in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasEvent {
    pub surface: Surface,
    #[serde(flatten)]
    pub input: CanvasInput,
}

/// The parameters of `canvas_note`: what a view asks of the browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CanvasNote {
    /// A message for the status line.
    Status { text: String },
    /// Show `path` in the browser, saying `text` (when there is any) once there.
    Goto {
        path: PathBuf,
        #[serde(default)]
        text: String,
    },
    /// Copy the files the browser carries into `into`.
    Send { into: PathBuf },
}

fn cleaned(text: &mut String) {
    text.retain(|c| !is_unsafe(c));
    if let Some((cut, _)) = text.char_indices().nth(MAX_CANVAS_TEXT_CHARS) {
        text.truncate(cut);
    }
}

fn path_is_unsafe(path: &std::path::Path) -> bool {
    path.to_string_lossy().chars().any(is_unsafe)
}

fn paint_fits(paint: Paint) -> bool {
    paint.hue.is_none_or(|hue| hue < HUES)
}

impl Canvas {
    /// This canvas with every text cleaned of control and direction-changing characters and cut
    /// short, or why it cannot be drawn. Cleaning goes first, so the limits count what would be
    /// drawn.
    ///
    /// # Errors
    ///
    /// A size outside 1 to [`MAX_CANVAS_SIDE`]; more parts, characters or claims than the
    /// limits; a part with no area or outside the canvas; a path that is too short, too long or
    /// slanted; a hue or a level that does not exist.
    pub fn checked(mut self) -> Result<Self, CanvasError> {
        let (width, height) = (self.width, self.height);
        if width == 0 || height == 0 || width > MAX_CANVAS_SIDE || height > MAX_CANVAS_SIDE {
            return Err(CanvasError::BadSize);
        }
        if self.items.len() > MAX_CANVAS_ITEMS {
            return Err(CanvasError::TooManyItems);
        }
        if self.claims.len() > MAX_CLAIMS {
            return Err(CanvasError::TooManyClaims);
        }
        let mut chars = 0usize;
        for (i, item) in self.items.iter_mut().enumerate() {
            match item {
                Item::Block { at, paint, level } => {
                    if !at.fits(width, height) {
                        return Err(CanvasError::OutsideCanvas(i));
                    }
                    if !paint_fits(*paint) || level.is_some_and(|level| level > 100) {
                        return Err(CanvasError::BadPaint(i));
                    }
                }
                Item::Frame { at, paint, title } => {
                    if !at.fits(width, height) {
                        return Err(CanvasError::OutsideCanvas(i));
                    }
                    if !paint_fits(*paint) {
                        return Err(CanvasError::BadPaint(i));
                    }
                    if let Some(title) = title {
                        cleaned(title);
                        chars += title.chars().count();
                    }
                }
                Item::Text {
                    x,
                    y,
                    width: span,
                    text,
                    paint,
                    ..
                } => {
                    let at = Cells {
                        x: *x,
                        y: *y,
                        w: *span,
                        h: 1,
                    };
                    if !at.fits(width, height) {
                        return Err(CanvasError::OutsideCanvas(i));
                    }
                    if !paint_fits(*paint) {
                        return Err(CanvasError::BadPaint(i));
                    }
                    cleaned(text);
                    chars += text.chars().count();
                }
                Item::Path { points, paint, .. } => {
                    let straight = points
                        .windows(2)
                        .all(|pair| pair[0].0 == pair[1].0 || pair[0].1 == pair[1].1);
                    if points.len() < 2 || points.len() > MAX_PATH_POINTS || !straight {
                        return Err(CanvasError::BadPath(i));
                    }
                    if points.iter().any(|&(x, y)| x >= width || y >= height) {
                        return Err(CanvasError::OutsideCanvas(i));
                    }
                    if !paint_fits(*paint) {
                        return Err(CanvasError::BadPaint(i));
                    }
                }
            }
        }
        for claim in &mut self.claims {
            cleaned(claim);
        }
        if let Some(hint) = &mut self.hint {
            cleaned(hint);
            chars += hint.chars().count();
        }
        if chars > MAX_CANVAS_CHARS {
            return Err(CanvasError::TooMuchText);
        }
        Ok(self)
    }
}

impl SetCanvas {
    /// This request with its canvas, if any, checked (see [`Canvas::checked`]).
    ///
    /// # Errors
    ///
    /// Whatever [`Canvas::checked`] refuses.
    pub fn checked(self) -> Result<Self, CanvasError> {
        let Self { surface, canvas } = self;
        let canvas = canvas.map(Canvas::checked).transpose()?;
        Ok(Self { surface, canvas })
    }
}

impl CanvasNote {
    /// This note with its text cleaned and cut short, or `None` for a folder whose path could act
    /// on the terminal when shown.
    pub fn checked(mut self) -> Option<Self> {
        match &mut self {
            Self::Status { text } => cleaned(text),
            Self::Goto { path, text } => {
                if path_is_unsafe(path) {
                    return None;
                }
                cleaned(text);
            }
            Self::Send { into } => {
                if path_is_unsafe(into) {
                    return None;
                }
            }
        }
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;

    fn block(x: u16, y: u16, w: u16, h: u16) -> Item {
        Item::Block {
            at: Cells { x, y, w, h },
            paint: Paint::default(),
            level: None,
        }
    }

    fn canvas(width: u16, height: u16, items: Vec<Item>) -> Canvas {
        Canvas {
            width,
            height,
            items,
            claims: Vec::new(),
            hint: None,
        }
    }

    #[test]
    fn a_surface_is_found_again_by_the_word_that_provides_it() {
        for surface in [Surface::Atlas, Surface::Wiring] {
            assert_eq!(Surface::from_provides(surface.provides()), Some(surface));
        }
        assert_eq!(Surface::from_provides("devices"), None);
    }

    #[test]
    fn the_wire_shape_is_the_documented_one() {
        let set = SetCanvas {
            surface: Surface::Atlas,
            canvas: Some(Canvas {
                claims: vec!["enter".into()],
                ..canvas(
                    10,
                    4,
                    vec![Item::Text {
                        x: 1,
                        y: 0,
                        width: 8,
                        text: "usr".into(),
                        paint: Paint {
                            tone: Tone::Accent,
                            hue: Some(3),
                            mark: Mark::Chosen,
                        },
                        bold: true,
                    }],
                )
            }),
        };
        assert_eq!(
            serde_json::to_value(&set).unwrap(),
            json!({
                "surface": "atlas",
                "canvas": {
                    "width": 10,
                    "height": 4,
                    "items": [{
                        "part": "text", "x": 1, "y": 0, "width": 8, "text": "usr",
                        "paint": {"tone": "accent", "hue": 3, "mark": "chosen"},
                        "bold": true
                    }],
                    "claims": ["enter"]
                }
            })
        );
    }

    #[test]
    fn an_input_has_its_documented_wire_shape_and_defaults() {
        let event = CanvasEvent {
            surface: Surface::Wiring,
            input: CanvasInput::Click {
                x: 3,
                y: 2,
                button: Button::Left,
                double: false,
                mods: Mods {
                    ctrl: true,
                    ..Mods::default()
                },
            },
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(
            value,
            json!({
                "surface": "wiring", "input": "click", "x": 3, "y": 2, "button": "left",
                "double": false, "mods": {"ctrl": true}
            })
        );
        assert_eq!(serde_json::from_value::<CanvasEvent>(value).unwrap(), event);
        let bare: CanvasEvent =
            serde_json::from_str(r#"{"surface":"atlas","input":"key","key":"j"}"#).unwrap();
        assert_eq!(
            bare.input,
            CanvasInput::Key {
                key: "j".into(),
                mods: Mods::default()
            }
        );
    }

    #[test]
    fn a_missing_canvas_is_allowed_and_means_not_ready() {
        let set: SetCanvas = serde_json::from_str(r#"{"surface":"atlas"}"#).unwrap();
        assert_eq!(set.checked().unwrap().canvas, None);
    }

    #[test]
    fn a_part_that_leaves_the_canvas_or_has_no_area_is_refused() {
        for item in [block(8, 0, 3, 1), block(0, 3, 1, 2), block(0, 0, 0, 1)] {
            assert_eq!(
                canvas(10, 4, vec![block(0, 0, 1, 1), item]).checked(),
                Err(CanvasError::OutsideCanvas(1))
            );
        }
        assert!(canvas(10, 4, vec![block(0, 0, 10, 4)]).checked().is_ok());
    }

    #[test]
    fn a_slanted_short_or_outside_path_is_refused() {
        let path = |points: Vec<(u16, u16)>| Item::Path {
            points,
            paint: Paint::default(),
            flow: false,
        };
        assert_eq!(
            canvas(10, 4, vec![path(vec![(0, 0)])]).checked(),
            Err(CanvasError::BadPath(0))
        );
        assert_eq!(
            canvas(10, 4, vec![path(vec![(0, 0), (3, 2)])]).checked(),
            Err(CanvasError::BadPath(0))
        );
        assert_eq!(
            canvas(10, 4, vec![path(vec![(0, 0), (10, 0)])]).checked(),
            Err(CanvasError::OutsideCanvas(0))
        );
        assert!(
            canvas(10, 4, vec![path(vec![(0, 0), (5, 0), (5, 3)])])
                .checked()
                .is_ok()
        );
    }

    #[test]
    fn a_hue_or_level_that_does_not_exist_is_refused() {
        let hued = Item::Block {
            at: Cells {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            paint: Paint {
                hue: Some(HUES),
                ..Paint::default()
            },
            level: None,
        };
        assert_eq!(
            canvas(2, 2, vec![hued]).checked(),
            Err(CanvasError::BadPaint(0))
        );
        let over = Item::Block {
            at: Cells {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            paint: Paint::default(),
            level: Some(101),
        };
        assert_eq!(
            canvas(2, 2, vec![over]).checked(),
            Err(CanvasError::BadPaint(0))
        );
    }

    #[test]
    fn text_that_could_act_on_the_terminal_is_cleaned() {
        let text = Item::Text {
            x: 0,
            y: 0,
            width: 8,
            text: "a\u{1b}]0;x\u{7}b\u{202e}c".into(),
            paint: Paint::default(),
            bold: false,
        };
        let cleaned = canvas(8, 1, vec![text]).checked().unwrap();
        let Item::Text { text, .. } = &cleaned.items[0] else {
            panic!("a text stays a text");
        };
        assert_eq!(text, "a]0;xbc");
    }

    #[test]
    fn the_limits_on_size_parts_text_and_claims_hold() {
        assert_eq!(canvas(0, 4, vec![]).checked(), Err(CanvasError::BadSize));
        assert_eq!(
            canvas(MAX_CANVAS_SIDE + 1, 4, vec![]).checked(),
            Err(CanvasError::BadSize)
        );
        assert_eq!(
            canvas(4, 4, vec![block(0, 0, 1, 1); MAX_CANVAS_ITEMS + 1]).checked(),
            Err(CanvasError::TooManyItems)
        );
        let many = Canvas {
            claims: vec!["j".into(); MAX_CLAIMS + 1],
            ..canvas(4, 4, vec![])
        };
        assert_eq!(many.checked(), Err(CanvasError::TooManyClaims));
        let words = |count: usize| {
            (0..count)
                .map(|_| Item::Text {
                    x: 0,
                    y: 0,
                    width: 1,
                    text: "x".repeat(MAX_CANVAS_TEXT_CHARS),
                    paint: Paint::default(),
                    bold: false,
                })
                .collect::<Vec<_>>()
        };
        let enough = MAX_CANVAS_CHARS / MAX_CANVAS_TEXT_CHARS;
        assert!(canvas(4, 4, words(enough)).checked().is_ok());
        assert_eq!(
            canvas(4, 4, words(enough + 1)).checked(),
            Err(CanvasError::TooMuchText)
        );
    }

    #[test]
    fn a_note_is_cleaned_and_a_path_that_could_act_on_the_terminal_is_dropped() {
        let status = CanvasNote::Status {
            text: "ok\u{1b}[31m".into(),
        };
        assert_eq!(
            status.checked(),
            Some(CanvasNote::Status {
                text: "ok[31m".into()
            })
        );
        let goto = CanvasNote::Goto {
            path: PathBuf::from("/a\u{1b}b"),
            text: String::new(),
        };
        assert_eq!(goto.checked(), None);
        let send = CanvasNote::Send {
            into: PathBuf::from("/a\u{202e}b"),
        };
        assert_eq!(send.checked(), None);
        let fine = CanvasNote::Send {
            into: PathBuf::from("/mnt/usb"),
        };
        assert_eq!(fine.clone().checked(), Some(fine));
    }

    fn arb_cells() -> impl Strategy<Value = Cells> {
        (0u16..40, 0u16..40, 0u16..40, 0u16..40).prop_map(|(x, y, w, h)| Cells { x, y, w, h })
    }

    proptest! {
        // Whatever a plugin sends, what survives the check lies wholly inside the canvas, so the
        // interface can draw it without clamping or bounds arithmetic of its own.
        #[test]
        fn a_checked_canvas_never_has_a_part_outside_it(
            width in 0u16..50,
            height in 0u16..50,
            parts in proptest::collection::vec(arb_cells(), 0..30),
        ) {
            let items = parts
                .into_iter()
                .map(|at| Item::Block { at, paint: Paint::default(), level: None })
                .collect();
            if let Ok(checked) = canvas(width, height, items).checked() {
                for item in &checked.items {
                    let Item::Block { at, .. } = item else { unreachable!() };
                    prop_assert!(at.w > 0 && at.h > 0);
                    prop_assert!(u32::from(at.x) + u32::from(at.w) <= u32::from(width));
                    prop_assert!(u32::from(at.y) + u32::from(at.h) <= u32::from(height));
                }
            }
        }

        // Whatever bytes a text holds, none of what could act on the terminal is left.
        #[test]
        fn a_checked_text_holds_nothing_unsafe(text in ".{0,400}") {
            let item = Item::Text {
                x: 0, y: 0, width: 1, text, paint: Paint::default(), bold: false,
            };
            if let Ok(checked) = canvas(2, 2, vec![item]).checked() {
                let Item::Text { text, .. } = &checked.items[0] else { unreachable!() };
                prop_assert!(!text.chars().any(is_unsafe));
                prop_assert!(text.chars().count() <= MAX_CANVAS_TEXT_CHARS);
            }
        }
    }
}
