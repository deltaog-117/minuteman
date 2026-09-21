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

//! The right-click menu: what it lists for what was clicked, where it sits on screen, and how a
//! pointer or a key moves through it. Like `browser_mouse`, everything here is pure — no
//! terminal, no event loop, no drawing — so the geometry that `overlay_view` draws from and the
//! mouse handler hit-tests against is one function, and the two cannot disagree about which row
//! a click landed on.
//!
//! A menu is a list of `Entry`s. A submenu (only "Open with" has one) opens beside its parent
//! row, and only one is ever open. What a chosen item *does* is a `MenuCommand`; the menu never
//! performs it, so it can be built and tested without an `App` or a filesystem.

use ratatui::layout::{Margin, Position, Rect};

use crate::hud::text_width;

/// Everything a menu item can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    Open,
    /// The `n`th entry of the "Open with" list the menu was built from.
    OpenWith(usize),
    Cut,
    Copy,
    /// Paste into the browsed directory.
    Paste,
    /// Paste into the folder that was clicked.
    PasteInto,
    Rename,
    Delete,
    /// The "new file or folder" prompt.
    New,
    ToggleMark,
    CopyPath,
    Inspect,
    /// Opens the disk usage view on the folder that was clicked, or the browsed one.
    DiskUsage,
    ToggleHidden,
    Refresh,
}

/// What was right-clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Entry {
        is_dir: bool,
    },
    /// Empty space in a column: the menu is about the browsed directory itself.
    Blank,
}

/// The facts about the browser the menu's contents depend on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    pub target: Target,
    /// Whether the clicked entry is one of the marked ones.
    pub marked: bool,
    pub mark_count: usize,
    /// Whether something is waiting to be pasted.
    pub clipboard: bool,
    pub hidden_shown: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    pub command: MenuCommand,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Separator,
    Item(Item),
    Submenu { label: String, items: Vec<Item> },
}

impl Entry {
    fn item(label: impl Into<String>, command: MenuCommand) -> Self {
        Entry::Item(Item {
            label: label.into(),
            command,
            enabled: true,
        })
    }

    fn item_if(label: impl Into<String>, command: MenuCommand, enabled: bool) -> Self {
        Entry::Item(Item {
            label: label.into(),
            command,
            enabled,
        })
    }

    /// A submenu with nothing in it is shown but cannot be opened.
    pub fn is_enabled(&self) -> bool {
        match self {
            Entry::Separator => false,
            Entry::Item(item) => item.enabled,
            Entry::Submenu { items, .. } => !items.is_empty(),
        }
    }

    fn label(&self) -> Option<&str> {
        match self {
            Entry::Separator => None,
            Entry::Item(item) => Some(&item.label),
            Entry::Submenu { label, .. } => Some(label),
        }
    }
}

/// What the menu lists for `context`. `open_with` names the "Open with" choices, in the order
/// `MenuCommand::OpenWith` indexes them.
pub fn entries(context: &Context, open_with: &[String]) -> Vec<Entry> {
    // A batch action names how many entries it will touch, since the marks — not just the
    // clicked row — are what `Cut`, `Copy` and `Delete` act on.
    let batch = |label: &str| {
        if context.marked && context.mark_count > 1 {
            format!("{label} ({} marked)", context.mark_count)
        } else {
            label.to_owned()
        }
    };
    match context.target {
        Target::Entry { is_dir } => {
            let mut list = vec![
                Entry::item("Open", MenuCommand::Open),
                Entry::Submenu {
                    label: "Open with".into(),
                    items: open_with
                        .iter()
                        .enumerate()
                        .map(|(i, name)| Item {
                            label: name.clone(),
                            command: MenuCommand::OpenWith(i),
                            enabled: true,
                        })
                        .collect(),
                },
                Entry::Separator,
                Entry::item(batch("Cut"), MenuCommand::Cut),
                Entry::item(batch("Copy"), MenuCommand::Copy),
            ];
            if is_dir {
                list.push(Entry::item_if(
                    "Paste into folder",
                    MenuCommand::PasteInto,
                    context.clipboard,
                ));
            }
            list.extend([
                Entry::Separator,
                Entry::item("Rename", MenuCommand::Rename),
                Entry::item(batch("Delete"), MenuCommand::Delete),
                Entry::Separator,
                Entry::item(
                    if context.marked { "Unmark" } else { "Mark" },
                    MenuCommand::ToggleMark,
                ),
                Entry::item("Copy path", MenuCommand::CopyPath),
                Entry::item("Inspect", MenuCommand::Inspect),
            ]);
            if is_dir {
                list.push(Entry::item("Disk usage", MenuCommand::DiskUsage));
            }
            list
        }
        Target::Blank => vec![
            Entry::item("New file or folder…", MenuCommand::New),
            Entry::item_if("Paste", MenuCommand::Paste, context.clipboard),
            Entry::Separator,
            Entry::item(
                if context.hidden_shown {
                    "Hide hidden files"
                } else {
                    "Show hidden files"
                },
                MenuCommand::ToggleHidden,
            ),
            Entry::item("Refresh", MenuCommand::Refresh),
            Entry::Separator,
            Entry::item("Copy path", MenuCommand::CopyPath),
            Entry::item("Inspect this folder", MenuCommand::Inspect),
            Entry::item("Disk usage", MenuCommand::DiskUsage),
        ],
    }
}

/// Where the highlight is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Main(usize),
    Sub(usize),
}

/// What the pointer is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Main(usize),
    Sub(usize),
    /// On a menu but not on a row: a border or a separator.
    Inert,
    Outside,
}

/// What an interaction with the menu asks the caller to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Run(MenuCommand),
    /// Nothing to do outside the menu; keep it open.
    Stay,
    Dismiss,
}

/// The four arrows and `Enter`, however they were typed (`hjkl` included).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Left,
    Right,
    Enter,
}

/// The rectangles a menu occupies on a given screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuLayout {
    pub main: Rect,
    pub sub: Option<Rect>,
}

/// Columns a row needs beyond its label: border, a space each side, and room for the `▸`
/// that marks a submenu.
const CHROME: u16 = 6;

pub struct ContextMenu {
    anchor: Position,
    target: Target,
    entries: Vec<Entry>,
    hover: Option<Slot>,
    /// The main-list index of the submenu currently open beside it.
    open: Option<usize>,
}

impl ContextMenu {
    /// A menu for `target` whose top-left corner wants to be at `anchor` (where the pointer was).
    pub fn new(anchor: Position, target: Target, entries: Vec<Entry>) -> Self {
        Self {
            anchor,
            target,
            entries,
            hover: None,
            open: None,
        }
    }

    /// What the menu was opened on, which decides what "Inspect" and "Copy path" refer to.
    pub fn target(&self) -> Target {
        self.target
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn hover(&self) -> Option<Slot> {
        self.hover
    }

    pub fn open_submenu(&self) -> Option<usize> {
        self.open
    }

    /// The items of the submenu that is open, if one is.
    pub fn sub_items(&self) -> Option<&[Item]> {
        match self.open.and_then(|i| self.entries.get(i)) {
            Some(Entry::Submenu { items, .. }) => Some(items),
            _ => None,
        }
    }

    /// Where the menu and its open submenu sit on a screen of `frame`: at the pointer, pulled
    /// back inside the screen when it would run off the right or bottom edge, and a submenu on
    /// the left of its parent when there is no room on the right.
    pub fn layout(&self, frame: Rect) -> MenuLayout {
        let labels = self.entries.iter().filter_map(Entry::label);
        let main_width = width_for(labels);
        let main = place(
            self.anchor,
            main_width,
            self.entries.len() as u16 + 2,
            frame,
        );
        let sub = self.open.zip(self.sub_items()).map(|(parent, items)| {
            let width = width_for(items.iter().map(|item| item.label.as_str()));
            let height = items.len() as u16 + 2;
            // Its first row lines up with the parent row it opened from.
            let top = main.y.saturating_add(parent as u16);
            let right_of = main.right();
            let x = if right_of.saturating_add(width) <= frame.right() {
                right_of
            } else {
                main.x.saturating_sub(width)
            };
            place(Position::new(x, top), width, height, frame)
        });
        MenuLayout { main, sub }
    }

    /// What `pos` is over.
    pub fn hit(&self, frame: Rect, pos: Position) -> Hit {
        let layout = self.layout(frame);
        // The submenu is drawn on top, so it gets the pointer first.
        if let Some(sub) = layout.sub
            && sub.contains(pos)
        {
            return match row_at(sub, pos) {
                Some(row) if self.sub_items().is_some_and(|items| row < items.len()) => {
                    Hit::Sub(row)
                }
                _ => Hit::Inert,
            };
        }
        if layout.main.contains(pos) {
            return match row_at(layout.main, pos) {
                Some(row) if matches!(self.entries.get(row), Some(e) if *e != Entry::Separator) => {
                    Hit::Main(row)
                }
                _ => Hit::Inert,
            };
        }
        Hit::Outside
    }

    /// Follows the pointer: highlights the row under it, and opens the submenu of a submenu row
    /// it rests on (closing it when it moves to another main row).
    pub fn hover_at(&mut self, frame: Rect, pos: Position) {
        match self.hit(frame, pos) {
            Hit::Main(i) if self.entries[i].is_enabled() => {
                self.hover = Some(Slot::Main(i));
                self.open = matches!(self.entries[i], Entry::Submenu { .. }).then_some(i);
            }
            Hit::Main(_) => {
                self.hover = None;
                self.open = None;
            }
            Hit::Sub(j) => {
                self.hover = self
                    .sub_items()
                    .is_some_and(|items| items[j].enabled)
                    .then_some(Slot::Sub(j));
            }
            // Crossing a border or separator keeps what was lit, so a slow diagonal into the
            // submenu doesn't drop it.
            Hit::Inert => {}
            Hit::Outside => self.hover = None,
        }
    }

    /// A left click at `pos`.
    pub fn click_at(&mut self, frame: Rect, pos: Position) -> Outcome {
        match self.hit(frame, pos) {
            Hit::Outside => Outcome::Dismiss,
            Hit::Inert => Outcome::Stay,
            Hit::Sub(j) => match self.sub_items().map(|items| &items[j]) {
                Some(item) if item.enabled => Outcome::Run(item.command),
                _ => Outcome::Stay,
            },
            Hit::Main(i) => match &self.entries[i] {
                Entry::Item(item) if item.enabled => Outcome::Run(item.command),
                Entry::Submenu { items, .. } if !items.is_empty() => {
                    self.hover = Some(Slot::Main(i));
                    self.open = Some(i);
                    Outcome::Stay
                }
                _ => Outcome::Stay,
            },
        }
    }

    /// One key of keyboard navigation.
    pub fn key(&mut self, nav: Nav) -> Outcome {
        match (nav, self.hover) {
            (Nav::Up | Nav::Down, hover) => {
                let step = if nav == Nav::Down { 1 } else { -1 };
                match hover {
                    Some(Slot::Sub(j)) => {
                        if let Some(next) = self.sub_items().and_then(|items| {
                            step_to(items.len(), Some(j), step, |k| items[k].enabled)
                        }) {
                            self.hover = Some(Slot::Sub(next));
                        }
                    }
                    _ => {
                        let current = match hover {
                            Some(Slot::Main(i)) => Some(i),
                            _ => None,
                        };
                        if let Some(next) = step_to(self.entries.len(), current, step, |k| {
                            self.entries[k].is_enabled()
                        }) {
                            self.hover = Some(Slot::Main(next));
                            // Arrowing past a submenu row leaves it shut; `Right` opens it.
                            self.open = None;
                        }
                    }
                }
                Outcome::Stay
            }
            (Nav::Right | Nav::Enter, Some(Slot::Main(i))) => match &self.entries[i] {
                Entry::Item(item) if nav == Nav::Enter && item.enabled => {
                    Outcome::Run(item.command)
                }
                Entry::Submenu { items, .. } if !items.is_empty() => {
                    self.open = Some(i);
                    self.hover = items.iter().position(|item| item.enabled).map(Slot::Sub);
                    Outcome::Stay
                }
                _ => Outcome::Stay,
            },
            (Nav::Enter, Some(Slot::Sub(j))) => match self.sub_items().map(|items| &items[j]) {
                Some(item) if item.enabled => Outcome::Run(item.command),
                _ => Outcome::Stay,
            },
            (Nav::Left, Some(Slot::Sub(_))) => {
                self.hover = self.open.map(Slot::Main);
                self.open = None;
                Outcome::Stay
            }
            (Nav::Left, _) => Outcome::Dismiss,
            (Nav::Right | Nav::Enter, _) => Outcome::Stay,
        }
    }
}

/// The width of a menu listing `labels`.
fn width_for<'a>(labels: impl Iterator<Item = &'a str>) -> u16 {
    labels
        .map(text_width)
        .max()
        .map_or(CHROME, |widest| widest as u16 + CHROME)
}

/// A `width` × `height` box with its corner at `at`, moved back inside `frame` if it would
/// overhang, and shrunk to `frame` if it is bigger than the screen.
fn place(at: Position, width: u16, height: u16, frame: Rect) -> Rect {
    let (width, height) = (width.min(frame.width), height.min(frame.height));
    let x = at.x.min(frame.right().saturating_sub(width)).max(frame.x);
    let y = at.y.min(frame.bottom().saturating_sub(height)).max(frame.y);
    Rect::new(x, y, width, height)
}

/// The row index under `pos` inside `menu`'s border, or `None` on the border.
fn row_at(menu: Rect, pos: Position) -> Option<usize> {
    let inner = menu.inner(Margin::new(1, 1));
    inner.contains(pos).then(|| usize::from(pos.y - inner.y))
}

/// The nearest index from `from` in direction `step` (wrapping) that `enabled` accepts.
/// With no `from`, a downward step starts at the first row and an upward one at the last.
fn step_to(
    len: usize,
    from: Option<usize>,
    step: isize,
    enabled: impl Fn(usize) -> bool,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let start = match (from, step > 0) {
        (Some(i), _) => i as isize,
        (None, true) => -1,
        (None, false) => len as isize,
    };
    (1..=len as isize)
        .map(|n| (start + n * step).rem_euclid(len as isize) as usize)
        .find(|&k| enabled(k))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn file_context() -> Context {
        Context {
            target: Target::Entry { is_dir: false },
            marked: false,
            mark_count: 0,
            clipboard: false,
            hidden_shown: false,
        }
    }

    fn frame() -> Rect {
        Rect::new(0, 0, 100, 30)
    }

    fn menu_for(context: &Context, at: (u16, u16)) -> ContextMenu {
        ContextMenu::new(
            Position::new(at.0, at.1),
            context.target,
            entries(context, &["Neovim".into(), "VLC".into()]),
        )
    }

    fn labels(list: &[Entry]) -> Vec<&str> {
        list.iter().filter_map(Entry::label).collect()
    }

    #[test]
    fn a_file_menu_lists_the_essentials_in_a_conventional_order() {
        let list = entries(&file_context(), &[]);
        assert_eq!(
            labels(&list),
            [
                "Open",
                "Open with",
                "Cut",
                "Copy",
                "Rename",
                "Delete",
                "Mark",
                "Copy path",
                "Inspect"
            ]
        );
    }

    #[test]
    fn a_folder_menu_can_paste_into_the_folder_but_only_with_something_to_paste() {
        let mut context = file_context();
        context.target = Target::Entry { is_dir: true };
        let paste = |context: &Context| {
            entries(context, &[])
                .into_iter()
                .find_map(|e| match e {
                    Entry::Item(item) if item.command == MenuCommand::PasteInto => Some(item),
                    _ => None,
                })
                .unwrap()
        };
        assert!(!paste(&context).enabled);
        context.clipboard = true;
        assert!(paste(&context).enabled);
        assert!(
            !labels(&entries(&file_context(), &[])).contains(&"Paste into folder"),
            "a file has nothing to paste into"
        );
    }

    #[test]
    fn blank_space_offers_the_directory_level_actions_only() {
        let context = Context {
            target: Target::Blank,
            hidden_shown: true,
            ..file_context()
        };
        let list = entries(&context, &[]);
        assert_eq!(
            labels(&list),
            [
                "New file or folder…",
                "Paste",
                "Hide hidden files",
                "Refresh",
                "Copy path",
                "Inspect this folder",
                "Disk usage"
            ]
        );
    }

    #[test]
    fn only_a_folder_or_blank_space_offers_disk_usage() {
        let mut context = file_context();
        assert!(!labels(&entries(&context, &[])).contains(&"Disk usage"));
        context.target = Target::Entry { is_dir: true };
        let list = entries(&context, &[]);
        assert_eq!(labels(&list).last(), Some(&"Disk usage"));
    }

    #[test]
    fn batch_actions_say_how_many_marked_entries_they_touch() {
        let context = Context {
            marked: true,
            mark_count: 3,
            ..file_context()
        };
        let list = entries(&context, &[]);
        let names = labels(&list);
        assert!(names.contains(&"Delete (3 marked)"));
        assert!(names.contains(&"Unmark"));
        assert!(!labels(&entries(&file_context(), &[])).contains(&"Delete (3 marked)"));
    }

    #[test]
    fn the_menu_opens_at_the_pointer_and_is_pulled_back_from_the_edges() {
        let menu = menu_for(&file_context(), (10, 5));
        let main = menu.layout(frame()).main;
        assert_eq!((main.x, main.y), (10, 5));

        let menu = menu_for(&file_context(), (99, 29));
        let main = menu.layout(frame()).main;
        assert_eq!(main.right(), 100);
        assert_eq!(main.bottom(), 30);
    }

    #[test]
    fn hovering_open_with_opens_its_submenu_beside_it_and_leaving_shuts_it() {
        let mut menu = menu_for(&file_context(), (10, 5));
        // Row 0 is "Open", row 1 is "Open with"; rows start one cell inside the border.
        menu.hover_at(frame(), Position::new(12, 7));
        assert_eq!(menu.hover(), Some(Slot::Main(1)));
        assert_eq!(menu.open_submenu(), Some(1));
        let layout = menu.layout(frame());
        let sub = layout.sub.expect("the submenu is open");
        assert_eq!(sub.x, layout.main.right());

        menu.hover_at(frame(), Position::new(sub.x + 2, sub.y + 1));
        assert_eq!(menu.hover(), Some(Slot::Sub(0)));
        assert_eq!(menu.open_submenu(), Some(1), "moving into it keeps it open");

        menu.hover_at(frame(), Position::new(12, 6));
        assert_eq!(menu.open_submenu(), None, "another row closes it");
    }

    #[test]
    fn a_submenu_that_does_not_fit_on_the_right_opens_on_the_left() {
        let mut menu = menu_for(&file_context(), (85, 5));
        let main = menu.layout(frame()).main;
        menu.hover_at(frame(), Position::new(main.x + 2, main.y + 2));
        let sub = menu.layout(frame()).sub.unwrap();
        assert!(sub.right() <= main.x, "the submenu sits left of its parent");
    }

    #[test]
    fn clicking_runs_an_item_and_outside_the_menu_dismisses_it() {
        let mut menu = menu_for(&file_context(), (10, 5));
        assert_eq!(
            menu.click_at(frame(), Position::new(12, 6)),
            Outcome::Run(MenuCommand::Open)
        );
        assert_eq!(
            menu.click_at(frame(), Position::new(50, 20)),
            Outcome::Dismiss
        );
        // The top border is part of the menu, and not a row.
        assert_eq!(menu.click_at(frame(), Position::new(12, 5)), Outcome::Stay);
    }

    #[test]
    fn clicking_a_submenu_row_opens_it_and_clicking_its_item_runs_it() {
        let mut menu = menu_for(&file_context(), (10, 5));
        assert_eq!(menu.click_at(frame(), Position::new(12, 7)), Outcome::Stay);
        let sub = menu.layout(frame()).sub.unwrap();
        assert_eq!(
            menu.click_at(frame(), Position::new(sub.x + 2, sub.y + 2)),
            Outcome::Run(MenuCommand::OpenWith(1))
        );
    }

    #[test]
    fn a_disabled_item_cannot_be_run_by_click_or_key() {
        let context = Context {
            target: Target::Blank,
            ..file_context()
        };
        let mut menu = menu_for(&context, (10, 5));
        // "Paste" is the second row and is disabled with an empty clipboard.
        assert_eq!(menu.click_at(frame(), Position::new(12, 7)), Outcome::Stay);
        menu.key(Nav::Down);
        assert_eq!(menu.hover(), Some(Slot::Main(0)));
        assert_eq!(
            menu.key(Nav::Down),
            Outcome::Stay,
            "the highlight skips a disabled row"
        );
        assert_eq!(menu.hover(), Some(Slot::Main(3)));
    }

    #[test]
    fn the_keyboard_walks_the_menu_and_wraps() {
        let mut menu = menu_for(&file_context(), (10, 5));
        menu.key(Nav::Up);
        assert_eq!(menu.hover(), Some(Slot::Main(menu.entries().len() - 1)));
        menu.key(Nav::Down);
        assert_eq!(menu.hover(), Some(Slot::Main(0)));
        assert_eq!(menu.key(Nav::Enter), Outcome::Run(MenuCommand::Open));
    }

    #[test]
    fn right_enters_the_submenu_and_left_comes_back_out_before_it_closes_the_menu() {
        let mut menu = menu_for(&file_context(), (10, 5));
        menu.key(Nav::Down);
        menu.key(Nav::Down);
        assert_eq!(menu.hover(), Some(Slot::Main(1)));
        menu.key(Nav::Right);
        assert_eq!(menu.hover(), Some(Slot::Sub(0)));
        menu.key(Nav::Down);
        assert_eq!(menu.key(Nav::Enter), Outcome::Run(MenuCommand::OpenWith(1)));

        assert_eq!(menu.key(Nav::Left), Outcome::Stay);
        assert_eq!(menu.hover(), Some(Slot::Main(1)));
        assert_eq!(menu.open_submenu(), None);
        assert_eq!(menu.key(Nav::Left), Outcome::Dismiss);
    }

    #[test]
    fn an_empty_open_with_list_is_shown_but_cannot_be_opened() {
        let mut menu = ContextMenu::new(
            Position::new(10, 5),
            file_context().target,
            entries(&file_context(), &[]),
        );
        menu.hover_at(frame(), Position::new(12, 7));
        assert_eq!(menu.hover(), None);
        assert_eq!(menu.open_submenu(), None);
        assert_eq!(menu.click_at(frame(), Position::new(12, 7)), Outcome::Stay);
    }

    proptest! {
        /// However the pointer sits, a menu that fits on screen ends up entirely on it.
        #[test]
        fn a_menu_and_its_submenu_stay_on_screen(
            x in 0u16..200, y in 0u16..80, w in 30u16..200, h in 14u16..80,
            hover_row in 0usize..12,
        ) {
            let screen = Rect::new(0, 0, w, h);
            let mut menu = menu_for(&file_context(), (x, y));
            let main = menu.layout(screen).main;
            prop_assert!(screen.contains(Position::new(main.x, main.y)));
            prop_assert!(main.right() <= screen.right() && main.bottom() <= screen.bottom());

            menu.hover_at(screen, Position::new(main.x + 2, main.y + 1 + hover_row as u16));
            if let Some(sub) = menu.layout(screen).sub {
                prop_assert!(sub.right() <= screen.right() && sub.bottom() <= screen.bottom());
            }
        }

        /// Every visible row is reachable: hit-testing the middle of a row finds that row.
        #[test]
        fn a_row_hits_the_entry_drawn_on_it(x in 0u16..90, y in 0u16..20, row in 0usize..9) {
            let screen = frame();
            let menu = menu_for(&file_context(), (x, y));
            let main = menu.layout(screen).main;
            let hit = menu.hit(screen, Position::new(main.x + 2, main.y + 1 + row as u16));
            match &menu.entries()[row] {
                Entry::Separator => prop_assert_eq!(hit, Hit::Inert),
                _ => prop_assert_eq!(hit, Hit::Main(row)),
            }
        }

        /// No sequence of pointer moves, clicks and keys can leave the menu pointing at a row
        /// that is not there or is not enabled.
        #[test]
        fn interaction_never_highlights_a_missing_or_disabled_row(
            steps in proptest::collection::vec((0u8..8, 0u16..100, 0u16..30), 0..40)
        ) {
            let screen = frame();
            let mut menu = menu_for(&file_context(), (30, 4));
            for (kind, x, y) in steps {
                let at = Position::new(x, y);
                match kind {
                    0 => menu.hover_at(screen, at),
                    1 => { menu.click_at(screen, at); }
                    2 => { menu.key(Nav::Up); }
                    3 => { menu.key(Nav::Down); }
                    4 => { menu.key(Nav::Left); }
                    5 => { menu.key(Nav::Right); }
                    _ => { menu.key(Nav::Enter); }
                }
                match menu.hover() {
                    Some(Slot::Main(i)) => prop_assert!(menu.entries()[i].is_enabled()),
                    Some(Slot::Sub(j)) => {
                        let items = menu.sub_items().expect("a sub highlight needs an open submenu");
                        prop_assert!(items[j].enabled);
                    }
                    None => {}
                }
            }
        }
    }
}
