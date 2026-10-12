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

//! The deck's state (`V`, the deck view): the devices found, how busy each is, which card is
//! selected and whether the keys are going to the rail. Like `settings_popup` it is pure: no
//! `Frame`, no terminal; `deck_view` draws it and `main` applies the few things it asks for, which
//! are to go to a folder, to show a message and to ask the devices plugin to do something.
//!
//! Nothing here looks at the machine. The plugin that provides `devices` does that, and sends the
//! whole list about once a second while the rail is showing (`accept`); mounting, unmounting and
//! ejecting are asked of it as a `DevicesCommand` and what came of them arrives as a note. What
//! the deck keeps is what the interface decides: the selection, the focus, the slide-in of a
//! device that has just appeared, and whether carried files will fit (`cargo_fit`).

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use plugin_api::{DevicesCommand, DevicesNote, SetDevices};
use ratatui::layout::{Position, Rect};
use shared::devices::activity::Rates;
use shared::devices::plan::Plan;
use shared::devices::{Device, holding};

use crate::cargo_fit::{self, Load, Verdict};
use crate::deck_view;
use crate::device_actions::send_check;

/// How long a card takes to slide in when a device is plugged in.
pub const SLIDE_IN: Duration = Duration::from_millis(260);

/// How long the interface waits before it tries to start a devices plugin that stopped again.
const RESTART_AFTER: Duration = Duration::from_secs(3);

/// Shown when the keys go to the cards.
pub const HINT: &str =
    "devices: ↑/↓ choose  Enter open  p send marked  m mount/unmount  e eject  Tab/Esc back";

/// What a key or click did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Not the deck's: the browser handles it as usual.
    Pass,
    /// Handled here.
    Handled,
    /// Go to this folder.
    Goto(PathBuf),
    /// Copy what is being carried into this folder, the root of the chosen device. The caller
    /// holds the files, so it does the copy; the deck has already checked that it will fit.
    Send(PathBuf),
    /// Ask the devices plugin to do this to a device.
    Do(DevicesCommand),
}

pub use crate::device_actions::Event;

pub struct DeckState {
    devices: Vec<Device>,
    rates: HashMap<String, Rates>,
    histories: HashMap<String, Vec<u64>>,
    /// When each device was first seen, so a new one slides in. The first list marks everything
    /// as long since seen.
    first_seen: HashMap<String, Instant>,
    scanned: bool,
    /// The node of the selected card, so the selection stays on its device as the list changes.
    selected_node: Option<String>,
    focused: bool,
    /// The first line item (a heading or a card) drawn, kept by the renderer between frames.
    first_card: Cell<usize>,
    /// A mount, unmount or eject is in progress, as the plugin last said.
    busy: bool,
    /// Why there is nothing to show: no devices plugin, or it stopped. Cleared by a list.
    missing: Option<String>,
    /// When a stopped plugin was last tried again.
    restarted: Option<Instant>,
    events: VecDeque<Event>,
    /// What the user is carrying, for the cards to say whether it fits.
    load: Option<Load>,
}

impl DeckState {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
            rates: HashMap::new(),
            histories: HashMap::new(),
            first_seen: HashMap::new(),
            scanned: false,
            selected_node: None,
            focused: false,
            first_card: Cell::new(0),
            busy: false,
            missing: None,
            restarted: None,
            events: VecDeque::new(),
            load: None,
        }
    }

    /// A deck that already holds `devices`, without ever hearing from a plugin: for the tests of
    /// this module and of how it is drawn.
    #[cfg(test)]
    pub fn with_devices(devices: Vec<Device>) -> Self {
        let mut deck = Self::new();
        deck.devices = devices;
        deck.scanned = true;
        deck.reselect();
        deck
    }

    /// Records that `node` appeared at `at`, so its card is mid-slide then.
    #[cfg(test)]
    pub fn mark_arrived(&mut self, node: &str, at: Instant) {
        self.first_seen.insert(node.to_string(), at);
    }

    #[cfg(test)]
    pub fn devices_mut(&mut self) -> &mut Vec<Device> {
        &mut self.devices
    }

    #[cfg(test)]
    pub fn set_scanned(&mut self, scanned: bool) {
        self.scanned = scanned;
    }

    /// Takes in the plugin's list, which replaces the last one.
    pub fn accept(&mut self, set: SetDevices, now: Instant) {
        self.missing = None;
        self.busy = set.busy;
        self.rates = set
            .devices
            .iter()
            .map(|d| (d.node.clone(), d.rates()))
            .collect();
        self.histories = set
            .devices
            .iter()
            .map(|d| (d.node.clone(), d.history.clone()))
            .collect();
        let devices: Vec<Device> = set.devices.iter().map(|d| d.device()).collect();
        // A device seen for the first time slides in, except on the very first list, when
        // everything is already there.
        let long_ago = now.checked_sub(SLIDE_IN * 4).unwrap_or(now);
        self.first_seen
            .retain(|node, _| devices.iter().any(|d| &d.node == node));
        for device in &devices {
            self.first_seen
                .entry(device.node.clone())
                .or_insert(if self.scanned { now } else { long_ago });
        }
        self.scanned = true;
        self.devices = devices;
        self.reselect();
    }

    /// Takes in what the plugin said an action came to.
    pub fn note(&mut self, note: DevicesNote) {
        self.events.push_back(match note {
            DevicesNote::Status { text } => Event::Status(text),
            DevicesNote::Goto { path, text } => Event::Goto(path, text),
        });
    }

    /// Queues a message for the caller to show.
    pub fn say(&mut self, text: impl Into<String>) {
        self.events.push_back(Event::Status(text.into()));
    }

    /// Says there is no one to fill the rail (`Some(why)`), or that there is again (`None`).
    pub fn set_missing(&mut self, why: Option<String>) {
        if why.is_some() {
            self.busy = false;
        }
        self.missing = why;
    }

    /// Why there is nothing to show, when there is a reason.
    pub fn missing(&self) -> Option<&str> {
        self.missing.as_deref()
    }

    /// Whether to start a plugin that has stopped again now: at most once in a while, so one that
    /// dies at once is not started in a loop.
    pub fn should_restart(&mut self, running: bool, now: Instant) -> bool {
        if running {
            self.restarted = None;
            return false;
        }
        let due = self
            .restarted
            .is_none_or(|at| now.duration_since(at) >= RESTART_AFTER);
        if due {
            self.restarted = Some(now);
        }
        due
    }

    /// Puts the selection back on the device it was on, or on the nearest card if that is gone.
    fn reselect(&mut self) {
        let at = self
            .selected_node
            .as_deref()
            .and_then(|node| self.devices.iter().position(|d| d.node == node))
            .unwrap_or_else(|| self.selected().min(self.devices.len().saturating_sub(1)));
        self.selected_node = self.devices.get(at).map(|d| d.node.clone());
    }

    /// Tells the deck what is being carried (or that nothing is), so each card can say whether it
    /// fits. Cheap; call it every turn.
    pub fn set_load(&mut self, load: Option<Load>) {
        self.load = load;
    }

    /// Whether the load fits on the device at `index`, or `None` when nothing is carried.
    pub fn verdict(&self, index: usize) -> Option<Verdict> {
        let load = self.load.as_ref()?;
        let device = self.devices.get(index)?;
        let same_device = self.holding(&load.source) == Some(index);
        Some(cargo_fit::verdict(device, load, same_device))
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// Whether the first list has come: until it has, an empty deck means "looking".
    pub fn has_scanned(&self) -> bool {
        self.scanned
    }

    pub fn selected(&self) -> usize {
        self.selected_node
            .as_deref()
            .and_then(|node| self.devices.iter().position(|d| d.node == node))
            .unwrap_or(0)
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    pub fn unfocus(&mut self) {
        self.focused = false;
    }

    pub fn first_card(&self) -> &Cell<usize> {
        &self.first_card
    }

    pub fn rates(&self, device: &Device) -> Rates {
        self.rates.get(&device.node).copied().unwrap_or_default()
    }

    pub fn history(&self, device: &Device) -> Vec<u64> {
        self.histories
            .get(&device.node)
            .cloned()
            .unwrap_or_default()
    }

    /// How far through its slide in the card of `device` is, from 0 (just plugged in) to 1.
    pub fn slide(&self, device: &Device, now: Instant) -> f64 {
        self.first_seen.get(&device.node).map_or(1.0, |seen| {
            (now.saturating_duration_since(*seen).as_secs_f64() / SLIDE_IN.as_secs_f64()).min(1.0)
        })
    }

    /// Whether any card is still sliding in, so the loop should tick fast.
    pub fn animating(&self, now: Instant) -> bool {
        self.devices.iter().any(|d| self.slide(d, now) < 1.0)
    }

    /// Whether a mount, unmount or eject is in progress.
    pub fn is_working(&self) -> bool {
        self.busy
    }

    /// The next message or folder change, if any.
    pub fn take_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// The device whose mount holds `dir`, if any: the one with the longest mount path that is a
    /// parent of it, so a folder on a separately mounted `/home` belongs to that and not to `/`.
    pub fn holding(&self, dir: &Path) -> Option<usize> {
        holding(&self.devices, dir)
    }

    fn step(&mut self, by: isize) {
        if self.devices.is_empty() {
            return;
        }
        let last = self.devices.len() - 1;
        let at = self.selected().saturating_add_signed(by).min(last);
        self.selected_node = Some(self.devices[at].node.clone());
    }

    /// Where the keys go in a deck whose rail is at the left of the files (`visible`; with no
    /// room for the rail they are all the browser's). `Tab` takes them to the rail and gives them
    /// back. With them there, the up and down arrows choose a card, `Enter` opens it (mounting it
    /// first if it is not mounted), `m` mounts or unmounts it, `e` ejects it, `p` sends what is
    /// carried and `Esc` gives the keys back. Every other key is the browser's, `h`, `j`, `k` and
    /// `l` included, so the files are browsed the same way in every view and `q` still quits.
    pub fn key(&mut self, key: KeyEvent, visible: bool) -> Outcome {
        if !visible {
            self.focused = false;
            return Outcome::Pass;
        }
        if key.code == KeyCode::Tab && key.modifiers == KeyModifiers::NONE {
            self.focused = !self.focused;
            return Outcome::Handled;
        }
        if !self.focused || key.modifiers.contains(KeyModifiers::CONTROL) {
            return Outcome::Pass;
        }
        match key.code {
            KeyCode::Esc => self.focused = false,
            KeyCode::Up => self.step(-1),
            KeyCode::Down => self.step(1),
            KeyCode::Enter => return self.open_selected(),
            KeyCode::Char('p') => return self.send_selected(),
            KeyCode::Char('m') => return self.toggle_selected(),
            KeyCode::Char('e') => return self.eject_selected(),
            _ => return Outcome::Pass,
        }
        Outcome::Handled
    }

    /// A click on the card under `pointer` in a deck drawn in `area` selects it and takes the
    /// keys; a click on the card that is already selected opens it. A click on no card is
    /// handled and does nothing.
    pub fn click(&mut self, area: Rect, pointer: Position) -> Outcome {
        let Some(index) = deck_view::card_at(area, self, pointer) else {
            return Outcome::Handled;
        };
        let already = self.focused && index == self.selected();
        self.focused = true;
        self.selected_node = Some(self.devices[index].node.clone());
        if already {
            self.open_selected()
        } else {
            Outcome::Handled
        }
    }

    /// Moves the selection by `by` cards, as the wheel does.
    pub fn scroll(&mut self, by: isize) {
        self.step(by);
    }

    fn selected_device(&self) -> Option<&Device> {
        self.devices.get(self.selected())
    }

    fn open_selected(&mut self) -> Outcome {
        let Some(device) = self.selected_device().cloned() else {
            return Outcome::Handled;
        };
        match device.mount {
            Some(mount) => Outcome::Goto(mount),
            None => Outcome::Do(DevicesCommand::Open { node: device.node }),
        }
    }

    /// `p`: send what is carried to the chosen device, if it will fit. Refused here, with the
    /// reason, when it will not, which is the point of asking before the copy starts and not
    /// halfway through it. With nothing carried the caller says so.
    fn send_selected(&mut self) -> Outcome {
        let Some(device) = self.selected_device().cloned() else {
            return Outcome::Handled;
        };
        match send_check(&device, self.verdict(self.selected())) {
            Ok(mount) => Outcome::Send(mount),
            Err(why) => {
                self.say(why);
                Outcome::Handled
            }
        }
    }

    fn toggle_selected(&mut self) -> Outcome {
        let Some(device) = self.selected_device().cloned() else {
            return Outcome::Handled;
        };
        match Plan::toggle(&device) {
            Ok(_) => Outcome::Do(DevicesCommand::Toggle { node: device.node }),
            Err(why) => {
                self.say(why);
                Outcome::Handled
            }
        }
    }

    fn eject_selected(&mut self) -> Outcome {
        let Some(device) = self.selected_device().cloned() else {
            return Outcome::Handled;
        };
        match Plan::eject(&device) {
            Ok(_) => Outcome::Do(DevicesCommand::Eject { node: device.node }),
            Err(why) => {
                self.say(why);
                Outcome::Handled
            }
        }
    }
}

impl Default for DeckState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use shared::devices::Kind;

    use super::*;

    fn device(node: &str, mount: Option<&str>, kind: Kind) -> Device {
        Device {
            node: node.into(),
            disk: node.trim_end_matches(char::is_numeric).into(),
            label: None,
            fs: Some("ext4".into()),
            mount: mount.map(PathBuf::from),
            total: 1000,
            free: 500,
            kind,
        }
    }

    /// A deck that already holds `devices`, without ever hearing from a plugin.
    fn deck_with(devices: Vec<Device>) -> DeckState {
        DeckState::with_devices(devices)
    }

    fn said(deck: &mut DeckState) -> Vec<String> {
        let mut said = Vec::new();
        while let Some(Event::Status(text)) = deck.take_event() {
            said.push(text);
        }
        said
    }

    fn listed(devices: &[Device]) -> SetDevices {
        SetDevices {
            devices: devices
                .iter()
                .map(|d| plugin_api::WireDevice::new(d, Rates::default(), Vec::new()))
                .collect(),
            busy: false,
        }
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn three() -> Vec<Device> {
        vec![
            device("sda1", Some("/"), Kind::Internal),
            device("sdb1", Some("/run/media/me/STICK"), Kind::Removable),
            device("sdc1", None, Kind::Removable),
        ]
    }

    #[test]
    fn the_deepest_mount_holding_a_folder_is_the_device_it_is_on() {
        let devices = vec![
            device("sda1", Some("/"), Kind::Internal),
            device("sda2", Some("/home"), Kind::Internal),
            device("sdc1", None, Kind::Removable),
        ];
        assert_eq!(holding(&devices, Path::new("/home/me/docs")), Some(1));
        assert_eq!(holding(&devices, Path::new("/etc")), Some(0));
        assert_eq!(
            holding(&devices, Path::new("/homestead")),
            Some(0),
            "a name, not a prefix"
        );
        assert_eq!(holding(&[], Path::new("/")), None);
    }

    #[test]
    fn tab_takes_the_keys_to_the_rail_and_gives_them_back() {
        let mut deck = deck_with(three());
        assert!(!deck.focused());
        assert_eq!(
            deck.key(press(KeyCode::Down), true),
            Outcome::Pass,
            "the browser's until Tab"
        );
        assert_eq!(deck.key(press(KeyCode::Tab), true), Outcome::Handled);
        assert!(deck.focused());
        assert_eq!(deck.key(press(KeyCode::Esc), true), Outcome::Handled);
        assert!(!deck.focused());
        deck.key(press(KeyCode::Tab), true);
        deck.key(press(KeyCode::Tab), true);
        assert!(!deck.focused());
    }

    #[test]
    fn hjkl_are_the_browsers_whether_or_not_the_rail_has_the_keys() {
        let mut deck = deck_with(three());
        for focused in [false, true] {
            if focused {
                deck.key(press(KeyCode::Tab), true);
            }
            for code in ['h', 'j', 'k', 'l'] {
                assert_eq!(
                    deck.key(press(KeyCode::Char(code)), true),
                    Outcome::Pass,
                    "{code} with the rail focused: {focused}"
                );
            }
            for code in [KeyCode::Left, KeyCode::Right] {
                assert_eq!(deck.key(press(code), true), Outcome::Pass, "{code:?}");
            }
        }
        assert_eq!(deck.selected(), 0, "none of them moved the rail");
    }

    #[test]
    fn the_up_and_down_arrows_choose_cards_and_stop_at_the_ends() {
        let mut deck = deck_with(three());
        assert_eq!(
            deck.key(press(KeyCode::Down), true),
            Outcome::Pass,
            "the browser's until Tab"
        );
        deck.key(press(KeyCode::Tab), true);
        assert_eq!(deck.selected(), 0);
        deck.key(press(KeyCode::Up), true);
        assert_eq!(deck.selected(), 0);
        deck.key(press(KeyCode::Down), true);
        deck.key(press(KeyCode::Down), true);
        deck.key(press(KeyCode::Down), true);
        assert_eq!(deck.selected(), 2);
        deck.key(press(KeyCode::Up), true);
        assert_eq!(deck.selected(), 1);
    }

    #[test]
    fn keys_the_deck_does_not_use_stay_the_browsers_even_with_it_focused() {
        let mut deck = deck_with(three());
        deck.key(press(KeyCode::Tab), true);
        for code in [KeyCode::Char('q'), KeyCode::Char('x'), KeyCode::Backspace] {
            assert_eq!(deck.key(press(code), true), Outcome::Pass, "{code:?}");
        }
        assert_eq!(
            deck.key(
                KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL),
                true
            ),
            Outcome::Pass
        );
    }

    #[test]
    fn with_no_room_for_the_rail_every_key_is_the_browsers() {
        let mut deck = deck_with(three());
        deck.key(press(KeyCode::Tab), true);
        for code in [KeyCode::Tab, KeyCode::Down, KeyCode::Enter] {
            assert_eq!(deck.key(press(code), false), Outcome::Pass, "{code:?}");
        }
        assert!(
            !deck.focused(),
            "the keys cannot stay on a rail that is not there"
        );
    }

    #[test]
    fn enter_on_a_mounted_card_goes_to_its_folder() {
        let mut deck = deck_with(three());
        deck.key(press(KeyCode::Tab), true);
        deck.key(press(KeyCode::Down), true);
        assert_eq!(
            deck.key(press(KeyCode::Enter), true),
            Outcome::Goto(PathBuf::from("/run/media/me/STICK"))
        );
    }

    #[test]
    fn enter_on_a_card_that_is_not_mounted_asks_the_plugin_to_mount_and_open_it() {
        let mut deck = deck_with(three());
        deck.key(press(KeyCode::Tab), true);
        deck.key(press(KeyCode::Down), true);
        deck.key(press(KeyCode::Down), true);
        assert_eq!(
            deck.key(press(KeyCode::Enter), true),
            Outcome::Do(DevicesCommand::Open {
                node: "sdc1".into()
            })
        );
    }

    #[test]
    fn m_and_e_ask_the_plugin_to_mount_or_eject_the_chosen_device() {
        let mut deck = deck_with(three());
        deck.key(press(KeyCode::Tab), true);
        deck.key(press(KeyCode::Down), true);
        assert_eq!(
            deck.key(press(KeyCode::Char('m')), true),
            Outcome::Do(DevicesCommand::Toggle {
                node: "sdb1".into()
            })
        );
        assert_eq!(
            deck.key(press(KeyCode::Char('e')), true),
            Outcome::Do(DevicesCommand::Eject {
                node: "sdb1".into()
            })
        );
    }

    #[test]
    fn a_network_share_and_a_fixed_disk_refuse_what_they_cannot_do_and_say_why() {
        let mut deck = deck_with(vec![
            device("nas:/export", Some("/mnt/nas"), Kind::Network),
            device("sda1", Some("/"), Kind::Internal),
        ]);
        deck.key(press(KeyCode::Tab), true);
        assert_eq!(deck.key(press(KeyCode::Char('m')), true), Outcome::Handled);
        deck.key(press(KeyCode::Down), true);
        assert_eq!(deck.key(press(KeyCode::Char('e')), true), Outcome::Handled);
        let said = said(&mut deck);
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].contains("network share"));
        assert!(said[1].contains("cannot be ejected"));
    }

    #[test]
    fn a_list_replaces_the_last_one_with_how_busy_each_device_is() {
        let mut deck = deck_with(Vec::new());
        let mut set = listed(&three());
        set.devices[1].read = 7;
        set.devices[1].history = vec![1, 2, 3];
        set.busy = true;
        deck.accept(set, Instant::now());
        assert_eq!(deck.devices().len(), 3);
        let stick = deck.devices()[1].clone();
        assert_eq!(deck.rates(&stick).read, 7);
        assert_eq!(deck.history(&stick), [1, 2, 3]);
        assert!(deck.is_working());
        deck.accept(listed(&three()[..1]), Instant::now());
        assert_eq!(deck.devices().len(), 1);
        assert_eq!(
            deck.rates(&stick),
            Rates::default(),
            "a device that left has no rate"
        );
        assert!(!deck.is_working());
    }

    #[test]
    fn a_list_clears_the_reason_nothing_was_shown_and_a_missing_plugin_stops_the_busy_mark() {
        let mut deck = deck_with(Vec::new());
        deck.set_missing(Some("no plugin".into()));
        assert_eq!(deck.missing(), Some("no plugin"));
        deck.accept(listed(&three()), Instant::now());
        assert_eq!(deck.missing(), None);
    }

    #[test]
    fn the_plugins_notes_come_out_as_messages_and_folders_in_order() {
        let mut deck = deck_with(Vec::new());
        deck.note(DevicesNote::Status { text: "a".into() });
        deck.note(DevicesNote::Goto {
            path: "/mnt".into(),
            text: "b".into(),
        });
        assert_eq!(deck.take_event(), Some(Event::Status("a".into())));
        assert_eq!(
            deck.take_event(),
            Some(Event::Goto("/mnt".into(), "b".into()))
        );
        assert_eq!(deck.take_event(), None);
    }

    #[test]
    fn a_stopped_plugin_is_started_again_but_not_in_a_loop() {
        let mut deck = deck_with(Vec::new());
        let now = Instant::now();
        assert!(!deck.should_restart(true, now));
        assert!(deck.should_restart(false, now));
        assert!(!deck.should_restart(false, now + Duration::from_secs(1)));
        assert!(deck.should_restart(false, now + RESTART_AFTER));
        assert!(!deck.should_restart(true, now + RESTART_AFTER));
        assert!(deck.should_restart(false, now + RESTART_AFTER));
    }

    fn load(bytes: u64, largest: u64) -> Load {
        Load {
            bytes,
            exact: true,
            largest,
            carry: cargo_fit::Carry::Copy,
            source: "/home/me/photos".into(),
        }
    }

    fn with_sizes(mut devices: Vec<Device>, free: &[u64]) -> Vec<Device> {
        for (device, free) in devices.iter_mut().zip(free) {
            device.free = *free;
            device.total = 1 << 40;
        }
        devices
    }

    #[test]
    fn each_card_gets_its_own_verdict_for_what_is_carried() {
        let mut deck = deck_with(with_sizes(three(), &[1 << 39, 1 << 20, 0]));
        assert_eq!(deck.verdict(0), None, "nothing carried, nothing to judge");
        deck.set_load(Some(load(1 << 30, 1 << 20)));
        assert!(matches!(deck.verdict(0), Some(Verdict::Fits { .. })));
        assert!(matches!(deck.verdict(1), Some(Verdict::Short { .. })));
        assert_eq!(deck.verdict(2), Some(Verdict::Unmounted));
        assert_eq!(deck.verdict(9), None);
        deck.set_load(None);
        assert_eq!(deck.verdict(0), None);
    }

    #[test]
    fn p_sends_to_a_device_it_fits_on_and_refuses_one_it_does_not_with_the_reason() {
        let mut deck = deck_with(with_sizes(three(), &[1 << 39, 1 << 20, 0]));
        deck.set_load(Some(load(1 << 30, 1 << 20)));
        deck.key(press(KeyCode::Tab), true);
        assert_eq!(
            deck.key(press(KeyCode::Char('p')), true),
            Outcome::Send(PathBuf::from("/")),
            "the first card has room"
        );
        deck.key(press(KeyCode::Down), true);
        assert_eq!(deck.key(press(KeyCode::Char('p')), true), Outcome::Handled);
        deck.key(press(KeyCode::Down), true);
        assert_eq!(deck.key(press(KeyCode::Char('p')), true), Outcome::Handled);
        let said = said(&mut deck);
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(
            said[0].contains("needs") && said[0].contains("more room"),
            "{said:?}"
        );
        assert!(said[1].contains("not mounted"), "{said:?}");
    }

    #[test]
    fn p_with_nothing_carried_still_goes_to_the_caller_who_says_so() {
        let mut deck = deck_with(with_sizes(three(), &[1 << 39, 1 << 39, 0]));
        deck.key(press(KeyCode::Tab), true);
        assert_eq!(
            deck.key(press(KeyCode::Char('p')), true),
            Outcome::Send(PathBuf::from("/"))
        );
    }

    #[test]
    fn a_move_within_the_disk_it_is_on_is_allowed_whatever_is_free() {
        let mut devices = with_sizes(three(), &[0, 1 << 39, 0]);
        devices[0].mount = Some("/home".into());
        let mut deck = deck_with(devices);
        let mut moving = load(1 << 38, 1 << 20);
        moving.carry = cargo_fit::Carry::Move;
        deck.set_load(Some(moving));
        assert_eq!(deck.verdict(0), Some(Verdict::SameDisk));
        deck.key(press(KeyCode::Tab), true);
        assert!(matches!(
            deck.key(press(KeyCode::Char('p')), true),
            Outcome::Send(_)
        ));
    }

    #[test]
    fn the_selection_follows_its_device_when_the_list_changes() {
        let mut deck = deck_with(three());
        deck.key(press(KeyCode::Tab), true);
        deck.key(press(KeyCode::Down), true);
        assert_eq!(deck.devices()[deck.selected()].node, "sdb1");
        let mut shifted = three();
        shifted.insert(0, device("nvme0n1p1", Some("/data"), Kind::Internal));
        deck.devices = shifted;
        deck.reselect();
        assert_eq!(deck.devices()[deck.selected()].node, "sdb1");
        deck.devices.retain(|d| d.node != "sdb1");
        deck.reselect();
        assert!(
            deck.selected() < deck.devices().len(),
            "it lands on a card that exists"
        );
    }

    #[test]
    fn a_device_plugged_in_after_the_first_scan_slides_in_and_the_first_ones_do_not() {
        let mut deck = deck_with(Vec::new());
        let now = Instant::now();
        deck.scanned = false;
        deck.accept(listed(&[device("sda1", Some("/"), Kind::Internal)]), now);
        assert!(
            !deck.animating(now),
            "everything already there is not an arrival"
        );
        let later = now + Duration::from_secs(2);
        deck.accept(listed(&three()), later);
        let stick = deck
            .devices()
            .iter()
            .find(|d| d.node == "sdb1")
            .unwrap()
            .clone();
        assert!(deck.slide(&stick, later) < 0.01);
        assert!(deck.animating(later));
        assert_eq!(deck.slide(&stick, later + SLIDE_IN), 1.0);
        assert!(!deck.animating(later + SLIDE_IN));
        let system = deck
            .devices()
            .iter()
            .find(|d| d.node == "sda1")
            .unwrap()
            .clone();
        assert_eq!(deck.slide(&system, later), 1.0);
    }
}
