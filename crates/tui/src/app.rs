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

//! Clipboard + prompt + background-operation state driving `file_ops` from the TUI. Keeps
//! `main.rs`'s event loop a thin `Action -> App method` dispatcher, the same way
//! `browser::BrowserState` keeps it thin for navigation.
//!
//! Paste and delete run on `tokio`'s blocking thread pool (via `Handle::spawn_blocking`) rather
//! than inline, so a large copy/move/delete never freezes the render loop. Rename and create
//! stay synchronous — both are single, near-instant metadata operations (rename never crosses
//! filesystems: `file_ops::rename` always targets the same parent directory).

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use browser::search::{KEPT, Outcome as SearchOutcome, Query, rank_entries};
use browser::{BrowserState, MovePlan};
use crossterm::event::{KeyCode, KeyEvent};
use file_ops::history::{Change, History, Report, TrashTracker};
use file_ops::{ConflictPolicy, FileOpsError, Outcome};
use plugin_api::{
    ArchiveDone, ArchiveProgress, ArchiveStatus, ArchiveWork, CompressJob, Conflict,
    DevicesCommand, ExtractJob, Format, Level, Password,
};
use plugin_host::protocol::PickerSpec;
use plugin_host::{
    ActionDispatch, ActionRequest, EventKind, JobOutcome, PluginManager, Restart, StateSnapshot,
    Triggers, UiRequest,
};
use shared::{LocalVfs, Vfs, VfsError};
use shell_overlay::CommandOutcome;
use theming::{OpenRule, OpenWith, PluginSpec, PreviewHook};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::associations::{self, Associations};
use crate::atlas::AtlasState;
use crate::cargo_fit::{Carry, Load};
use crate::command::{self, Command};
use crate::compress_popup::{self, CompressPopup};
use crate::deck::DeckState;
use crate::disk_usage::DiskUsageView;
use crate::extract_popup::{self, ExtractPopup};
use crate::inspect_panel::InspectPanel;
use crate::line_edit::LineEdit;
use crate::live_refresh::LiveRefresh;
use crate::marked_size::{MarkedSize, Total};
use crate::nav_job::{NavJob, NavPoll};
use crate::open;
use crate::picker_popup::PickerPopup;
use crate::plugin_views::Slots;
use crate::search_job::{SearchJob, SearchState};
use crate::wiring::WiringState;

/// How many lines of a shell command's output the status line shows before summarizing the rest.
const STATUS_OUTPUT_LINES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardMode {
    Copy,
    Move,
}

#[derive(Debug, Clone)]
pub struct Clipboard {
    pub paths: Vec<PathBuf>,
    pub mode: ClipboardMode,
}

#[derive(Debug)]
pub enum ConflictSource {
    /// `index` is the position in `clip.paths` that hit the conflict; the caller resumes the
    /// batch at `index` (retry) or `index + 1` (skip) once the user answers.
    Paste {
        clip: Clipboard,
        dst_dir: PathBuf,
        index: usize,
        dst: PathBuf,
    },
    Rename {
        target: PathBuf,
        new_name: String,
    },
    /// `:compress` found `out` already there.
    Compress {
        pending: PendingCompress,
        out: PathBuf,
    },
    /// `:extract` found the folder `dest` already there.
    Extract {
        pending: PendingExtract,
        dest: PathBuf,
    },
}

/// Where a `/` search started, so `Esc` (or an emptied query) can put the browser back: the
/// directory it was in and the cursor's index there. A search can carry the browser into another
/// directory, so the index alone is no longer enough.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOrigin {
    pub dir: PathBuf,
    pub index: usize,
}

/// A command waiting for the terminal: `main` suspends the interface, runs `line` under `sh -c`
/// in `cwd`, and hands the result back through `App::finish_handover`. The app can't do that
/// itself because it doesn't own the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handover {
    pub line: String,
    pub cwd: PathBuf,
}

#[derive(Debug)]
pub enum Prompt {
    RenameInput {
        target: PathBuf,
        buffer: LineEdit,
    },
    CreateInput {
        buffer: LineEdit,
    },
    /// The name to save under in `dir`, while another program has asked for a save path (see
    /// `chooser`).
    SaveName {
        dir: PathBuf,
        buffer: LineEdit,
    },
    /// Asks before the file a save would name is replaced.
    ConfirmReplace {
        target: PathBuf,
    },
    /// Confirms a permanent delete of one or more targets — either the single cursor entry, or
    /// every currently marked path if any are marked (Ranger-style: marks win over the cursor).
    ConfirmDelete {
        targets: Vec<PathBuf>,
    },
    /// Confirms sending one or more targets to the desktop trash (same marks-win-over-cursor
    /// batching as `ConfirmDelete`). Reversible, so `Enter` confirms it as well as `y`.
    ConfirmTrash {
        targets: Vec<PathBuf>,
    },
    Conflict(ConflictSource),
    /// Incremental filename search (`/`). `origin` is where to return to on `Esc`.
    SearchInput {
        buffer: LineEdit,
        origin: SearchOrigin,
    },
    /// The `:`-command prompt (`:q`, `:cd <path>`, `:mkdir`, `:touch`, or any shell command).
    CommandInput {
        buffer: LineEdit,
    },
    /// The command typed for the "Open with" menu's "Other…": what should open `path`.
    OpenOther {
        path: PathBuf,
        buffer: LineEdit,
    },
    /// The password for an encrypted archive about to be extracted, shown as dots. `archive` is
    /// the one being asked about, `error` why the last try was not accepted.
    ExtractPassword {
        pending: PendingExtract,
        archive: PathBuf,
        buffer: Password,
        error: Option<String>,
    },
    /// The password to seal `archive` with (`:compress -p`): asked once, then again to confirm.
    /// `first` holds the first answer while the second is being typed.
    CompressPassword {
        pending: PendingCompress,
        archive: PathBuf,
        first: Option<Password>,
        buffer: Password,
        error: Option<String>,
    },
}

impl Prompt {
    /// The status bar's mode label for this prompt.
    pub fn label(&self) -> &'static str {
        match self {
            Prompt::RenameInput { .. } => "RENAME",
            Prompt::CreateInput { .. } => "CREATE",
            Prompt::SaveName { .. } => "SAVE AS",
            Prompt::ConfirmReplace { .. } => "REPLACE",
            Prompt::ConfirmDelete { .. } => "DELETE",
            Prompt::ConfirmTrash { .. } => "TRASH",
            Prompt::Conflict(_) => "CONFLICT",
            Prompt::SearchInput { .. } => "SEARCH",
            Prompt::CommandInput { .. } => "COMMAND",
            Prompt::OpenOther { .. } => "OPEN WITH",
            Prompt::ExtractPassword { .. } | Prompt::CompressPassword { .. } => "PASSWORD",
        }
    }

    /// Whether the user is typing into a buffer (so a cursor belongs at the end of the text),
    /// as opposed to answering a one-key question.
    pub fn is_text_input(&self) -> bool {
        matches!(
            self,
            Prompt::RenameInput { .. }
                | Prompt::CreateInput { .. }
                | Prompt::SaveName { .. }
                | Prompt::SearchInput { .. }
                | Prompt::CommandInput { .. }
                | Prompt::OpenOther { .. }
                | Prompt::ExtractPassword { .. }
                | Prompt::CompressPassword { .. }
        )
    }

    /// Whether confirming this prompt is destructive (permanent delete, overwrite).
    pub fn is_destructive(&self) -> bool {
        matches!(
            self,
            Prompt::ConfirmDelete { .. } | Prompt::ConfirmReplace { .. } | Prompt::Conflict(_)
        )
    }

    /// [`display`](Self::display) with `glyph` drawn where typing will land: inside the text for a
    /// line that has a cursor, at the end for a password (whose dots only ever grow).
    pub fn display_with_cursor(&self, glyph: &str) -> String {
        match self {
            Prompt::RenameInput { buffer, .. } => format!("rename: {}", buffer.render(glyph)),
            Prompt::CreateInput { buffer } => {
                format!(
                    "create (end with / for a directory): {}",
                    buffer.render(glyph)
                )
            }
            Prompt::SaveName { dir, buffer } => {
                format!("save in {} as: {}", display_name(dir), buffer.render(glyph))
            }
            Prompt::SearchInput { buffer, .. } => format!("/{}", buffer.render(glyph)),
            Prompt::CommandInput { buffer } => format!(":{}", buffer.render(glyph)),
            Prompt::OpenOther { path, buffer } => {
                format!("open {} with: {}", display_name(path), buffer.render(glyph))
            }
            other => format!("{}{glyph}", other.display()),
        }
    }

    pub fn display(&self) -> String {
        match self {
            Prompt::RenameInput { buffer, .. } => format!("rename: {buffer}"),
            Prompt::CreateInput { buffer } => {
                format!("create (end with / for a directory): {buffer}")
            }
            Prompt::SaveName { dir, buffer } => {
                format!("save in {} as: {buffer}", display_name(dir))
            }
            Prompt::ConfirmReplace { target } => {
                format!("replace '{}'? (y/N)", display_name(target))
            }
            Prompt::ConfirmDelete { targets } if targets.len() == 1 => {
                format!("delete '{}' permanently? (y/N)", display_name(&targets[0]))
            }
            Prompt::ConfirmDelete { targets } => {
                format!("delete {} marked items permanently? (y/N)", targets.len())
            }
            Prompt::ConfirmTrash { targets } if targets.len() == 1 => {
                format!("trash '{}'? (enter/y)", display_name(&targets[0]))
            }
            Prompt::ConfirmTrash { targets } => {
                format!("trash {} marked items? (enter/y)", targets.len())
            }
            Prompt::Conflict(ConflictSource::Paste { dst, .. }) => format!(
                "'{}' already exists — overwrite / skip / abort? (o/s/a)",
                display_name(dst)
            ),
            Prompt::Conflict(ConflictSource::Rename { new_name, .. }) => {
                format!("'{new_name}' already exists — overwrite / skip / abort? (o/s/a)")
            }
            Prompt::Conflict(ConflictSource::Compress { out, .. }) => format!(
                "'{}' already exists — overwrite / skip / abort? (o/s/a)",
                display_name(out)
            ),
            Prompt::Conflict(ConflictSource::Extract { dest, .. }) => format!(
                "'{}' already exists — replace files / skip existing / abort? (o/s/a)",
                display_name(dest)
            ),
            Prompt::SearchInput { buffer, .. } => format!("/{buffer}"),
            Prompt::CommandInput { buffer } => format!(":{buffer}"),
            Prompt::OpenOther { path, buffer } => {
                format!("open {} with: {buffer}", display_name(path))
            }
            Prompt::CompressPassword {
                archive,
                first,
                buffer,
                error,
                ..
            } => {
                let dots = "•".repeat(buffer.char_count());
                let ask = match first {
                    None => "password",
                    Some(_) => "confirm password",
                };
                match error {
                    Some(error) => {
                        format!("{error} — {ask} for {}: {dots}", display_name(archive))
                    }
                    None => format!("{ask} for {}: {dots}", display_name(archive)),
                }
            }
            Prompt::ExtractPassword {
                archive,
                buffer,
                error,
                ..
            } => {
                let dots = "•".repeat(buffer.char_count());
                match error {
                    Some(error) => {
                        format!("{error} — password for {}: {dots}", display_name(archive))
                    }
                    None => format!("password for {}: {dots}", display_name(archive)),
                }
            }
        }
    }
}

/// What a running background operation is doing, and what it needs to retry with if it turns
/// out to hit a conflict (only `Paste` can — `Delete` never has an `AlreadyExists` case).
///
/// `Paste` processes `clip.paths` one item at a time — `index` is the item currently in flight,
/// `dst` its destination under `dst_dir`. `poll_bulk` advances `index` and re-spawns the next
/// item itself once the current one finishes, so a multi-item batch stays a single continuous
/// "busy" operation from the UI's perspective rather than N separate ones.
#[derive(Debug)]
enum BulkKind {
    Paste {
        clip: Clipboard,
        dst_dir: PathBuf,
        index: usize,
        dst: PathBuf,
        /// `Overwrite` replaced something, which cannot be put back, so it is not recorded.
        policy: ConflictPolicy,
    },
    Delete {
        targets: Vec<PathBuf>,
    },
    Trash {
        targets: Vec<PathBuf>,
    },
    /// Unpacking archives, each into its own directory.
    Extract,
    /// Packing into one archive.
    Compress,
    /// A `:` command line running under `sh -c`, from `started`.
    Shell {
        command: String,
        started: Instant,
    },
}

impl BulkKind {
    fn progressing_label(&self) -> &'static str {
        match self {
            BulkKind::Paste { clip, .. } => match clip.mode {
                ClipboardMode::Copy => "copying",
                ClipboardMode::Move => "moving",
            },
            BulkKind::Delete { .. } => "deleting",
            BulkKind::Trash { .. } => "trashing",
            BulkKind::Extract => "extracting",
            BulkKind::Compress => "compressing",
            BulkKind::Shell { .. } => "running",
        }
    }

    fn past_label(&self) -> &'static str {
        match self {
            BulkKind::Paste { clip, .. } => match clip.mode {
                ClipboardMode::Copy => "copy",
                ClipboardMode::Move => "move",
            },
            BulkKind::Delete { .. } => "delete",
            BulkKind::Trash { .. } => "trash",
            BulkKind::Extract => "extract",
            BulkKind::Compress => "compress",
            BulkKind::Shell { .. } => "command",
        }
    }
}

/// Whether anything is at `path`, a dangling symlink included (`exists` follows links and would
/// call one missing).
/// Sends a file to this machine's trash, or says why it cannot: a remote machine has none.
fn trash_here(path: &Path) -> Result<(), file_ops::FileOpsError> {
    if shared::ssh_path::is_remote(path) {
        return Err(VfsError::Unsupported("a trash on a remote machine").into());
    }
    if vfs_archive::is_inside(path) {
        return Err(VfsError::Unsupported("an archive is read-only").into());
    }
    file_ops::trash(path)
}

fn lexists(vfs: &dyn Vfs, path: &Path) -> bool {
    vfs.symlink_metadata(path).is_ok()
}

/// The request the archive plugin is working on. The plugin reports through the UI queue, so this
/// holds what turns its answers back into the bulk operation the rest of the interface knows.
struct ArchiveRun {
    job: u64,
    /// Where the running bulk operation reads its progress and result.
    tx: UnboundedSender<BulkMsg>,
    /// For each job of the request, the paths to send to the trash once that job has written its
    /// archive (a compress that deletes its originals, an extract that deletes its archive).
    trash_after: Vec<Vec<PathBuf>>,
    /// An extract, kept so a password the plugin asks for can start it again.
    again: Option<PendingExtract>,
    /// Since when the plugin's process has not been running with no result, so a plugin that
    /// died mid-request ends the operation instead of leaving it busy for ever.
    gone_since: Option<Instant>,
}

/// How long the plugin may be gone before its request is given up on. Its last lines may still be
/// on their way to the interface when its process is seen to end.
const ARCHIVER_GRACE: Duration = Duration::from_secs(1);

/// A compress waiting on an answer from the user: whether to replace an archive already there, or
/// the password to seal it with (`:compress -p`).
#[derive(Debug)]
pub struct PendingCompress {
    jobs: Vec<CompressJob>,
    format: Format,
    level: Level,
    /// `Abort` until the user has been asked about an archive already in the way.
    policy: ConflictPolicy,
    delete_originals: bool,
    ask_password: bool,
}

/// An extract waiting for the password its archive needs.
#[derive(Debug)]
pub struct PendingExtract {
    jobs: Vec<ExtractJob>,
    policy: ConflictPolicy,
    delete_archives: bool,
}

enum BulkMsg {
    Progress {
        path: String,
    },
    /// The archive plugin has reached the `done`th entry of its request, called `path`. A total,
    /// not a step, because the plugin thins its messages.
    Entries {
        done: u64,
        path: String,
    },
    /// What a trash batch put in the trash, sent just before its `Done`; `untracked` counts the
    /// items that went there but could not be found again to restore.
    Trashed {
        changes: Vec<Change>,
        untracked: usize,
    },
    Done(Result<Outcome, FileOpsError>),
    ShellDone(std::io::Result<CommandOutcome>),
}

/// A background operation in flight. `cancel` is `None` for `Delete`, since
/// `Vfs::remove_dir_all` is one opaque blocking call with no per-item hook to check against.
/// A `Shell` command is cancelled by killing its shell.
struct BulkOp {
    kind: BulkKind,
    items_done: u64,
    current: String,
    cancel: Option<Arc<AtomicBool>>,
    rx: UnboundedReceiver<BulkMsg>,
}

/// A snapshot of the running background operation, for the header's progress gauge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub label: &'static str,
    /// Items finished so far (files and directories, as `file_ops` reports them).
    pub done: u64,
    /// `(index of the item in flight, total items)` for a multi-item paste; `None` otherwise,
    /// since a single directory's total size isn't known up front.
    pub batch: Option<(usize, usize)>,
}

pub struct App {
    pub clipboard: Option<Clipboard>,
    pub prompt: Option<Prompt>,
    pub status: Option<String>,
    /// Set when another program asked for a pick (`--chooser-file`); see `chooser`.
    pub(crate) chooser: Option<crate::chooser::Chooser>,
    /// The answer to that pick, once the user has given it. The event loop ends on seeing it.
    pub(crate) picked: Option<shared::chooser::Selection>,
    bulk: Option<BulkOp>,
    /// The request the archive plugin is working on, while a compress or extract runs.
    archive_run: Option<ArchiveRun>,
    /// The number the next archive request is sent under, so a late answer to an old one can be
    /// told from the current one.
    next_archive_job: u64,
    /// What can be taken back with `Undo` and done again with `Redo` (see `file_ops::history`).
    history: History,
    /// How many items of the trash batch just run could not be tracked for undo, for the status line.
    untracked: usize,
    live: LiveRefresh,
    /// What the marked entries add up to, for the header pill (see `marked_size`).
    marked_size: MarkedSize,
    /// What the yanked or cut clipboard adds up to, for the deck's fit check. The marks are spent
    /// by a yank, so the two are never the load at once, and the marks' own total is reused.
    clipboard_size: MarkedSize,
    /// What plugins have put in the interface's slots (see `plugin_views`).
    slots: Slots,
    /// Whether the bundled git plugin is started (`git_status` in `config.toml`).
    git_status: bool,
    /// Whether the bundled system plugin is started (`system_hud` in `config.toml`).
    system_hud: bool,
    /// The `[[preview_hook]]` tables, of which the ones that extract text go to the bundled
    /// preview plugin when it starts.
    preview_hooks: Vec<PreviewHook>,
    handle: tokio::runtime::Handle,
    /// Program names a `:` command hands the terminal to (see `command::parse`).
    interactive: Vec<String>,
    /// The `[[open_rule]]` tables: which program opens which files, ahead of the desktop's choice.
    open_rules: Vec<OpenRule>,
    /// The hand-written `[[open_with]]` entries the menu lists.
    open_with: Vec<OpenWith>,
    /// What the desktop has registered for each file type, read on first use rather than at
    /// startup, since most sessions never open a file.
    associations: std::cell::OnceCell<Associations>,
    handover: Option<Handover>,
    /// The `/` prompt's search in flight, if any. Only ever `Some` while that prompt is open.
    search_job: Option<SearchJob>,
    /// A folder being opened in the background (see `nav_job`); while it runs the app is busy and
    /// the browser still shows the old folder.
    nav: Option<NavJob>,
    /// How long `begin_move` waits for the folder before going busy and returning. A local folder
    /// opens well inside it, so ordinary browsing never drops a key pressed right after; only a
    /// slow one (another machine) takes the background route.
    nav_grace: Duration,
    search_state: SearchState,
    /// What the `/` search found, best first, for stepping through with the arrow keys, and which
    /// of them the cursor is on.
    search_matches: Vec<PathBuf>,
    search_position: usize,
    plugins: PluginManager,
    /// Plugin actions run since the last tick, answered once it has published what they left.
    ran_actions: Vec<ActionRequest>,
    /// When `find` was handed to its plugin and the picker has not opened yet, so the loop wakes
    /// often enough to open it the moment the plugin asks.
    finding_since: Option<Instant>,
    /// The filesystem being browsed. Every feature reaches a browsed path through this, and the
    /// background jobs take a clone, so a backend other than the local disk needs no change here.
    vfs: Arc<dyn Vfs>,
    /// Whether the last key event carried the keyboard protocol's Caps Lock flag, for the header
    /// pill (see `main`'s `with_caps_lock_applied`, which reads the same flag to fix up a
    /// letter's case). Only ever known while the terminal reports it; stays `false` otherwise.
    caps_lock: bool,
}

impl App {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        let vfs: Arc<dyn Vfs> = Arc::new(LocalVfs);
        Self {
            clipboard: None,
            prompt: None,
            status: None,
            chooser: None,
            picked: None,
            bulk: None,
            archive_run: None,
            next_archive_job: 1,
            history: History::default(),
            untracked: 0,
            live: LiveRefresh::new(handle.clone(), Arc::clone(&vfs)),
            marked_size: MarkedSize::new(handle.clone(), Arc::clone(&vfs)),
            clipboard_size: MarkedSize::new(handle.clone(), Arc::clone(&vfs)),
            slots: Slots::default(),
            git_status: true,
            system_hud: false,
            preview_hooks: Vec::new(),
            handle,
            interactive: Vec::new(),
            open_rules: Vec::new(),
            open_with: Vec::new(),
            associations: std::cell::OnceCell::new(),
            handover: None,
            search_job: None,
            nav: None,
            nav_grace: Duration::from_millis(40),
            search_state: SearchState::Idle,
            search_matches: Vec::new(),
            search_position: 0,
            plugins: {
                let mut plugins = PluginManager::new();
                plugins.reserve_commands(command::BUILTIN_WORDS.iter().map(ToString::to_string));
                plugins.declare_actions(crate::plugin_action::runnable_names());
                plugins
            },
            ran_actions: Vec::new(),
            finding_since: None,
            vfs,
            caps_lock: false,
        }
    }

    /// Opens the disk usage view on `path`, starting its scan on the blocking pool.
    /// Opens the trash browser, which starts reading the desktop trash at once.
    pub fn begin_trash_view(&self) -> crate::trash_view::TrashView {
        crate::trash_view::TrashView::open(
            self.handle.clone(),
            Arc::new(crate::trash_view::SystemTrash),
        )
    }

    pub fn begin_disk_usage(&self, path: PathBuf) -> DiskUsageView {
        DiskUsageView::open(self.handle.clone(), Arc::clone(&self.vfs), path)
    }

    /// The deck of device cards (`V`, the deck view). The plugin that provides `devices` looks at
    /// the machine only while it exists; with none to ask, the rail says so and the status line
    /// says where to get it.
    pub fn begin_deck(&mut self) -> DeckState {
        let mut deck = DeckState::new();
        if !self
            .plugins
            .dispatch_devices(DevicesCommand::Watch { on: true })
        {
            deck.set_missing(Some("mman-devices was not found".into()));
            self.status = Some(crate::bundled::missing("devices"));
        }
        deck
    }

    /// Tells the devices plugin the rail has gone, so it stops looking at the machine.
    pub fn end_deck(&mut self) {
        let _ = self
            .plugins
            .dispatch_devices(DevicesCommand::Watch { on: false });
    }

    /// Whether the devices plugin has a live process right now.
    pub fn devices_running(&self) -> bool {
        self.plugins.devices_running()
    }

    /// Asks the devices plugin to start watching again after it stopped. `false` when it cannot
    /// be started.
    pub fn restart_devices(&mut self) -> bool {
        self.plugins
            .dispatch_devices(DevicesCommand::Watch { on: true })
    }

    /// Asks the devices plugin to do `command` to a device, or says where to get the plugin.
    pub fn ask_devices(&mut self, command: DevicesCommand) {
        if !self.plugins.dispatch_devices(command) {
            self.status = Some(crate::bundled::missing("devices"));
        }
    }

    /// The wiring diagram of the machine's storage (`V`, the wiring view). It looks at the machine
    /// only while it exists.
    pub fn begin_wiring(&self, show_hidden: bool) -> WiringState {
        WiringState::new(self.handle.clone(), Arc::clone(&self.vfs), show_hidden)
    }

    /// The map of the machine's storage (`V`, the atlas view). It looks at the machine only while
    /// it exists.
    pub fn begin_atlas(&self) -> AtlasState {
        AtlasState::new(self.handle.clone(), Arc::clone(&self.vfs))
    }

    /// Hands the `find` action to the plugin that provides it (the fuzzy finder, which ships
    /// bundled): it walks everything below the browsed directory, hidden entries only if the
    /// browser is showing them, and opens its picker when it has something to show (see
    /// `take_ui_request`). With no such plugin the status bar says how to get one.
    pub fn begin_find(&mut self, browser: &BrowserState) {
        self.begin_picker_action(theming::Action::Find, "the finder", browser);
    }

    /// Hands the `locate` action to the plugin that provides it (the structured search, which
    /// ships bundled): the same walk and picker as `find`, matched by glob, regular expression and
    /// filters instead of by fuzzy score.
    pub fn begin_locate(&mut self, browser: &BrowserState) {
        self.begin_picker_action(theming::Action::Locate, "the search", browser);
    }

    /// Sends `action` to its provider and waits for the picker it will open, which `what` names in
    /// the status bar meanwhile.
    fn begin_picker_action(&mut self, action: theming::Action, what: &str, browser: &BrowserState) {
        // Asked a moment ago and not open yet: a second press must not open a second one.
        if self.awaiting_picker() {
            return;
        }
        let name = action.name();
        match self
            .plugins
            .dispatch_action(name, browser.current_dir(), &[])
        {
            ActionDispatch::Sent => {
                self.status = Some(format!("opening {what}…"));
                self.finding_since = Some(Instant::now());
            }
            ActionDispatch::NoProvider => self.status = Some(crate::bundled::missing(name)),
            // The plugin's own log line says why, and `poll_plugins` puts it in the status bar.
            ActionDispatch::Unavailable => {}
        }
    }

    /// The picker `plugin` asked for with `spec`.
    pub fn begin_picker(&mut self, plugin: &str, spec: &PickerSpec) -> PickerPopup {
        self.finding_since = None;
        PickerPopup::open(&self.handle, Arc::clone(&self.vfs), plugin, spec)
    }

    /// Whether a picker has been asked for and is expected any moment (for a couple of seconds at
    /// most, since a plugin that never answers should not keep the loop busy).
    pub fn awaiting_picker(&self) -> bool {
        self.finding_since
            .is_some_and(|since| since.elapsed() < Duration::from_secs(2))
    }

    /// The oldest thing a plugin asked of its picker that is waiting for this turn, if any.
    pub fn take_ui_request(&mut self) -> Option<UiRequest> {
        self.plugins.take_ui()
    }

    /// Whether a plugin is configured to preview files. Without one the preview column shows only
    /// the file's name.
    pub fn has_previewer(&self) -> bool {
        self.plugins.has_previewer()
    }

    /// Whether the previewing plugin has a live process right now.
    pub fn previewer_running(&self) -> bool {
        self.plugins.previewer_running()
    }

    /// Tells the previewing plugin which file the preview column is about, or that it is about
    /// none. Returns whether the plugin got it; `false` means no document will come.
    pub fn tell_previewer(&mut self, ask: &crate::text_preview::PreviewAsk) -> bool {
        self.plugins.dispatch_preview(ask.0.as_deref())
    }

    /// Passes what was typed into `popup` on to the plugin that owns it.
    pub fn send_picker_query(&self, popup: &mut PickerPopup) {
        if let Some((generation, text)) = popup.take_query() {
            self.plugins
                .send_picker_query(popup.plugin(), popup.picker(), generation, &text);
        }
    }

    /// Tells the plugin that owns `popup` that it was closed, so it can stop its work.
    pub fn picker_closed(&self, popup: &PickerPopup) {
        self.plugins
            .send_picker_closed(popup.plugin(), popup.picker());
    }

    /// Whether the plugin that owns `popup` is still there to answer it.
    pub fn picker_alive(&self, popup: &PickerPopup) -> bool {
        self.plugins.is_running(popup.plugin())
    }

    /// Browses `vfs` instead of the local disk: the background jobs, previews and scans started by
    /// this app all read through it.
    pub fn with_vfs(mut self, vfs: Arc<dyn Vfs>) -> Self {
        self.plugins.set_vfs(Arc::clone(&vfs));
        self.live = LiveRefresh::new(self.handle.clone(), Arc::clone(&vfs));
        self.marked_size = MarkedSize::new(self.handle.clone(), Arc::clone(&vfs));
        self.clipboard_size = MarkedSize::new(self.handle.clone(), Arc::clone(&vfs));
        self.vfs = vfs;
        self
    }

    /// What to tell the user when a remote folder would not open: where to get the plugin when
    /// none is running (the error alone starts with a long address and would hide it), the error
    /// otherwise.
    pub fn remote_open_failure(&self, error: &shared::VfsError) -> String {
        if self.plugins.serves_ssh() {
            format!("cannot open the remote folder: {error}")
        } else {
            crate::bundled::missing(plugin_api::PROVIDES_VFS_SSH)
        }
    }

    /// What to tell the user when an archive would not open: where to get the plugin when none is
    /// running (the error alone starts with a long path and would hide it), the error otherwise.
    pub fn archive_open_failure(&self, error: &shared::VfsError) -> String {
        if self.plugins.serves_archives() {
            format!("cannot open the archive: {error}")
        } else {
            crate::bundled::missing(plugin_api::PROVIDES_VFS_ARCHIVE)
        }
    }

    /// Uses `link` to reach the plugin that serves the paths inside archives: the one the `Vfs`
    /// given to `with_vfs` was built with. Call before `with_plugins`.
    pub fn with_archive_link(mut self, link: Arc<plugin_host::ServeLink>) -> Self {
        self.plugins.set_serve_link(link);
        self
    }

    /// Uses `link` to reach the plugin that serves the paths on remote machines: the one the `Vfs`
    /// given to `with_vfs` was built with. Call before `with_plugins`.
    pub fn with_ssh_link(mut self, link: Arc<plugin_host::ServeLink>) -> Self {
        self.plugins.set_serve_ssh_link(link);
        self
    }

    /// Turns the status bar's git segment on or off (`git_status` in `config.toml`): off means the
    /// bundled git plugin is not started. Call before `with_plugins`.
    pub fn with_git_status(mut self, enabled: bool) -> Self {
        self.git_status = enabled;
        self
    }

    /// Starts the bundled system plugin, whose figures fill the header pill (`system_hud` in
    /// `config.toml`).
    pub fn with_system_hud(mut self, enabled: bool) -> Self {
        self.system_hud = enabled;
        self
    }

    /// Hands the `[[preview_hook]]` tables to the bundled preview plugin, which runs the ones
    /// that extract text. Call before `with_plugins`.
    pub fn with_preview_hooks(mut self, hooks: Vec<PreviewHook>) -> Self {
        self.preview_hooks = hooks;
        self
    }

    /// Sets which program names `:` hands the terminal to.
    pub fn with_interactive_commands(mut self, interactive: Vec<String>) -> Self {
        self.interactive = interactive;
        self
    }

    /// Sets the `[[open_rule]]` and `[[open_with]]` tables.
    pub fn with_open_config(mut self, rules: Vec<OpenRule>, open_with: Vec<OpenWith>) -> Self {
        self.open_rules = rules;
        self.open_with = open_with;
        self
    }

    /// Starts every configured `[[plugin]]` process (see `plugins`), each firing its `init`
    /// event immediately, its `key` event whenever one of its `on_key` keys is pressed and its
    /// `command` event for its `commands` words or `:plugin <name>`. A plugin that
    /// fails to spawn (bad command, missing binary) is skipped with a warning on stderr rather
    /// than treated as fatal — the same "one bad entry must never take the whole session down"
    /// rule `theming::Config::load` already follows for a malformed config file.
    pub fn with_plugins(mut self, specs: Vec<PluginSpec>, cwd: &Path) -> Self {
        let bundled = crate::bundled::specs(
            &specs,
            self.git_status,
            self.system_hud,
            &self.preview_hooks,
        );
        // Without a previewer the column shows only names; say why instead of leaving it a mystery.
        if let Some(notice) = crate::bundled::missing_previewer(&specs, &bundled) {
            self.status = Some(notice);
        }
        for spec in specs.into_iter().chain(bundled) {
            if let Err(e) = self.plugins.spawn(
                &self.handle,
                &spec.name,
                &spec.command,
                &spec.args,
                Triggers {
                    keys: spec.on_keys,
                    commands: spec.commands,
                    actions: spec.provides,
                    events: spec
                        .events
                        .iter()
                        .filter_map(|name| {
                            let kind = EventKind::parse(name);
                            if kind.is_none() {
                                eprintln!(
                                    "minuteman: plugin {}: unknown event '{name}'",
                                    spec.name
                                );
                            }
                            kind
                        })
                        .collect(),
                },
                cwd,
            ) {
                eprintln!("minuteman: {e}");
            }
        }
        self
    }

    #[cfg(test)]
    pub fn status_line(&self) -> String {
        self.status_line_with(None)
    }

    /// The status line, with a text prompt's cursor drawn (as `cursor`) where typing will land.
    pub fn status_line_with(&self, cursor: Option<&str>) -> String {
        if let Some(bulk) = &self.bulk {
            return match &bulk.kind {
                BulkKind::Delete { targets } | BulkKind::Trash { targets } => {
                    format!(
                        "{}… {}",
                        bulk.kind.progressing_label(),
                        batch_label(targets)
                    )
                }
                BulkKind::Extract | BulkKind::Compress => format!(
                    "{}… {} — Esc to cancel",
                    bulk.kind.progressing_label(),
                    bulk.current
                ),
                BulkKind::Shell { command, .. } => {
                    format!(
                        "{}… {command} — Esc to cancel",
                        bulk.kind.progressing_label()
                    )
                }
                BulkKind::Paste { clip, index, .. } => {
                    let cancel_hint = if bulk.cancel.is_some() {
                        " — Esc to cancel"
                    } else {
                        ""
                    };
                    let batch_hint = if clip.paths.len() > 1 {
                        format!(" [{}/{}]", index + 1, clip.paths.len())
                    } else {
                        String::new()
                    };
                    format!(
                        "{}… {} done ({}){batch_hint}{cancel_hint}",
                        bulk.kind.progressing_label(),
                        bulk.items_done,
                        bulk.current
                    )
                }
            };
        }
        let shown = |prompt: &Prompt| match cursor {
            Some(glyph) if prompt.is_text_input() => prompt.display_with_cursor(glyph),
            _ => prompt.display(),
        };
        match &self.prompt {
            Some(prompt @ Prompt::SearchInput { .. }) => match self.search_state.note() {
                Some(note) => format!("{note} {}", shown(prompt)),
                None => shown(prompt),
            },
            Some(prompt) => shown(prompt),
            None => self.status.clone().unwrap_or_default(),
        }
    }

    pub fn is_busy(&self) -> bool {
        self.bulk.is_some() || self.nav.is_some()
    }

    /// Starts opening the folder `plan` describes, without waiting for it: the old folder stays on
    /// screen and keys are held back (`is_busy`) until `poll_nav` applies the result. `done_status`
    /// is shown on success and `error_prefix` leads the message on failure. Ignored while another
    /// move is still loading.
    pub fn begin_move(
        &mut self,
        browser: &mut BrowserState,
        plan: MovePlan,
        done_status: Option<String>,
        error_prefix: &str,
    ) {
        if self.nav.is_some() {
            return;
        }
        let job = NavJob::start(
            &self.handle,
            Arc::clone(&self.vfs),
            plan,
            done_status,
            error_prefix,
        );
        self.status = Some(format!("opening {}…", job.label));
        self.nav = Some(job);
        let deadline = Instant::now() + self.nav_grace;
        while self.nav.is_some() && Instant::now() < deadline {
            self.poll_nav(browser);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Applies a finished folder load to `browser`, or reports why it failed. Call once per render
    /// tick.
    pub fn poll_nav(&mut self, browser: &mut BrowserState) {
        let Some(job) = self.nav.as_mut() else {
            return;
        };
        let result = match job.poll() {
            NavPoll::Pending => return,
            NavPoll::Done(result) => result,
            NavPoll::Lost => {
                self.nav = None;
                self.status = Some("could not open the folder".into());
                return;
            }
        };
        let Some(job) = self.nav.take() else {
            return;
        };
        match result {
            Ok(loaded) => {
                browser.apply_move(loaded);
                self.status = job.done_status;
            }
            Err(e) if vfs_remote::is_plugin_absent(&e) => {
                self.status = Some(self.remote_open_failure(&e));
            }
            Err(e) => self.status = Some(format!("{}: {e}", job.error_prefix)),
        }
    }

    pub fn progress(&self) -> Option<Progress> {
        if let (None, Some(_)) = (&self.bulk, &self.nav) {
            return Some(Progress {
                label: "opening",
                done: 0,
                batch: None,
            });
        }
        let bulk = self.bulk.as_ref()?;
        let batch = match &bulk.kind {
            BulkKind::Paste { clip, index, .. } if clip.paths.len() > 1 => {
                Some((*index, clip.paths.len()))
            }
            _ => None,
        };
        // A command has no item count, so its pill counts elapsed seconds instead — which also
        // keeps the gauge moving while it runs.
        let done = match &bulk.kind {
            BulkKind::Shell { started, .. } => started.elapsed().as_secs(),
            _ => bulk.items_done,
        };
        Some(Progress {
            label: bulk.kind.progressing_label(),
            done,
            batch,
        })
    }

    /// Keeps the browser's lists in step with changes made on disk behind its back (see
    /// `live_refresh`). Call once per render tick; returns whether the selected file itself
    /// changed, so its preview needs re-reading.
    pub fn poll_disk(&mut self, browser: &mut BrowserState, now: Instant) -> bool {
        self.live.poll(browser, now)
    }

    /// Keeps the figures the HUD shows about the browsed directory and the marks up to date: the
    /// marked entries' total size. It runs its work on the blocking
    /// pool (see `marked_size`); call once per render tick.
    pub fn poll_hud(&mut self, browser: &BrowserState) {
        self.marked_size.poll(&browser.marked_paths());
        let clipped: Vec<PathBuf> = self
            .clipboard
            .as_ref()
            .map(|clip| clip.paths.clone())
            .unwrap_or_default();
        self.clipboard_size.poll(&clipped);
    }

    /// Surfaces every plugin's pending `log` line as the status message (last one wins, the same
    /// way every other status write already behaves), publishes the browser's state for
    /// plugins to read with `get_state`, and then answers the actions run since the last call; `view` is the view's name, which lives outside `App`.
    /// Call once per render tick.
    pub fn poll_plugins(&mut self, browser: &BrowserState, view: &str) {
        if !self.plugins.is_empty() {
            let sort = browser.sort();
            let mut marks = browser.marked_paths();
            // A set has no order of its own; sorting keeps an unchanged set from looking changed.
            marks.sort();
            self.plugins.publish_state(StateSnapshot {
                cwd: browser.current_dir().to_path_buf(),
                cursor: browser.selected_entry().map(|entry| entry.path.clone()),
                marks,
                show_hidden: browser.show_hidden(),
                sort: sort.key.label().to_string(),
                sort_reversed: sort.reversed,
                view: view.to_string(),
            });
        }
        for request in self.ran_actions.drain(..) {
            request.ran();
        }
        for message in self.plugins.poll() {
            self.status = Some(format!("[{}] {}", message.plugin, message.message));
        }
    }

    /// The oldest action a plugin asked to run, if any (see `plugin_action::accept`).
    pub fn take_plugin_action(&mut self) -> Option<ActionRequest> {
        self.plugins.take_action()
    }

    /// Holds `request`, whose action has just run, until the next `poll_plugins` has published
    /// the state it left: a plugin that reads `get_state` after the reply then sees that state.
    pub fn ran_plugin_action(&mut self, request: ActionRequest) {
        self.ran_actions.push(request);
    }

    /// Fires the plugin bound to `code`, if any is configured, with the current marks (or the
    /// single selected entry — see `marked_or_selected`) and the browsed directory. Returns
    /// whether a plugin consumed the key, so a truly unbound key stays a no-op.
    pub fn dispatch_plugin_key(&mut self, code: KeyCode, browser: &BrowserState) -> bool {
        let selection = Self::marked_or_selected(browser);
        self.plugins
            .dispatch_key(code, browser.current_dir(), &selection)
    }

    /// What the marks add up to, once a walk has finished.
    pub fn marked_total(&self) -> Option<Total> {
        self.marked_size.total()
    }

    /// What the user is carrying: the yanked or cut files if there are any, else the marked ones,
    /// sized and ready for the deck to hold up against each device. `None` when nothing is held
    /// or the sizing walk has not finished.
    pub fn load(&self, browser: &BrowserState) -> Option<Load> {
        let (measured, carry, source) = match &self.clipboard {
            Some(clip) => (
                self.clipboard_size.measured()?,
                match clip.mode {
                    ClipboardMode::Copy => Carry::Copy,
                    ClipboardMode::Move => Carry::Move,
                },
                clip.paths.first()?.clone(),
            ),
            None => (
                self.marked_size.measured()?,
                Carry::Copy,
                browser.marked_paths().into_iter().next()?,
            ),
        };
        Some(Load {
            bytes: measured.bytes,
            exact: measured.exact,
            largest: measured.largest,
            carry,
            source,
        })
    }

    /// Applies a plugin's `set_view` to the slot it names.
    pub fn set_view(&mut self, plugin: &str, set: plugin_host::SetView) {
        self.slots.apply(plugin, set);
    }

    /// What fills `slot`, if a plugin has filled it.
    pub fn slot_view(&self, slot: plugin_host::Slot) -> Option<&plugin_host::protocol::View> {
        self.slots.view(slot)
    }

    /// The badge a plugin gave the entry called `name` in the current folder.
    pub fn entry_badge(&self, name: &str) -> Option<&plugin_host::protocol::Line> {
        self.slots.badge(name)
    }

    /// Whether any entry has a badge, so the list reserves room for them only when one shows.
    pub fn has_badges(&self) -> bool {
        self.slots.has_badges()
    }

    /// Drops what plugins that have stopped had put in the slots, so a dead plugin's text never
    /// lingers. Call once per render tick.
    pub fn prune_views(&mut self) {
        let plugins = &self.plugins;
        self.slots.retain_running(|name| plugins.is_running(name));
    }

    /// Records whether the key event just read carried the keyboard protocol's Caps Lock flag.
    /// Call on every key event (not just the ones that reach a binding), so the header pill
    /// stays correct even while a modal view or prompt owns the keyboard.
    pub fn set_caps_lock(&mut self, on: bool) {
        self.caps_lock = on;
    }

    /// Whether Caps Lock is currently on, for the header pill.
    pub fn caps_lock(&self) -> bool {
        self.caps_lock
    }

    /// Drains any progress/completion messages from the running background operation, if any.
    /// Call once per render tick.
    pub fn poll_bulk(&mut self, browser: &mut BrowserState, vfs: &dyn Vfs) -> Result<()> {
        let Some(bulk) = self.bulk.as_mut() else {
            return Ok(());
        };

        let mut done = None;
        let mut shell_done = None;
        let mut trashed = Vec::new();
        while let Ok(msg) = bulk.rx.try_recv() {
            match msg {
                BulkMsg::Progress { path } => {
                    bulk.items_done += 1;
                    bulk.current = path;
                }
                BulkMsg::Entries { done, path } => {
                    bulk.items_done = done;
                    bulk.current = path;
                }
                BulkMsg::Trashed { changes, untracked } => {
                    trashed.extend(changes);
                    self.untracked += untracked;
                }
                BulkMsg::Done(result) => done = Some(result),
                BulkMsg::ShellDone(result) => shell_done = Some(result),
            }
        }

        for change in trashed {
            self.history.record(change);
        }

        if let Some(result) = shell_done {
            let bulk = self.bulk.take().expect("checked Some above");
            // Whatever the command did to the directory, show it now rather than on the next
            // background refresh; it may also have deleted marked paths.
            browser.reload(vfs)?;
            browser.prune_marks(vfs);
            if let BulkKind::Shell { command, .. } = bulk.kind {
                self.plugins.notify_job_finished(
                    "command",
                    if result.is_ok() {
                        JobOutcome::Completed
                    } else {
                        JobOutcome::Failed
                    },
                );
                self.status = Some(match result {
                    Ok(outcome) => shell_status(&command, &outcome),
                    Err(e) => format!("could not run '{command}': {e}"),
                });
            }
            return Ok(());
        }

        let Some(result) = done else {
            return Ok(());
        };
        let bulk = self.bulk.take().expect("checked Some above");
        let past_label = bulk.kind.past_label();
        let overwrote = matches!(
            &bulk.kind,
            BulkKind::Paste {
                policy: ConflictPolicy::Overwrite,
                ..
            }
        );
        if let (
            Ok(Outcome::Completed),
            BulkKind::Paste {
                clip,
                index,
                dst,
                policy,
                ..
            },
        ) = (&result, &bulk.kind)
        {
            self.record_paste(clip, *index, dst, *policy);
        }

        // Delete and Trash both remove entries from the listing outright (no conflict case,
        // unlike Paste), so they share every branch below that Paste doesn't.
        // A compress can also remove entries (it may trash the originals), so it is treated the
        // same way to keep the listing and the marks in step.
        let is_delete = matches!(
            bulk.kind,
            BulkKind::Delete { .. } | BulkKind::Trash { .. } | BulkKind::Compress
        );
        // Each item of a batch gets its own cancel flag, so a cancel that lands just as an item
        // finishes would otherwise be forgotten and the batch would carry on with the next one.
        let cancelled = bulk
            .cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed));

        match result {
            Ok(Outcome::Completed | Outcome::Skipped) if cancelled => {
                browser.reload(vfs)?;
                self.plugins
                    .notify_job_finished(past_label, JobOutcome::Cancelled);
                self.status = Some(format!("{past_label} cancelled"));
            }
            Ok(Outcome::Completed) => {
                browser.reload(vfs)?;
                if let BulkKind::Paste {
                    clip,
                    dst_dir,
                    index,
                    ..
                } = bulk.kind
                {
                    let Some(clip) = self.try_continue_paste(clip, dst_dir, index) else {
                        return Ok(());
                    };
                    // Only the cut this paste came from: a drop moves files without ever
                    // touching the clipboard, and must not empty one the user filled meanwhile.
                    if clip.mode == ClipboardMode::Move
                        && self
                            .clipboard
                            .as_ref()
                            .is_some_and(|held| held.paths == clip.paths)
                    {
                        self.clipboard = None;
                    }
                    if clip.mode == ClipboardMode::Move {
                        browser.prune_marks(vfs);
                    }
                } else if is_delete {
                    browser.prune_marks(vfs);
                }
                let caveat = match (overwrote, std::mem::take(&mut self.untracked)) {
                    (true, _) => " (an overwrite cannot be undone)".to_string(),
                    (false, 0) => String::new(),
                    (false, n) => format!(" ({n} could not be tracked, so not undoable)"),
                };
                self.plugins
                    .notify_job_finished(past_label, JobOutcome::Completed);
                self.status = Some(format!("{past_label} complete{caveat}"));
            }
            Ok(Outcome::Skipped) => {
                if let BulkKind::Paste {
                    clip,
                    dst_dir,
                    index,
                    ..
                } = bulk.kind
                    && self.try_continue_paste(clip, dst_dir, index).is_none()
                {
                    return Ok(());
                }
                self.plugins
                    .notify_job_finished(past_label, JobOutcome::Skipped);
                self.status = Some(format!("{past_label} skipped"));
            }
            Err(FileOpsError::Cancelled) => {
                self.plugins
                    .notify_job_finished(past_label, JobOutcome::Cancelled);
                self.status = Some(format!("{past_label} cancelled"));
            }
            Err(FileOpsError::Vfs(VfsError::AlreadyExists(existing))) => match bulk.kind {
                BulkKind::Paste {
                    clip,
                    dst_dir,
                    index,
                    dst,
                    ..
                } => {
                    self.prompt = Some(Prompt::Conflict(ConflictSource::Paste {
                        clip,
                        dst_dir,
                        index,
                        dst,
                    }));
                }
                // No prompt for these: overwriting could clobber a whole extracted tree, so the
                // user is told what is in the way and can remove or rename it.
                BulkKind::Extract | BulkKind::Compress => {
                    self.plugins
                        .notify_job_finished(past_label, JobOutcome::Failed);
                    self.status = Some(format!(
                        "{past_label} failed: {} already exists",
                        existing.display()
                    ));
                }
                BulkKind::Delete { .. } | BulkKind::Trash { .. } | BulkKind::Shell { .. } => {
                    self.plugins
                        .notify_job_finished(past_label, JobOutcome::Failed);
                    self.status = Some(format!("{past_label} failed: unexpected conflict"));
                }
            },
            // A delete can fail partway through a multi-target batch, having already removed
            // some marked paths from disk — reload/prune so the browser reflects that instead
            // of showing stale entries and stale marks alongside the failure message.
            Err(e) => {
                if is_delete {
                    browser.reload(vfs)?;
                    browser.prune_marks(vfs);
                }
                self.plugins
                    .notify_job_finished(past_label, JobOutcome::Failed);
                self.status = Some(format!("{past_label} failed: {e}"));
            }
        }
        Ok(())
    }

    /// Requests cancellation of the running background operation, if it supports it.
    pub fn cancel_bulk(&mut self) {
        if self.nav.take().is_some() {
            self.status = Some("cancelled".into());
            return;
        }
        match self.bulk.as_ref().and_then(|b| b.cancel.as_ref()) {
            Some(cancel) => {
                cancel.store(true, Ordering::Relaxed);
                self.signal_archive_cancel();
                self.status = Some("cancelling…".into());
            }
            None => self.status = Some("nothing cancellable in progress".into()),
        }
    }

    /// Cancels everything pending in one go, from any directory: the yanked or cut clipboard,
    /// every mark, and a running copy/move/command. A cut only takes effect when pasted, so
    /// forgetting one never touches disk. A running delete can't be interrupted (see `BulkOp`),
    /// and the status line says so rather than pretending.
    pub fn cancel_all(&mut self, browser: &mut BrowserState) {
        let mut cancelled = Vec::new();
        let mut uncancellable = false;
        if self.nav.take().is_some() {
            cancelled.push("opening a folder".to_string());
        }
        if let Some(bulk) = &self.bulk {
            match &bulk.cancel {
                Some(flag) => {
                    flag.store(true, Ordering::Relaxed);
                    self.signal_archive_cancel();
                    cancelled.push(format!("running {}", bulk.kind.past_label()));
                }
                None => uncancellable = true,
            }
        }
        if let Some(clip) = self.clipboard.take() {
            let verb = match clip.mode {
                ClipboardMode::Copy => "yank",
                ClipboardMode::Move => "cut",
            };
            cancelled.push(format!("{verb} of {}", batch_label(&clip.paths)));
        }
        match browser.clear_marks() {
            0 => {}
            1 => cancelled.push("1 mark".into()),
            marks => cancelled.push(format!("{marks} marks")),
        }

        self.status = Some(match (cancelled.is_empty(), uncancellable) {
            (true, false) => "nothing to cancel".into(),
            (true, true) => "a running delete cannot be cancelled".into(),
            (false, false) => format!("cancelled: {}", cancelled.join(", ")),
            (false, true) => format!(
                "cancelled: {} (a running delete cannot be)",
                cancelled.join(", ")
            ),
        });
    }

    /// Snapshots the current batch: every marked path if any are marked, otherwise just the
    /// entry under the cursor. Ranger-style "act on marks if any, else the current file"
    /// convention, shared by the trash and delete keys and by the cursor fallback of yank and cut.
    fn marked_or_selected(browser: &BrowserState) -> Vec<PathBuf> {
        match browser.marked_paths() {
            marked if !marked.is_empty() => marked,
            _ => browser
                .selected_entry()
                .map(|entry| vec![entry.path.clone()])
                .unwrap_or_default(),
        }
    }

    /// What compress and extract act on: the entries marked with `Select` (`v`), and nothing
    /// else. The cursor row is not a stand-in for them, unlike for `y`, `c`, `x` and `X`.
    fn marked_only(browser: &BrowserState) -> Vec<PathBuf> {
        browser.marked_paths()
    }

    /// Whether anything is marked with `Select`. The compress and extract keys act on marks only,
    /// so with none this reports it on the status line and returns `false`.
    pub fn require_marks(&mut self, browser: &BrowserState) -> bool {
        let any = !browser.marked_paths().is_empty();
        if !any {
            self.status = Some(NOTHING_MARKED.into());
        }
        any
    }

    /// Copies the marked entries to the clipboard. The marks are spent: `y` takes what `v` chose,
    /// leaving nothing selected, so selecting stays a separate step from acting.
    pub fn yank(&mut self, browser: &mut BrowserState) {
        self.carry(browser, ClipboardMode::Copy);
    }

    /// Like [`yank`](Self::yank), but the entries are moved when pasted.
    pub fn cut(&mut self, browser: &mut BrowserState) {
        self.carry(browser, ClipboardMode::Move);
    }

    fn carry(&mut self, browser: &mut BrowserState, mode: ClipboardMode) {
        let paths = Self::marked_only(browser);
        if paths.is_empty() {
            self.status = Some(NOTHING_MARKED.into());
            return;
        }
        browser.clear_marks();
        self.stage(paths, mode);
    }

    fn stage(&mut self, paths: Vec<PathBuf>, mode: ClipboardMode) {
        self.status = Some(match mode {
            ClipboardMode::Copy => format!("yanked {}", batch_label(&paths)),
            ClipboardMode::Move => format!("cut {} to move", batch_label(&paths)),
        });
        self.clipboard = Some(Clipboard { paths, mode });
    }

    /// The `y` key. With entries marked it copies them into the browsed folder at once, so the
    /// order is: mark, go where they should end up, press the key. With none marked it takes the
    /// entry under the cursor onto the clipboard, for `p` to drop where wanted.
    pub fn yank_key(&mut self, browser: &mut BrowserState) {
        self.carry_key(browser, ClipboardMode::Copy);
    }

    /// The `c` key: [`yank_key`](Self::yank_key), but the entries are moved rather than copied.
    pub fn cut_key(&mut self, browser: &mut BrowserState) {
        self.carry_key(browser, ClipboardMode::Move);
    }

    fn carry_key(&mut self, browser: &mut BrowserState, mode: ClipboardMode) {
        let marked = Self::marked_only(browser);
        if marked.is_empty() {
            match browser.selected_entry() {
                Some(entry) => {
                    let paths = vec![entry.path.clone()];
                    self.stage(paths, mode);
                }
                None => self.status = Some(NOTHING_HERE.into()),
            }
            return;
        }
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        let dst = browser.current_dir().to_path_buf();
        // Moving a file to the folder it is already in would only raise a conflict with itself.
        let paths: Vec<PathBuf> = match mode {
            ClipboardMode::Move => marked
                .into_iter()
                .filter(|path| path.parent() != Some(dst.as_path()))
                .collect(),
            ClipboardMode::Copy => marked,
        };
        if paths.is_empty() {
            self.status = Some("already in this folder".into());
            return;
        }
        browser.clear_marks();
        self.begin_drop(paths, mode, dst);
    }

    /// Asks to send the marked entries to the desktop trash, or the entry under the cursor when
    /// none is marked.
    pub fn begin_trash(&mut self, browser: &BrowserState) {
        let targets = Self::marked_or_selected(browser);
        if targets.is_empty() {
            self.status = Some(NOTHING_HERE.into());
            return;
        }
        self.prompt = Some(Prompt::ConfirmTrash { targets });
    }

    /// Same batching as `begin_trash`, but for a permanent, trash-bypassing delete.
    pub fn begin_delete_permanently(&mut self, browser: &BrowserState) {
        let targets = Self::marked_or_selected(browser);
        if targets.is_empty() {
            self.status = Some(NOTHING_HERE.into());
            return;
        }
        self.prompt = Some(Prompt::ConfirmDelete { targets });
    }

    pub fn begin_rename(&mut self, browser: &BrowserState) {
        match browser.selected_entry() {
            Some(entry) => {
                self.prompt = Some(Prompt::RenameInput {
                    target: entry.path.clone(),
                    buffer: LineEdit::new(entry.name.clone()),
                });
            }
            None => self.status = Some("nothing selected".into()),
        }
    }

    pub fn begin_create(&mut self) {
        self.prompt = Some(Prompt::CreateInput {
            buffer: LineEdit::default(),
        });
    }

    pub fn begin_search(&mut self, browser: &BrowserState) {
        self.end_search();
        self.prompt = Some(Prompt::SearchInput {
            buffer: LineEdit::default(),
            origin: SearchOrigin {
                dir: browser.current_dir().to_path_buf(),
                index: browser.selected_index(),
            },
        });
    }

    pub fn begin_command(&mut self) {
        self.prompt = Some(Prompt::CommandInput {
            buffer: LineEdit::default(),
        });
    }

    pub fn begin_paste(&mut self, browser: &BrowserState) {
        self.begin_paste_into(browser.current_dir().to_path_buf());
    }

    /// Pastes the clipboard into `dst_dir` — the browsed directory for `Paste`, or a folder that
    /// was right-clicked for the menu's "Paste into folder".
    pub fn begin_paste_into(&mut self, dst_dir: PathBuf) {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        let Some(clip) = self.clipboard.clone() else {
            self.status = Some("clipboard is empty".into());
            return;
        };
        self.start_history_batch();
        self.spawn_paste_item(clip, dst_dir, 0, ConflictPolicy::Abort);
    }

    /// Moves or copies `paths` into `dst_dir` for a drag and drop, through the same background
    /// paste a `p` does (progress, cancel, and the overwrite/skip/abort prompt on a conflict).
    /// The yank/cut clipboard is left alone: a drop is not a paste of what the user copied.
    pub fn begin_drop(&mut self, paths: Vec<PathBuf>, mode: ClipboardMode, dst_dir: PathBuf) {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        self.start_history_batch();
        self.spawn_paste_item(Clipboard { paths, mode }, dst_dir, 0, ConflictPolicy::Abort);
    }

    /// Opens the Inspect panel for `path` by asking the plugin that provides `inspect` to
    /// describe it, or says on the status line why it cannot (usually: where to get the plugin).
    pub fn begin_inspect(&mut self, path: &Path) -> Option<InspectPanel> {
        if self.plugins.dispatch_inspect(Some(path)) {
            Some(InspectPanel::open(path))
        } else {
            self.status = Some(crate::bundled::missing("inspect"));
            None
        }
    }

    /// Tells the inspecting plugin the panel has closed, so a count under way stops.
    pub fn end_inspect(&mut self) {
        let _ = self.plugins.dispatch_inspect(None);
    }

    /// Whether the inspecting plugin has a live process right now: when it has not, a panel still
    /// waiting for rows never gets them.
    pub fn inspector_running(&self) -> bool {
        self.plugins.inspector_running()
    }

    fn associations(&self) -> &Associations {
        self.associations.get_or_init(Associations::from_env)
    }

    /// Every choice the "Open with" submenu offers for `path`, best first: matching
    /// `[[open_rule]]`s, the `[[open_with]]` entries, then the programs installed for its type.
    pub fn open_choices(&self, path: &Path) -> Vec<OpenWith> {
        let assoc = self.associations();
        let mime = assoc.mime_of(path);
        let discovered = assoc
            .apps_for(&mime)
            .iter()
            .map(associations::choice)
            .collect();
        open::choices(
            &self.open_rules,
            &self.open_with,
            discovered,
            &display_name(path),
            &mime,
        )
    }

    /// Whether an `[[open_rule]]` of the user's covers `path`, which then beats the built-in
    /// habit of entering an archive as a folder.
    pub fn open_rule_covers(&self, path: &Path) -> bool {
        let mime = self.associations().mime_of(path);
        open::first_rule(&self.open_rules, &display_name(path), &mime).is_some()
    }

    /// Opens `path` the way the user would expect: the first `[[open_rule]]` that covers it, else
    /// the program the desktop has as the default for its type, else whatever `xdg-open` picks.
    pub fn open_default(&mut self, browser: &BrowserState, path: &Path) {
        let mime = self.associations().mime_of(path);
        let choice = open::first_rule(&self.open_rules, &display_name(path), &mime)
            .map(open::rule_choice)
            .or_else(|| {
                self.associations()
                    .default_for(&mime)
                    .map(|entry| associations::choice(&entry))
            });
        match choice {
            Some(choice) => self.open_choice(browser, &choice, path),
            None => self.open_with(browser, open::DEFAULT_OPENER, path),
        }
    }

    /// Opens `path` with one of the menu's choices, taking over the terminal if it needs it.
    pub fn open_choice(&mut self, browser: &BrowserState, choice: &OpenWith, path: &Path) {
        self.launch(browser, &choice.command, path, choice.terminal);
    }

    /// Asks for a command to open `path` with — the menu's "Other…".
    pub fn begin_open_other(&mut self, path: PathBuf) {
        self.prompt = Some(Prompt::OpenOther {
            path,
            buffer: LineEdit::default(),
        });
    }

    /// Opens `path` with `command` (see `open::command_line` for where the path goes). A program
    /// in the `interactive_commands` list takes over the terminal like a `:` command does; any
    /// other is started on its own, since it opens a window and must not be waited on.
    pub fn open_with(&mut self, browser: &BrowserState, command: &str, path: &Path) {
        self.launch(browser, command, path, false);
    }

    /// The one place a file is handed to a program. `terminal` forces the terminal handover for a
    /// program that needs it whatever its name (a `Terminal=true` desktop entry).
    fn launch(&mut self, browser: &BrowserState, command: &str, path: &Path, terminal: bool) {
        if shared::ssh_path::is_remote(path) {
            self.status = Some(format!(
                "cannot open {}: it is on a remote machine, copy it here first",
                display_name(path)
            ));
            return;
        }
        if vfs_archive::is_inside(path) {
            self.status = Some(format!(
                "cannot open {}: it is inside an archive, copy it out first",
                display_name(path)
            ));
            return;
        }
        let Some(program) = open::program_word(command) else {
            self.status = Some("open with: the command is empty".into());
            return;
        };
        let program = program.as_str();
        // `VAR=value prog` is shell syntax, not a program name to look for.
        if !program.contains('=') && !open::program_exists(program) {
            self.status = Some(format!("{program}: command not found"));
            return;
        }
        let line = open::command_line(command, path);
        if terminal || command::names_interactive_program(program, &self.interactive) {
            if self.is_busy() {
                self.status = Some(format!("an operation is already in progress: {line}"));
            } else {
                self.handover = Some(Handover {
                    line,
                    cwd: browser.current_dir().to_path_buf(),
                });
            }
            return;
        }
        self.status = Some(
            match shell_overlay::spawn_detached(browser.current_dir(), &line) {
                Ok(()) => format!(
                    "opening {} with {}",
                    display_name(path),
                    Path::new(program).file_name().map_or_else(
                        || program.to_string(),
                        |name| name.to_string_lossy().into_owned()
                    )
                ),
                Err(e) => format!("cannot open {}: {e}", display_name(path)),
            },
        );
    }

    /// Routes a raw key to the active prompt. No-op if there is no active prompt. Returns
    /// `ControlFlow::Break` if a `:`-command (`:q`/`:quit`) requested the app exit.
    #[cfg(test)]
    pub fn handle_prompt_key(
        &mut self,
        code: KeyCode,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
    ) -> Result<ControlFlow<()>> {
        self.handle_prompt_event(
            KeyEvent::new(code, crossterm::event::KeyModifiers::NONE),
            vfs,
            browser,
        )
    }

    /// Feeds one key press to the open prompt. A text prompt edits its line with the whole key
    /// (arrows, Home/End, Delete and the Ctrl/Alt shortcuts, see `line_edit`); the others look at
    /// the key code alone.
    pub fn handle_prompt_event(
        &mut self,
        key: KeyEvent,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
    ) -> Result<ControlFlow<()>> {
        let code = key.code;
        let Some(prompt) = self.prompt.take() else {
            return Ok(ControlFlow::Continue(()));
        };

        let mut flow = ControlFlow::Continue(());
        match prompt {
            Prompt::RenameInput { target, mut buffer } => match code {
                KeyCode::Esc => self.status = Some("rename cancelled".into()),
                KeyCode::Enter if buffer.is_empty() => {
                    self.status = Some("rename cancelled: empty name".into());
                }
                KeyCode::Enter => {
                    let name = buffer.to_string();
                    self.finish_rename(vfs, browser, target, name, ConflictPolicy::Abort)?;
                }
                _ => {
                    buffer.edit(key);
                    self.prompt = Some(Prompt::RenameInput { target, buffer });
                }
            },
            Prompt::CreateInput { mut buffer } => match code {
                KeyCode::Esc => self.status = Some("create cancelled".into()),
                KeyCode::Enter if buffer.is_empty() => {
                    self.status = Some("create cancelled: empty name".into());
                }
                KeyCode::Enter => self.finish_create(vfs, browser, buffer.to_string())?,
                _ => {
                    buffer.edit(key);
                    self.prompt = Some(Prompt::CreateInput { buffer });
                }
            },
            Prompt::SaveName { dir, mut buffer } => match code {
                KeyCode::Esc => self.status = Some("save cancelled".into()),
                KeyCode::Enter => self.finish_save_name(vfs, dir, buffer),
                _ => {
                    buffer.edit(key);
                    self.prompt = Some(Prompt::SaveName { dir, buffer });
                }
            },
            Prompt::ConfirmReplace { target } => match code {
                KeyCode::Char('y' | 'Y') => self.accept_save(target),
                _ => self.status = Some("not replaced".into()),
            },
            Prompt::ConfirmDelete { targets } => match code {
                KeyCode::Char('y' | 'Y') => {
                    browser.clear_marks();
                    self.spawn_delete(targets);
                }
                _ => self.status = Some("delete cancelled".into()),
            },
            Prompt::ConfirmTrash { targets } => match code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                    browser.clear_marks();
                    self.spawn_trash(targets);
                }
                _ => self.status = Some("trash cancelled".into()),
            },
            Prompt::Conflict(source) => match code {
                KeyCode::Char('o') => {
                    self.resolve_conflict(vfs, browser, source, ConflictPolicy::Overwrite)?
                }
                KeyCode::Char('s') => {
                    self.resolve_conflict(vfs, browser, source, ConflictPolicy::Skip)?
                }
                _ => self.status = Some("aborted".into()),
            },
            Prompt::SearchInput { mut buffer, origin } => match code {
                KeyCode::Esc => {
                    self.end_search();
                    self.status = Some(match Self::restore_origin(vfs, browser, &origin) {
                        Ok(()) => "search cancelled".into(),
                        Err(e) => format!("search cancelled; could not go back: {e}"),
                    });
                }
                KeyCode::Enter => {
                    // Still walking: stop where the cursor is rather than jump later, unasked.
                    if self.search_job.is_some() {
                        self.status = Some("search stopped".into());
                    }
                    self.end_search();
                }
                // Through the kept matches, best first, wrapping round at either end.
                KeyCode::Down => {
                    self.step_search(vfs, browser, 1);
                    self.prompt = Some(Prompt::SearchInput { buffer, origin });
                }
                KeyCode::Up => {
                    self.step_search(vfs, browser, -1);
                    self.prompt = Some(Prompt::SearchInput { buffer, origin });
                }
                KeyCode::Char(c @ ('n' | 'p'))
                    if key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL) =>
                {
                    self.step_search(vfs, browser, if c == 'n' { 1 } else { -1 });
                    self.prompt = Some(Prompt::SearchInput { buffer, origin });
                }
                _ => {
                    // Only a change to the text starts a new search; moving the cursor does not.
                    if buffer.edit(key) {
                        self.update_search(vfs, browser, &buffer, &origin);
                    }
                    self.prompt = Some(Prompt::SearchInput { buffer, origin });
                }
            },
            Prompt::OpenOther { path, mut buffer } => match code {
                KeyCode::Esc => self.status = Some("open with cancelled".into()),
                KeyCode::Enter => {
                    let command = buffer.trim().to_string();
                    if command.is_empty() {
                        self.status = Some("open with cancelled".into());
                    } else {
                        self.open_with(browser, &command, &path);
                    }
                }
                _ => {
                    buffer.edit(key);
                    self.prompt = Some(Prompt::OpenOther { path, buffer });
                }
            },
            Prompt::CompressPassword {
                pending,
                archive,
                first,
                mut buffer,
                ..
            } => match code {
                KeyCode::Esc => self.status = Some("compress cancelled".into()),
                KeyCode::Enter => self.submit_compress_password(pending, archive, first, buffer),
                code @ (KeyCode::Backspace | KeyCode::Char(_)) => {
                    match code {
                        KeyCode::Backspace => buffer.pop(),
                        KeyCode::Char(c) => buffer.push(c),
                        _ => {}
                    }
                    self.prompt = Some(Prompt::CompressPassword {
                        pending,
                        archive,
                        first,
                        buffer,
                        error: None,
                    });
                }
                _ => {
                    self.prompt = Some(Prompt::CompressPassword {
                        pending,
                        archive,
                        first,
                        buffer,
                        error: None,
                    });
                }
            },
            Prompt::ExtractPassword {
                pending,
                archive,
                mut buffer,
                ..
            } => match code {
                KeyCode::Esc => self.status = Some("extract cancelled".into()),
                KeyCode::Enter => self.submit_extract_password(pending, archive, buffer),
                KeyCode::Backspace => {
                    buffer.pop();
                    self.prompt = Some(Prompt::ExtractPassword {
                        pending,
                        archive,
                        buffer,
                        error: None,
                    });
                }
                KeyCode::Char(c) => {
                    buffer.push(c);
                    self.prompt = Some(Prompt::ExtractPassword {
                        pending,
                        archive,
                        buffer,
                        error: None,
                    });
                }
                _ => {
                    self.prompt = Some(Prompt::ExtractPassword {
                        pending,
                        archive,
                        buffer,
                        error: None,
                    });
                }
            },
            Prompt::CommandInput { mut buffer } => match code {
                KeyCode::Esc => self.status = Some("command cancelled".into()),
                KeyCode::Enter => flow = self.run_command(vfs, browser, &buffer)?,
                _ => {
                    buffer.edit(key);
                    self.prompt = Some(Prompt::CommandInput { buffer });
                }
            },
        }
        Ok(flow)
    }

    /// Reacts to the search text changing: looks in the directory the search started in first —
    /// instantly, from the list already on screen — and otherwise walks everything below it on
    /// the blocking pool (see `search_job`), the hit arriving later through `poll_search`. A
    /// stale walk is dropped (and so cancelled) first. An empty query puts the browser back.
    fn update_search(
        &mut self,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        text: &str,
        origin: &SearchOrigin,
    ) {
        self.search_job = None;
        self.search_matches.clear();
        let Some(query) = Query::new(text) else {
            self.search_state = SearchState::Idle;
            if let Err(e) = Self::restore_origin(vfs, browser, origin) {
                self.status = Some(format!("could not go back: {e}"));
            }
            return;
        };

        // The first level of the walk is the listing already on screen, so a match right here
        // shows at once.
        if browser.current_dir() == origin.dir {
            let here = rank_entries(&query, browser.current_entries());
            if !here.is_empty() {
                self.search_matches = here;
                self.show_search_match(vfs, browser, 0);
                return;
            }
        }
        self.search_job = Some(SearchJob::start(
            &self.handle,
            Arc::clone(&self.vfs),
            origin.dir.clone(),
            query,
            browser.show_hidden(),
        ));
        self.search_state = SearchState::Searching;
    }

    /// Ends the `/` prompt's search, cancelling a walk still in flight.
    fn end_search(&mut self) {
        self.search_job = None;
        self.search_matches.clear();
        self.search_state = SearchState::Idle;
    }

    /// Puts the cursor on the `position`th kept match, going to its folder if it is elsewhere.
    fn show_search_match(&mut self, vfs: &dyn Vfs, browser: &mut BrowserState, position: usize) {
        let Some(path) = self.search_matches.get(position).cloned() else {
            return;
        };
        self.search_state = match browser.reveal(vfs, &path) {
            Ok(()) => {
                self.search_position = position;
                SearchState::Found {
                    position,
                    total: self.search_matches.len(),
                    more: self.search_matches.len() >= KEPT,
                }
            }
            Err(e) => {
                self.status = Some(format!("search: could not open {}: {e}", path.display()));
                SearchState::Idle
            }
        };
    }

    /// Moves to the next (`+1`) or previous (`-1`) kept match, wrapping round at either end. Does
    /// nothing with fewer than two.
    fn step_search(&mut self, vfs: &dyn Vfs, browser: &mut BrowserState, by: isize) {
        let total = self.search_matches.len();
        if total < 2 {
            return;
        }
        let next = (self.search_position as isize + by).rem_euclid(total as isize) as usize;
        self.show_search_match(vfs, browser, next);
    }

    /// Returns the browser to where a search began: its directory, then its cursor.
    fn restore_origin(
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        origin: &SearchOrigin,
    ) -> Result<(), VfsError> {
        if browser.current_dir() != origin.dir {
            browser.goto(vfs, &origin.dir)?;
        }
        browser.select_index(origin.index);
        Ok(())
    }

    /// Applies a finished search to the browser: opens the hit's directory with the cursor on it.
    /// Call once per render tick, like `poll_bulk`.
    pub fn poll_search(&mut self, browser: &mut BrowserState, vfs: &dyn Vfs) {
        let Some(outcome) = self.search_job.as_mut().and_then(SearchJob::poll) else {
            return;
        };
        self.search_job = None;
        match outcome {
            SearchOutcome::Found(paths) => {
                self.search_matches = paths;
                self.show_search_match(vfs, browser, 0);
            }
            SearchOutcome::NotFound { truncated } => {
                self.search_state = SearchState::NotFound { truncated };
            }
            SearchOutcome::Cancelled => self.search_state = SearchState::Idle,
        }
    }

    /// The command waiting for the terminal, if a `:` command asked for one.
    pub fn take_handover(&mut self) -> Option<Handover> {
        self.handover.take()
    }

    /// Reports how a handed-over command ended and re-reads the listing, which it may have
    /// changed (an editor saving, a pager doing nothing).
    pub fn finish_handover(
        &mut self,
        browser: &mut BrowserState,
        vfs: &dyn Vfs,
        handover: &Handover,
        result: &std::io::Result<ExitStatus>,
    ) -> Result<()> {
        browser.reload(vfs)?;
        browser.prune_marks(vfs);
        self.status = Some(handover_status(&handover.line, result));
        Ok(())
    }

    /// Parses and runs a `:`-command buffer (without the leading `:`). Built-ins run right here;
    /// anything else runs under `sh -c` on the blocking pool (see `command`). Problems surface
    /// as a status message rather than an error — a typo shouldn't need a `Result` unwind.
    fn run_command(
        &mut self,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        buffer: &str,
    ) -> Result<ControlFlow<()>> {
        match command::parse(buffer, &self.interactive) {
            Ok(None) => {}
            Ok(Some(Command::Quit)) => return Ok(ControlFlow::Break(())),
            Ok(Some(Command::Cd(path))) => {
                let plan = browser.plan_goto(Path::new(&path));
                self.begin_move(browser, plan, Some(format!("cd {path}")), "cd failed");
            }
            Ok(Some(Command::Mkdir {
                parents: true,
                names,
            })) => {
                self.run_builtin(
                    vfs,
                    browser,
                    "mkdir",
                    &names,
                    file_ops::create_directory_all,
                )?;
            }
            Ok(Some(Command::Mkdir {
                parents: false,
                names,
            })) => {
                self.run_builtin(vfs, browser, "mkdir", &names, file_ops::create_directory)?;
            }
            Ok(Some(Command::Touch { names })) => {
                self.run_builtin(vfs, browser, "touch", &names, file_ops::touch)?;
            }
            Ok(Some(Command::Trash)) => self.begin_trash(browser),
            Ok(Some(Command::Extract)) => self.begin_extract(browser),
            Ok(Some(Command::Compress { name, password })) => {
                self.begin_compress(browser, &name, password);
            }
            Ok(Some(Command::Plugin { name, args })) => {
                let selection = Self::marked_or_selected(browser);
                if !self
                    .plugins
                    .dispatch_named(&name, args, browser.current_dir(), &selection)
                {
                    self.status = Some(match self.plugins.names().as_slice() {
                        [] => format!("plugin {name}: no plugins are running"),
                        running => {
                            format!("plugin {name}: not found (running: {})", running.join(", "))
                        }
                    });
                }
            }
            Ok(Some(Command::Plugins)) => {
                self.status = Some(match self.plugins.summaries().as_slice() {
                    [] => "no plugins are configured".to_string(),
                    plugins => plugins
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; "),
                });
            }
            Ok(Some(Command::PluginRestart { name })) => {
                self.status = Some(match self.plugins.restart(&name) {
                    Restart::Restarted => format!("plugin {name}: restarted"),
                    Restart::NotFound => match self.plugins.names().as_slice() {
                        [] => format!("plugin {name}: no plugins are running"),
                        running => {
                            format!("plugin {name}: not found (running: {})", running.join(", "))
                        }
                    },
                    Restart::Failed(why) => format!("plugin {name}: could not restart: {why}"),
                });
            }
            Ok(Some(Command::Shell(line))) if self.run_plugin_command(browser, &line) => {}
            Ok(Some(Command::Shell(line))) => self.spawn_shell_command(browser, line),
            Ok(Some(Command::Interactive(line))) if self.is_busy() => {
                self.status = Some(format!("an operation is already in progress: {line}"));
            }
            Ok(Some(Command::Interactive(line))) => {
                self.handover = Some(Handover {
                    line,
                    cwd: browser.current_dir().to_path_buf(),
                });
            }
            Err(message) => self.status = Some(message),
        }
        Ok(ControlFlow::Continue(()))
    }

    /// Hands `line` to the plugin that declared its first word as a `:` command, when there is
    /// one; returns whether one took it. Tried only after the built-ins, so a plugin can add
    /// commands but never replace `cd` or `q`, and before the line would run under `sh -c`.
    fn run_plugin_command(&mut self, browser: &BrowserState, line: &str) -> bool {
        let Some(word) = line.split_whitespace().next() else {
            return false;
        };
        // A line using shell syntax is the user writing shell, and a malformed one is `sh`'s to
        // report, so neither reaches a plugin.
        let Ok(args) = command::split_words(line[word.len()..].trim()) else {
            return false;
        };
        if command::uses_shell_syntax(line) {
            return false;
        }
        let selection = Self::marked_or_selected(browser);
        self.plugins
            .dispatch_command(word, args, browser.current_dir(), &selection)
    }

    /// Applies `op` to each of `names` (relative to the browsed directory) and reports the
    /// outcome, reloading the listing so the result is visible at once. Keeps going past a
    /// failure, like coreutils, so one bad name doesn't stop the rest.
    fn run_builtin(
        &mut self,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        verb: &str,
        names: &[String],
        op: fn(&dyn Vfs, &Path) -> Result<(), FileOpsError>,
    ) -> Result<()> {
        let mut done = 0;
        let mut first_error = None;
        self.start_history_batch();
        for name in names {
            let path = browser.current_dir().join(name);
            // What this creates, outermost first: anything already there is not undoable.
            let missing: Vec<PathBuf> = path
                .ancestors()
                .take_while(|ancestor| !ancestor.as_os_str().is_empty() && !vfs.exists(ancestor))
                .map(Path::to_path_buf)
                .collect();
            match op(vfs, &path) {
                Ok(()) => {
                    done += 1;
                    for created in missing.into_iter().rev() {
                        let is_dir = vfs.is_dir(&created);
                        self.history.record(Change::Created {
                            path: created,
                            is_dir,
                        });
                    }
                }
                Err(e) => {
                    first_error.get_or_insert_with(|| format!("{verb} {name}: {e}"));
                }
            }
        }
        browser.reload(vfs)?;

        self.status = Some(match (first_error, names) {
            (Some(error), _) if done > 0 => format!("{error} ({done} of {} done)", names.len()),
            (Some(error), _) => error,
            (None, [single]) => format!("{verb} {single}"),
            (None, _) => format!("{verb}: {done} done"),
        });
        Ok(())
    }

    /// Runs `line` under `sh -c` in the browsed directory on the blocking pool, so a slow
    /// command never freezes the render loop. `Esc` kills it (see `cancel_bulk`).
    fn spawn_shell_command(&mut self, browser: &BrowserState, line: String) {
        // Run here, it would act on this machine's files, not the ones in view.
        if shared::ssh_path::is_remote(browser.current_dir()) {
            self.status =
                Some("commands run on this machine; leave the remote folder first".into());
            return;
        }
        if vfs_archive::is_inside(browser.current_dir()) {
            self.status = Some("commands run on a real folder; leave the archive first".into());
            return;
        }
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        let (tx, rx) = unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_bg = Arc::clone(&cancel);
        let cwd = browser.current_dir().to_path_buf();
        let line_bg = line.clone();

        self.handle.spawn_blocking(move || {
            let _ = tx.send(BulkMsg::ShellDone(shell_overlay::run_command(
                &cwd, &line_bg, &cancel_bg,
            )));
        });

        self.bulk = Some(BulkOp {
            kind: BulkKind::Shell {
                command: line,
                started: Instant::now(),
            },
            items_done: 0,
            current: String::new(),
            cancel: Some(cancel),
            rx,
        });
    }

    fn resolve_conflict(
        &mut self,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        source: ConflictSource,
        policy: ConflictPolicy,
    ) -> Result<()> {
        match source {
            ConflictSource::Paste {
                clip,
                dst_dir,
                index,
                ..
            } => {
                self.spawn_paste_item(clip, dst_dir, index, policy);
                Ok(())
            }
            ConflictSource::Rename { target, new_name } => {
                self.finish_rename(vfs, browser, target, new_name, policy)
            }
            ConflictSource::Compress { mut pending, .. } => {
                if policy == ConflictPolicy::Skip {
                    self.status = Some("compress skipped".into());
                } else {
                    pending.policy = policy;
                    self.continue_compress(pending);
                }
                Ok(())
            }
            ConflictSource::Extract { mut pending, .. } => {
                pending.policy = policy;
                self.continue_extract(pending);
                Ok(())
            }
        }
    }

    /// If `index` is not the last item in `clip.paths`, spawns the next item (via
    /// `spawn_paste_item`, with a fresh `ConflictPolicy::Abort`) and returns `None` — the caller
    /// should stop, since the batch is still in flight. Otherwise returns `clip` back so the
    /// caller can finish up (e.g. clearing the clipboard on a completed move).
    fn try_continue_paste(
        &mut self,
        clip: Clipboard,
        dst_dir: PathBuf,
        index: usize,
    ) -> Option<Clipboard> {
        let next_index = index + 1;
        if next_index < clip.paths.len() {
            self.spawn_paste_item(clip, dst_dir, next_index, ConflictPolicy::Abort);
            None
        } else {
            Some(clip)
        }
    }

    /// Spawns a copy or move of `clip.paths[index]` into `dst_dir` on tokio's blocking thread
    /// pool. Progress and the final result arrive later via `poll_bulk` — this call itself never
    /// blocks. `poll_bulk` re-invokes this for the next item once one finishes, so a multi-item
    /// batch (from marks) stays one continuous background operation from the UI's perspective.
    fn spawn_paste_item(
        &mut self,
        clip: Clipboard,
        dst_dir: PathBuf,
        index: usize,
        policy: ConflictPolicy,
    ) {
        let Some(src) = clip.paths.get(index).cloned() else {
            self.status = Some("paste failed: batch index out of range".into());
            return;
        };
        let Some(name) = src.file_name() else {
            self.status = Some("clipboard entry has no file name".into());
            return;
        };
        let dst = dst_dir.join(name);

        let (tx, rx): (UnboundedSender<BulkMsg>, _) = unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_bg = Arc::clone(&cancel);
        let mode = clip.mode;
        let dst_bg = dst.clone();
        let vfs = Arc::clone(&self.vfs);

        self.handle.spawn_blocking(move || {
            let progress_tx = tx.clone();
            let mut on_progress = move |path: &Path| -> ControlFlow<()> {
                let _ = progress_tx.send(BulkMsg::Progress {
                    path: display_name(path),
                });
                if cancel_bg.load(Ordering::Relaxed) {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            };
            let result = match mode {
                ClipboardMode::Copy => file_ops::copy_with_progress(
                    vfs.as_ref(),
                    &src,
                    &dst_bg,
                    policy,
                    &mut on_progress,
                ),
                ClipboardMode::Move => file_ops::mv_with_progress(
                    vfs.as_ref(),
                    &src,
                    &dst_bg,
                    policy,
                    &mut on_progress,
                ),
            };
            let _ = tx.send(BulkMsg::Done(result));
        });

        self.bulk = Some(BulkOp {
            kind: BulkKind::Paste {
                clip,
                dst_dir,
                index,
                dst,
                policy,
            },
            items_done: 0,
            current: String::new(),
            cancel: Some(cancel),
            rx,
        });
    }

    /// Spawns a delete of one or more targets on tokio's blocking thread pool, one
    /// `file_ops::delete` call per target in order. Stops at the first failure — the same
    /// abort-not-partial semantics `ConflictPolicy::Abort` gives paste — rather than trying to
    /// press on through the rest of the batch. No per-item cancellation (see `BulkOp` doc), but
    /// it still keeps a large recursive delete from freezing the render loop.
    fn spawn_delete(&mut self, targets: Vec<PathBuf>) {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        let (tx, rx) = unbounded_channel();
        let targets_bg = targets.clone();
        let vfs = Arc::clone(&self.vfs);

        self.handle.spawn_blocking(move || {
            let mut result = Ok(Outcome::Completed);
            for target in &targets_bg {
                match file_ops::delete(vfs.as_ref(), target) {
                    Ok(()) => {
                        let _ = tx.send(BulkMsg::Progress {
                            path: display_name(target),
                        });
                    }
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                }
            }
            let _ = tx.send(BulkMsg::Done(result));
        });

        self.bulk = Some(BulkOp {
            kind: BulkKind::Delete { targets },
            items_done: 0,
            current: String::new(),
            cancel: None,
            rx,
        });
    }

    /// Starts extracting the marked (or selected) archives, each into a new directory named after
    /// it beside the archive's own listing, so a tarball full of loose files cannot litter the
    /// browsed directory.
    fn begin_extract(&mut self, browser: &mut BrowserState) {
        let archives: Vec<PathBuf> = Self::marked_only(browser)
            .into_iter()
            .filter(|path| Format::of(path).is_some())
            .collect();
        match archives.is_empty() {
            true => {
                self.status =
                    Some("extract: no zip, tar or tar.gz marked — mark with v first".into())
            }
            false => {
                browser.clear_marks();
                let jobs = Self::extract_jobs_per_archive(browser.current_dir(), archives);
                self.continue_extract(PendingExtract {
                    jobs,
                    policy: ConflictPolicy::Abort,
                    delete_archives: false,
                });
            }
        }
    }

    /// Starts `pending`, first asking what to do about a folder that is already there, the way a
    /// paste asks about a file. `Abort` is what a fresh extract carries, so it means "not asked
    /// yet"; answering abort ends the extract instead of coming back here.
    fn continue_extract(&mut self, pending: PendingExtract) {
        if pending.policy == ConflictPolicy::Abort
            && let Some(job) = pending
                .jobs
                .iter()
                .find(|job| lexists(self.vfs.as_ref(), &job.dest))
        {
            let dest = job.dest.clone();
            self.prompt = Some(Prompt::Conflict(ConflictSource::Extract { pending, dest }));
            return;
        }
        self.spawn_extract(pending.jobs, pending.policy, pending.delete_archives, None);
    }

    /// One job per archive, each into a folder of `dir` named after it.
    fn extract_jobs_per_archive(dir: &Path, archives: Vec<PathBuf>) -> Vec<ExtractJob> {
        archives
            .into_iter()
            .map(|archive| {
                let stem = Format::stem_of(&archive).unwrap_or_else(|| "extracted".into());
                ExtractJob {
                    dest: dir.join(stem),
                    archive,
                }
            })
            .collect()
    }

    /// The extract form for the marked (or selected) archives, or `None` if there are none.
    pub fn begin_extract_form(&mut self, browser: &BrowserState) -> Option<ExtractPopup> {
        let archives: Vec<PathBuf> = Self::marked_only(browser)
            .into_iter()
            .filter(|path| Format::of(path).is_some())
            .collect();
        let mut form = ExtractPopup::new(archives, browser.current_dir().to_path_buf());
        match form.as_mut() {
            Some(form) => {
                let warning = Self::extract_warning(self.vfs.as_ref(), form);
                form.set_warning(warning);
            }
            None => {
                self.status =
                    Some("extract: no zip, tar or tar.gz marked — mark with v first".into());
            }
        }
        form
    }

    /// Starts the job a submitted extract form describes.
    ///
    /// # Errors
    ///
    /// A message for the form to show when the job cannot start: another operation is running,
    /// (with existing files set to stop) a folder the job would create already exists, a path is
    /// not on this machine's disk, or there is no archive plugin to do it.
    pub fn start_extract(&mut self, request: extract_popup::Request) -> Result<(), String> {
        if self.is_busy() {
            return Err("an operation is already in progress".into());
        }
        if let Some(problem) = Self::extract_problem(self.vfs.as_ref(), &request) {
            return Err(problem);
        }
        let jobs = Self::extract_jobs(&request);
        self.try_spawn_extract(jobs, request.policy, request.delete_archives, None)
    }

    /// The jobs an extract form's request describes.
    fn extract_jobs(request: &extract_popup::Request) -> Vec<ExtractJob> {
        match &request.destination {
            extract_popup::Destination::Here => request
                .archives
                .iter()
                .map(|archive| ExtractJob {
                    archive: archive.clone(),
                    dest: request.dir.clone(),
                })
                .collect(),
            extract_popup::Destination::Folder(name) => request
                .archives
                .iter()
                .map(|archive| ExtractJob {
                    archive: archive.clone(),
                    dest: request.dir.join(name),
                })
                .collect(),
            extract_popup::Destination::PerArchive => {
                Self::extract_jobs_per_archive(&request.dir, request.archives.clone())
            }
        }
    }

    /// Why an extract form's request would be refused, if it would. Extracting into a folder that
    /// is already there is what "skip" and "replace" are for; with "stop" it would only fail on the
    /// first clashing file, after writing the others. Shown live as a warning and again on Enter.
    pub fn extract_problem(vfs: &dyn Vfs, request: &extract_popup::Request) -> Option<String> {
        if request.policy != ConflictPolicy::Abort {
            return None;
        }
        Self::extract_jobs(request)
            .iter()
            .find(|job| job.dest != request.dir && lexists(vfs, &job.dest))
            .map(|taken| {
                format!(
                    "{} already exists: set Existing files to skip or replace",
                    display_name(&taken.dest)
                )
            })
    }

    /// Starts packing the marked (or selected) entries into `name` in the browsed directory. With
    /// `ask_password` (`:compress -p`) a zip is sealed with a password asked for in a prompt.
    fn begin_compress(&mut self, browser: &mut BrowserState, name: &str, ask_password: bool) {
        let Some(format) = Format::of(Path::new(name)) else {
            self.status = Some("compress: name must end in .zip, .tar or .tar.gz".into());
            return;
        };
        if ask_password && format != Format::Zip {
            self.status = Some("compress: only a zip can be encrypted".into());
            return;
        }
        let sources = Self::marked_only(browser);
        if sources.is_empty() {
            self.status = Some("compress: nothing marked — mark with v first".into());
            return;
        }
        browser.clear_marks();
        let out = browser.current_dir().join(name);
        self.continue_compress(PendingCompress {
            jobs: vec![CompressJob { sources, out }],
            format,
            level: Level::default(),
            policy: ConflictPolicy::Abort,
            delete_originals: false,
            ask_password,
        });
    }

    /// Takes `pending` to its next step: asking about an archive already in the way, then for the
    /// password, then running. `Abort` is what a fresh compress carries, so it means "not asked
    /// yet"; answering abort ends the compress instead of coming back here.
    fn continue_compress(&mut self, pending: PendingCompress) {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        if pending.policy == ConflictPolicy::Abort
            && let Some(job) = pending
                .jobs
                .iter()
                .find(|job| lexists(self.vfs.as_ref(), &job.out))
        {
            let out = job.out.clone();
            self.prompt = Some(Prompt::Conflict(ConflictSource::Compress { pending, out }));
            return;
        }
        if pending.ask_password {
            let archive = pending.jobs[0].out.clone();
            self.prompt = Some(Prompt::CompressPassword {
                pending,
                archive,
                first: None,
                buffer: Password::new(),
                error: None,
            });
            return;
        }
        self.spawn_compress(
            pending.jobs,
            pending.format,
            pending.level,
            None,
            pending.policy,
            pending.delete_originals,
        );
    }

    /// Takes one answer to the password prompt: the first is kept and asked for again, the second
    /// must match it, and only then does the compress start.
    fn submit_compress_password(
        &mut self,
        pending: PendingCompress,
        archive: PathBuf,
        first: Option<Password>,
        buffer: Password,
    ) {
        let ask = |pending, archive, first, error: Option<&str>| Prompt::CompressPassword {
            pending,
            archive,
            first,
            buffer: Password::new(),
            error: error.map(str::to_string),
        };
        if buffer.is_empty() {
            self.prompt = Some(ask(pending, archive, first, Some("type the password")));
            return;
        }
        match first {
            None => self.prompt = Some(ask(pending, archive, Some(buffer), None)),
            Some(first) if first == buffer => self.spawn_compress(
                pending.jobs,
                pending.format,
                pending.level,
                Some(first),
                pending.policy,
                pending.delete_originals,
            ),
            Some(_) => {
                self.prompt = Some(ask(
                    pending,
                    archive,
                    None,
                    Some("the passwords do not match"),
                ));
            }
        }
    }

    /// Checks the typed password the way the plugin will, and starts the extract again with it.
    /// The plugin checks it against every encrypted archive of the batch before writing anything,
    /// and asks again if one is refused.
    fn submit_extract_password(
        &mut self,
        pending: PendingExtract,
        archive: PathBuf,
        buffer: Password,
    ) {
        if buffer.is_empty() {
            self.prompt = Some(Prompt::ExtractPassword {
                pending,
                archive,
                buffer: Password::new(),
                error: Some("type the password".to_string()),
            });
            return;
        }
        self.spawn_extract(
            pending.jobs,
            pending.policy,
            pending.delete_archives,
            Some(buffer),
        );
    }

    /// The channel and cancel flag every archive job reports through: the plugin's answers are
    /// turned into messages on the channel (see `archive_progress` and `archive_done`).
    fn archive_channel() -> (
        UnboundedSender<BulkMsg>,
        UnboundedReceiver<BulkMsg>,
        Arc<AtomicBool>,
    ) {
        let (tx, rx) = unbounded_channel();
        (tx, rx, Arc::new(AtomicBool::new(false)))
    }

    /// Why `paths` cannot be handed to the archive plugin, if one cannot: it works on this
    /// machine's disk only, so a folder on a remote machine or inside another archive is refused
    /// until the plugin can be given files through the host.
    fn archive_refusal<'a>(
        verb: &str,
        paths: impl IntoIterator<Item = &'a Path>,
    ) -> Option<String> {
        paths.into_iter().find_map(|path| {
            let place = if shared::ssh_path::is_remote(path) {
                "a remote machine"
            } else if vfs_archive::is_inside(path) {
                "inside an archive"
            } else {
                return None;
            };
            Some(format!(
                "{verb}: {} is on {place}; archives are only made and opened on this machine's \
                 disk for now",
                display_name(path)
            ))
        })
    }

    /// What the plugin does when a name is taken, from what the interface calls it.
    fn conflict_of(policy: ConflictPolicy) -> Conflict {
        match policy {
            ConflictPolicy::Abort => Conflict::Abort,
            ConflictPolicy::Skip => Conflict::Skip,
            ConflictPolicy::Overwrite => Conflict::Overwrite,
        }
    }

    /// Hands `work` to the archive plugin and shows it as a running operation of `kind`.
    /// `trash_after` holds, for each job, what goes to the trash once that job has written its
    /// archive; `again` lets an extract be started afresh when the plugin asks for a password.
    ///
    /// # Errors
    ///
    /// Why it did not start: another operation is running, a path is not on this machine's disk,
    /// or no archive plugin is there to take it.
    fn start_archive(
        &mut self,
        kind: BulkKind,
        work: ArchiveWork,
        trash_after: Vec<Vec<PathBuf>>,
        again: Option<PendingExtract>,
    ) -> Result<(), String> {
        if self.is_busy() {
            return Err("an operation is already in progress".into());
        }
        let job = self.next_archive_job;
        self.next_archive_job += 1;
        if !self.plugins.dispatch_archive(job, work) {
            return Err(crate::bundled::missing("archive"));
        }
        let (tx, rx, cancel) = Self::archive_channel();
        self.archive_run = Some(ArchiveRun {
            job,
            tx,
            trash_after,
            again,
            gone_since: None,
        });
        self.bulk = Some(BulkOp {
            kind,
            items_done: 0,
            current: String::new(),
            cancel: Some(cancel),
            rx,
        });
        Ok(())
    }

    /// Starts one extraction per entry of `jobs`, in order, stopping at the first failure. With
    /// `delete_archives`, each archive goes to the trash once it has been extracted, so a failure
    /// never costs an archive its contents. With no `password`, the plugin asks for one (see
    /// `archive_done`) if an archive needs it.
    fn spawn_extract(
        &mut self,
        jobs: Vec<ExtractJob>,
        policy: ConflictPolicy,
        delete_archives: bool,
        password: Option<Password>,
    ) {
        if let Err(message) = self.try_spawn_extract(jobs, policy, delete_archives, password) {
            self.status = Some(message);
        }
    }

    fn try_spawn_extract(
        &mut self,
        jobs: Vec<ExtractJob>,
        policy: ConflictPolicy,
        delete_archives: bool,
        password: Option<Password>,
    ) -> Result<(), String> {
        let touched = jobs
            .iter()
            .flat_map(|job| [job.archive.as_path(), job.dest.as_path()]);
        if let Some(refusal) = Self::archive_refusal("extract", touched) {
            return Err(refusal);
        }
        let trash_after = match delete_archives {
            true => jobs.iter().map(|job| vec![job.archive.clone()]).collect(),
            false => Vec::new(),
        };
        let work = ArchiveWork::Extract {
            jobs: jobs.clone(),
            policy: Self::conflict_of(policy),
            password,
        };
        let again = PendingExtract {
            jobs,
            policy,
            delete_archives,
        };
        self.start_archive(BulkKind::Extract, work, trash_after, Some(again))
    }

    /// Starts one archive job per entry of `jobs`, in order, stopping at the first failure. With
    /// `delete_originals`, each job's sources go to the trash once its archive is complete, so a
    /// failure never costs a file its only copy.
    fn spawn_compress(
        &mut self,
        jobs: Vec<CompressJob>,
        format: Format,
        level: Level,
        password: Option<Password>,
        policy: ConflictPolicy,
        delete_originals: bool,
    ) {
        if let Err(message) =
            self.try_spawn_compress(jobs, format, level, password, policy, delete_originals)
        {
            self.status = Some(message);
        }
    }

    fn try_spawn_compress(
        &mut self,
        jobs: Vec<CompressJob>,
        format: Format,
        level: Level,
        password: Option<Password>,
        policy: ConflictPolicy,
        delete_originals: bool,
    ) -> Result<(), String> {
        let touched = jobs.iter().flat_map(|job| {
            job.sources
                .iter()
                .map(PathBuf::as_path)
                .chain(std::iter::once(job.out.as_path()))
        });
        if let Some(refusal) = Self::archive_refusal("compress", touched) {
            return Err(refusal);
        }
        let trash_after = match delete_originals {
            true => jobs.iter().map(|job| job.sources.clone()).collect(),
            false => Vec::new(),
        };
        let work = ArchiveWork::Compress {
            jobs,
            format,
            level,
            policy: Self::conflict_of(policy),
            password,
        };
        self.start_archive(BulkKind::Compress, work, trash_after, None)
    }

    /// Passes the archive plugin's progress on to the running operation, unless it is about a
    /// request that has ended.
    pub fn archive_progress(&mut self, progress: ArchiveProgress) {
        if let Some(run) = self
            .archive_run
            .as_ref()
            .filter(|run| run.job == progress.job)
        {
            let _ = run.tx.send(BulkMsg::Entries {
                done: progress.done,
                path: progress.path,
            });
        }
    }

    /// Ends the running operation the way the archive plugin says its request ended: a password
    /// to ask for, or a result after the originals of the jobs that finished have gone to the
    /// trash. An answer to a request that is no longer running is dropped.
    pub fn archive_done(&mut self, done: ArchiveDone) {
        if self
            .archive_run
            .as_ref()
            .is_none_or(|run| run.job != done.job)
        {
            return;
        }
        let run = self.archive_run.take().expect("checked just above");
        let outcome = match done.status {
            ArchiveStatus::PasswordNeeded { archive } => {
                return self.ask_extract_password(run, archive, None);
            }
            ArchiveStatus::WrongPassword { archive } => {
                let error = format!("wrong password for {}", display_name(&archive));
                return self.ask_extract_password(run, archive, Some(error));
            }
            ArchiveStatus::Completed => Ok(Outcome::Completed),
            ArchiveStatus::Cancelled => Err(FileOpsError::Cancelled),
            ArchiveStatus::Failed { message } => Err(FileOpsError::Plugin(message)),
        };
        let to_trash: Vec<PathBuf> = done
            .finished
            .iter()
            .filter_map(|index| run.trash_after.get(*index))
            .flatten()
            .cloned()
            .collect();
        let tx = run.tx;
        if to_trash.is_empty() {
            let _ = tx.send(BulkMsg::Done(outcome));
            return;
        }
        self.handle.spawn_blocking(move || {
            let mut result = outcome;
            for path in &to_trash {
                if let Err(e) = trash_here(path) {
                    // What failed in the archive stays the headline; a path that stayed behind is
                    // only worth saying when the archive itself was fine.
                    if result.is_ok() {
                        result = Err(e);
                    }
                    break;
                }
            }
            let _ = tx.send(BulkMsg::Done(result));
        });
    }

    /// Ends the running extract and asks for the password `archive` needs, to start it again with.
    fn ask_extract_password(&mut self, run: ArchiveRun, archive: PathBuf, error: Option<String>) {
        self.bulk = None;
        let Some(pending) = run.again else {
            self.status = Some(
                "extract failed: a password was asked for by a request that could not take one"
                    .into(),
            );
            return;
        };
        self.prompt = Some(Prompt::ExtractPassword {
            pending,
            archive,
            buffer: Password::new(),
            error,
        });
    }

    /// Gives up on a request whose plugin has stopped without ending it. Call once per render
    /// tick, after the plugin's queued answers have been taken: they can still be on their way
    /// when its process is seen to end, so it is given `ARCHIVER_GRACE` first.
    pub fn check_archiver(&mut self, now: Instant) {
        let running = self.plugins.archiver_running();
        let Some(run) = self.archive_run.as_mut() else {
            return;
        };
        if running {
            run.gone_since = None;
            return;
        }
        let since = *run.gone_since.get_or_insert(now);
        if now.duration_since(since) >= ARCHIVER_GRACE {
            let run = self.archive_run.take().expect("checked just above");
            let _ = run.tx.send(BulkMsg::Done(Err(FileOpsError::Plugin(
                "the archive plugin stopped before it finished".into(),
            ))));
        }
    }

    /// Tells the archive plugin to stop the request in flight, if there is one.
    fn signal_archive_cancel(&self) {
        if let Some(run) = &self.archive_run {
            self.plugins.cancel_archive(run.job);
        }
    }

    /// The compress form for the marked (or selected) entries, or `None` if there are none.
    pub fn begin_compress_form(&mut self, browser: &BrowserState) -> Option<CompressPopup> {
        let mut form = CompressPopup::new(
            Self::marked_only(browser),
            browser.current_dir().to_path_buf(),
            self.vfs.as_ref(),
        );
        match form.as_mut() {
            Some(form) => {
                let warning = Self::compress_warning(self.vfs.as_ref(), form);
                form.set_warning(warning);
            }
            None => self.status = Some("compress: nothing marked — mark with v first".into()),
        }
        form
    }

    /// Starts the job a submitted compress form describes.
    ///
    /// # Errors
    ///
    /// A message for the form to show when the job cannot start: another operation is running,
    /// a file the job would create already exists (nothing is ever overwritten), a path is not on
    /// this machine's disk, or there is no archive plugin to do it.
    pub fn start_compress(&mut self, request: compress_popup::Request) -> Result<(), String> {
        if self.is_busy() {
            return Err("an operation is already in progress".into());
        }
        if let Some(problem) = Self::compress_problem(self.vfs.as_ref(), &request) {
            return Err(problem);
        }
        let jobs = Self::compress_jobs(&request);
        self.try_spawn_compress(
            jobs,
            request.format,
            request.level,
            request.password,
            ConflictPolicy::Abort,
            request.delete_originals,
        )
    }

    /// The heads-up for the compress form as it stands, worked out from the filesystem.
    fn compress_warning(vfs: &dyn Vfs, popup: &CompressPopup) -> Option<String> {
        popup
            .build()
            .ok()
            .and_then(|request| Self::compress_problem(vfs, &request))
    }

    /// The heads-up for the extract form as it stands, worked out from the filesystem.
    fn extract_warning(vfs: &dyn Vfs, popup: &ExtractPopup) -> Option<String> {
        popup
            .build()
            .ok()
            .and_then(|request| Self::extract_problem(vfs, &request))
    }

    /// Re-works the form's heads-up from the filesystem: call after every change to the form.
    pub fn refresh_compress_warning(&self, popup: &mut CompressPopup) {
        let warning = Self::compress_warning(self.vfs.as_ref(), popup);
        popup.set_warning(warning);
    }

    /// Re-works the form's heads-up from the filesystem: call after every change to the form.
    pub fn refresh_extract_warning(&self, popup: &mut ExtractPopup) {
        let warning = Self::extract_warning(self.vfs.as_ref(), popup);
        popup.set_warning(warning);
    }

    /// The jobs a compress form's request describes.
    fn compress_jobs(request: &compress_popup::Request) -> Vec<CompressJob> {
        match &request.layout {
            compress_popup::Layout::One { file_name } => vec![CompressJob {
                sources: request.sources.clone(),
                out: request.dir.join(file_name),
            }],
            compress_popup::Layout::PerItem => {
                let extension = request.format.extension();
                request
                    .sources
                    .iter()
                    .filter_map(|source| {
                        // `a.txt` becomes `a.txt.zip`, so `a.txt` and `a.pdf` cannot collide.
                        let mut name = source.file_name()?.to_os_string();
                        name.push(format!(".{extension}"));
                        Some(CompressJob {
                            out: request.dir.join(name),
                            sources: vec![source.clone()],
                        })
                    })
                    .collect()
            }
        }
    }

    /// Why a compress form's request would be refused, if it would: the folder to save in is not
    /// there, or a file the job would create already exists (the form never overwrites). Shown
    /// live as a warning and again on Enter.
    pub fn compress_problem(vfs: &dyn Vfs, request: &compress_popup::Request) -> Option<String> {
        if !vfs.is_dir(&request.dir) {
            return Some(format!("{} is not a folder", request.dir.display()));
        }
        Self::compress_jobs(request)
            .iter()
            .find(|job| lexists(vfs, &job.out))
            .map(|taken| format!("{} already exists", display_name(&taken.out)))
    }

    /// Same shape as `spawn_delete`, but sends each target to the desktop trash
    /// (`file_ops::trash`) instead of deleting it — no `Vfs` involved (see that function's docs).
    fn spawn_trash(&mut self, targets: Vec<PathBuf>) {
        // Only this machine has a trash; sending the files to it would not find them, and
        // deleting them for good instead is not what a trash key promises.
        if targets
            .iter()
            .any(|target| shared::ssh_path::is_remote(target))
        {
            self.status = Some("a remote machine has no trash; use the permanent delete".into());
            return;
        }
        if targets.iter().any(|target| vfs_archive::is_inside(target)) {
            self.status = Some("an archive is read-only".into());
            return;
        }
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return;
        }
        self.start_history_batch();
        self.untracked = 0;
        let (tx, rx) = unbounded_channel();
        let targets_bg = targets.clone();

        self.handle.spawn_blocking(move || {
            let mut result = Ok(Outcome::Completed);
            // Started first so what lands in the trash can be told from what was already there.
            let tracker = TrashTracker::start();
            let mut sent = Vec::new();
            for target in &targets_bg {
                match file_ops::trash(target) {
                    Ok(()) => {
                        sent.push(target.clone());
                        let _ = tx.send(BulkMsg::Progress {
                            path: display_name(target),
                        });
                    }
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                }
            }
            let (changes, untracked) = tracker.finish(&sent);
            let _ = tx.send(BulkMsg::Trashed { changes, untracked });
            let _ = tx.send(BulkMsg::Done(result));
        });

        self.bulk = Some(BulkOp {
            kind: BulkKind::Trash { targets },
            items_done: 0,
            current: String::new(),
            cancel: None,
            rx,
        });
    }

    /// Records a finished copy or move of `clip.paths[index]` to `dst`, unless it replaced
    /// something (an overwrite cannot be put back).
    fn record_paste(&mut self, clip: &Clipboard, index: usize, dst: &Path, policy: ConflictPolicy) {
        let Some(src) = clip.paths.get(index) else {
            return;
        };
        if policy == ConflictPolicy::Overwrite {
            return;
        }
        self.history.record(match clip.mode {
            ClipboardMode::Copy => Change::Copied {
                src: src.clone(),
                dst: dst.to_path_buf(),
            },
            ClipboardMode::Move => Change::Moved {
                from: src.clone(),
                to: dst.to_path_buf(),
            },
        });
    }

    /// Takes back the newest file operation, or says why it cannot.
    pub fn undo(&mut self, browser: &mut BrowserState, vfs: &dyn Vfs) -> Result<()> {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return Ok(());
        }
        let report = self.history.undo(vfs);
        self.report_history(browser, vfs, "undo", "undid", report)
    }

    /// Does again what `undo` took back.
    pub fn redo(&mut self, browser: &mut BrowserState, vfs: &dyn Vfs) -> Result<()> {
        if self.is_busy() {
            self.status = Some("an operation is already in progress".into());
            return Ok(());
        }
        let report = self.history.redo(vfs);
        self.report_history(browser, vfs, "redo", "redid", report)
    }

    fn report_history(
        &mut self,
        browser: &mut BrowserState,
        vfs: &dyn Vfs,
        verb: &str,
        past: &str,
        report: Report,
    ) -> Result<()> {
        self.status = Some(match report {
            Report::Empty => format!("nothing to {verb}"),
            Report::Done { label } => format!("{past} {label}"),
            Report::Failed { label, error } => format!("{verb} {label} failed: {error}"),
        });
        browser.reload(vfs)?;
        browser.prune_marks(vfs);
        Ok(())
    }

    /// Ends the batch being recorded and starts a fresh one: each user-started operation is its
    /// own undo step.
    fn start_history_batch(&mut self) {
        self.history.commit();
    }

    fn finish_rename(
        &mut self,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        target: PathBuf,
        new_name: String,
        policy: ConflictPolicy,
    ) -> Result<()> {
        match file_ops::rename(vfs, &target, &new_name, policy) {
            Ok(Outcome::Completed) => {
                self.start_history_batch();
                if policy != ConflictPolicy::Overwrite
                    && let Some(parent) = target.parent()
                {
                    self.history.record(Change::Moved {
                        from: target.clone(),
                        to: parent.join(&new_name),
                    });
                }
                browser.reload(vfs)?;
                self.status = Some(format!("renamed to {new_name}"));
            }
            Ok(Outcome::Skipped) => self.status = Some("rename skipped".into()),
            Err(FileOpsError::Vfs(VfsError::AlreadyExists(_))) => {
                self.prompt = Some(Prompt::Conflict(ConflictSource::Rename {
                    target,
                    new_name,
                }));
            }
            Err(e) => self.status = Some(format!("rename failed: {e}")),
        }
        Ok(())
    }

    fn finish_create(
        &mut self,
        vfs: &dyn Vfs,
        browser: &mut BrowserState,
        name: String,
    ) -> Result<()> {
        let is_dir = name.ends_with('/');
        let trimmed = name.trim_end_matches('/');
        if trimmed.is_empty() {
            self.status = Some("create cancelled: empty name".into());
            return Ok(());
        }

        let path = browser.current_dir().join(trimmed);
        let result = if is_dir {
            file_ops::create_directory(vfs, &path)
        } else {
            file_ops::create_new_file(vfs, &path)
        };

        match result {
            Ok(()) => {
                self.start_history_batch();
                self.history.record(Change::Created { path, is_dir });
                browser.reload(vfs)?;
                self.status = Some(format!("created {trimmed}"));
            }
            Err(e) => self.status = Some(format!("create failed: {e}")),
        }
        Ok(())
    }
}

/// One status-line summary of a finished command: its output (lines joined, so `ls` fits on the
/// bar) when it printed any, else just that it ran, and the exit code when it failed.
fn shell_status(command: &str, outcome: &CommandOutcome) -> String {
    let CommandOutcome::Finished(output) = outcome else {
        return format!("cancelled: {command}");
    };
    let lines: Vec<&str> = output
        .text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let mut shown = lines
        .iter()
        .take(STATUS_OUTPUT_LINES)
        .copied()
        .collect::<Vec<_>>()
        .join(" | ");
    if lines.len() > STATUS_OUTPUT_LINES {
        shown.push_str(&format!(
            " (+{} more lines)",
            lines.len() - STATUS_OUTPUT_LINES
        ));
    } else if output.truncated {
        shown.push_str(" (output truncated)");
    }

    match (output.code, shown.is_empty()) {
        (Some(0), true) => format!("ran: {command}"),
        (Some(0), false) => shown,
        (Some(code), true) => format!("exit {code}: {command}"),
        (Some(code), false) => format!("exit {code}: {shown}"),
        (None, _) => format!("killed by a signal: {command}"),
    }
}

/// The status line for a command that had the terminal to itself.
fn handover_status(line: &str, result: &std::io::Result<ExitStatus>) -> String {
    match result {
        Ok(status) if status.success() => format!("{line}: done"),
        Ok(status) => match status.code() {
            Some(code) => format!("{line}: exited with code {code}"),
            None => format!("{line}: ended by a signal"),
        },
        Err(e) => format!("could not run '{line}': {e}"),
    }
}

/// What an operation says when nothing is marked.
const NOTHING_MARKED: &str = "nothing marked — mark with v first";

/// What an operation says when nothing is marked and no entry is under the cursor either.
const NOTHING_HERE: &str = "nothing here to act on";

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Renders a batch of paths for a status message: the single name, or an item count.
fn batch_label(paths: &[PathBuf]) -> String {
    match paths {
        [single] => display_name(single),
        many => format!("{} items", many.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shell_overlay::CommandOutput;

    fn finished(code: i32, text: &str) -> CommandOutcome {
        CommandOutcome::Finished(CommandOutput {
            code: Some(code),
            text: text.into(),
            truncated: false,
        })
    }

    #[test]
    fn a_quiet_success_says_it_ran_and_a_chatty_one_shows_its_output() {
        assert_eq!(shell_status("true", &finished(0, "")), "ran: true");
        assert_eq!(shell_status("ls", &finished(0, "a\n\nb\n")), "a | b");
    }

    #[test]
    fn a_failure_carries_its_exit_code_and_any_message() {
        assert_eq!(shell_status("false", &finished(1, "")), "exit 1: false");
        assert_eq!(
            shell_status("ls x", &finished(2, "ls: cannot access 'x'\n")),
            "exit 2: ls: cannot access 'x'"
        );
    }

    #[test]
    fn long_output_is_summarized_and_cancellation_and_signals_are_named() {
        let many = (1..=12).map(|n| format!("{n}\n")).collect::<String>();
        assert_eq!(
            shell_status("seq 12", &finished(0, &many)),
            "1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 (+4 more lines)"
        );
        assert_eq!(
            shell_status("sleep 9", &CommandOutcome::Cancelled),
            "cancelled: sleep 9"
        );
        let killed = CommandOutcome::Finished(CommandOutput {
            code: None,
            text: String::new(),
            truncated: false,
        });
        assert_eq!(shell_status("x", &killed), "killed by a signal: x");
    }

    struct Fixture {
        _runtime: tokio::runtime::Runtime,
        root: PathBuf,
        app: App,
        browser: BrowserState,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "minuteman-app-test-{label}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let app =
                App::new(runtime.handle().clone()).with_interactive_commands(vec!["nvim".into()]);
            let browser = BrowserState::new(&LocalVfs, root.clone()).unwrap();
            Self {
                _runtime: runtime,
                root,
                app,
                browser,
            }
        }

        fn flow(&mut self, line: &str) -> ControlFlow<()> {
            self.app
                .run_command(&LocalVfs, &mut self.browser, line)
                .unwrap()
        }

        /// Where the archive recorder writes what it is sent: beside the fixture's folder, so it
        /// never shows up in the listing.
        fn archive_log(&self) -> PathBuf {
            PathBuf::from(format!("{}.archive-log", self.root.display()))
        }

        /// Attaches a plugin that provides `archive` and writes every line it is sent to a file,
        /// standing in for `mman-archive`, whose work is tested in its own crate. What these tests
        /// check is what the interface asks the plugin for, and what it does with each answer.
        fn with_archiver(mut self) -> Self {
            let spec = PluginSpec {
                name: "archive-recorder".to_string(),
                command: "sh".to_string(),
                args: vec![
                    "-c".to_string(),
                    format!("cat > '{}'", self.archive_log().display()),
                ],
                on_keys: Vec::new(),
                commands: Vec::new(),
                events: Vec::new(),
                provides: vec!["archive".to_string()],
            };
            let handle = self._runtime.handle().clone();
            self.app = std::mem::replace(&mut self.app, App::new(handle))
                .with_plugins(vec![spec], &self.root);
            self
        }

        /// Every line the recorder has been sent so far that has the event `name`, oldest first.
        fn recorded(&self, name: &str) -> Vec<serde_json::Value> {
            std::fs::read_to_string(self.archive_log())
                .unwrap_or_default()
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .filter(|line| line["params"]["name"] == name)
                .map(|line| line["params"].clone())
                .collect()
        }

        /// Waits for the recorder to have been sent `count` archive requests, and returns the
        /// newest one's `archive` parameters.
        fn request(&self, count: usize) -> serde_json::Value {
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let seen = self.recorded("archive");
                if seen.len() >= count {
                    return seen[count - 1]["archive"].clone();
                }
                assert!(
                    Instant::now() < deadline,
                    "request {count} never reached the plugin"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        /// Plays the plugin's answer to request number `count`: it ended with `status` having
        /// written the jobs in `finished`.
        fn archive_ends(&mut self, count: usize, finished: &[usize], status: ArchiveStatus) {
            let job = self.request(count)["job"].as_u64().unwrap();
            self.app.archive_done(ArchiveDone {
                job,
                finished: finished.to_vec(),
                status,
            });
        }

        /// Runs a command that must not ask the app to quit.
        fn run(&mut self, line: &str) {
            assert_eq!(self.flow(line), ControlFlow::Continue(()), "{line}");
        }

        /// Waits for a background `:` command to finish, applying its result the way the render
        /// loop does.
        fn wait_for_idle(&mut self) {
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            while self.app.is_busy() && Instant::now() < deadline {
                self.app.poll_bulk(&mut self.browser, &LocalVfs).unwrap();
                self.app.poll_nav(&mut self.browser);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(!self.app.is_busy(), "command did not finish in time");
        }

        /// Presses one key at the open prompt, as the event loop would.
        fn press(&mut self, code: KeyCode) {
            let flow = self
                .app
                .handle_prompt_key(code, &LocalVfs, &mut self.browser)
                .unwrap();
            assert_eq!(flow, ControlFlow::Continue(()));
        }

        fn type_text(&mut self, text: &str) {
            for c in text.chars() {
                self.press(KeyCode::Char(c));
            }
        }

        /// Waits for the `/` prompt's background walk to finish, applying its result the way the
        /// render loop does.
        fn wait_for_search(&mut self) {
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            while self.app.search_job.is_some() && Instant::now() < deadline {
                self.app.poll_search(&mut self.browser, &LocalVfs);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(
                self.app.search_job.is_none(),
                "search did not finish in time"
            );
        }

        fn selected_name(&self) -> String {
            self.browser.selected_entry().unwrap().name.clone()
        }

        fn listed(&self) -> Vec<String> {
            self.browser
                .current_entries()
                .iter()
                .map(|e| e.name.clone())
                .collect()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(self.archive_log());
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// Chooses an entry the only way there is: puts the cursor on it and marks it, as `v` does.
    fn select_named(f: &mut Fixture, name: &str) {
        let index = f.listed().iter().position(|n| n == name).unwrap();
        f.browser.select_index(index);
        let path = f.root.join(name);
        if !f.browser.is_marked(&path) {
            f.browser.toggle_mark();
        }
    }

    #[test]
    fn compress_and_extract_hand_their_work_to_the_archive_plugin_and_finish_on_its_word() {
        let mut f = Fixture::new("archive-round-trip").with_archiver();
        std::fs::create_dir_all(f.root.join("proj/inner")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        select_named(&mut f, "proj");

        f.run("compress proj.tar.gz");
        assert!(f.app.is_busy(), "the plugin has the work");
        let request = f.request(1);
        let work = &request["work"];
        assert_eq!(work["op"], "compress");
        assert_eq!(work["format"], "tar_gz");
        assert_eq!(work["policy"], "abort");
        assert_eq!(
            work["jobs"],
            serde_json::json!([{
                "sources": [f.root.join("proj")],
                "out": f.root.join("proj.tar.gz"),
            }])
        );
        f.archive_ends(1, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
        assert_eq!(f.app.status.as_deref(), Some("compress complete"));

        std::fs::write(f.root.join("proj.tar.gz"), b"").unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        select_named(&mut f, "proj.tar.gz");
        f.run("extract");
        // The folder the archive would go into is the original `proj`, which is still there.
        assert!(matches!(
            f.app.prompt,
            Some(Prompt::Conflict(ConflictSource::Extract { .. }))
        ));
        f.press(KeyCode::Char('o'));
        let request = f.request(2);
        let work = &request["work"];
        assert_eq!(work["op"], "extract");
        assert_eq!(work["policy"], "overwrite");
        assert_eq!(
            work["jobs"],
            serde_json::json!([{
                "archive": f.root.join("proj.tar.gz"),
                "dest": f.root.join("proj"),
            }])
        );
        f.archive_ends(2, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
        assert_eq!(f.app.status.as_deref(), Some("extract complete"));
    }

    #[test]
    fn the_plugins_progress_reaches_the_gauge_and_a_stale_answer_is_dropped() {
        let mut f = Fixture::new("archive-progress").with_archiver();
        f.run("touch a.txt");
        select_named(&mut f, "a.txt");
        f.run("compress a.zip");
        let job = f.request(1)["job"].as_u64().unwrap();

        f.app.archive_progress(ArchiveProgress {
            job,
            done: 7,
            path: "x.txt".into(),
        });
        f.app.archive_progress(ArchiveProgress {
            job: job + 1,
            done: 99,
            path: "old.txt".into(),
        });
        f.app.poll_bulk(&mut f.browser, &LocalVfs).unwrap();
        let progress = f.app.progress().expect("still running");
        assert_eq!((progress.label, progress.done), ("compressing", 7));

        // An answer to another request does not end this one.
        f.app.archive_done(ArchiveDone {
            job: job + 1,
            finished: vec![0],
            status: ArchiveStatus::Completed,
        });
        f.app.poll_bulk(&mut f.browser, &LocalVfs).unwrap();
        assert!(f.app.is_busy());
        f.archive_ends(1, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
    }

    #[test]
    fn a_failure_the_plugin_reports_is_shown_in_its_words() {
        let mut f = Fixture::new("archive-failed").with_archiver();
        f.run("touch a.txt");
        select_named(&mut f, "a.txt");
        f.run("compress a.zip");

        f.archive_ends(
            1,
            &[],
            ArchiveStatus::Failed {
                message: "a.zip already exists".into(),
            },
        );
        f.wait_for_idle();

        assert_eq!(
            f.app.status.as_deref(),
            Some("compress failed: a.zip already exists")
        );
    }

    #[test]
    fn escape_asks_the_plugin_to_stop_and_its_cancelled_result_ends_the_operation() {
        let mut f = Fixture::new("archive-cancel").with_archiver();
        f.run("touch a.txt");
        select_named(&mut f, "a.txt");
        f.run("compress a.zip");
        let job = f.request(1)["job"].as_u64().unwrap();

        f.app.cancel_bulk();

        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while f.recorded("archive_cancel").is_empty() {
            assert!(
                Instant::now() < deadline,
                "the plugin was never told to stop"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            f.recorded("archive_cancel")[0]["archive_cancel"]["job"],
            job
        );
        assert!(f.app.is_busy(), "it is over when the plugin says so");
        f.archive_ends(1, &[], ArchiveStatus::Cancelled);
        f.wait_for_idle();
        assert_eq!(f.app.status.as_deref(), Some("compress cancelled"));
    }

    #[test]
    fn with_no_archive_plugin_an_archive_that_will_not_open_says_how_to_get_the_plugin_first() {
        let f = Fixture::new("archive-open-missing");
        let error = shared::VfsError::NotFound(std::path::PathBuf::from("/some/long/path/a.zip!"));
        let said = f.app.archive_open_failure(&error);
        // The status line cuts the end of a long message, so the plugin must come before any path.
        assert!(
            said.starts_with("archives are a plugin and mman-archive"),
            "{said}"
        );
        assert!(
            said.contains("cargo install --path plugins/archive"),
            "{said}"
        );
    }

    #[test]
    fn with_no_archive_plugin_compress_and_extract_say_how_to_get_it_and_start_nothing() {
        let mut f = Fixture::new("archive-missing");
        f.run("touch a.txt out.zip");
        select_named(&mut f, "a.txt");

        f.run("compress b.zip");
        let status = f.app.status.clone().expect("it says something");
        assert!(status.contains("mman-archive"), "{status}");
        assert!(!f.app.is_busy());

        select_named(&mut f, "out.zip");
        f.run("extract");
        let status = f.app.status.clone().expect("it says something");
        assert!(status.contains("mman-archive"), "{status}");
        assert!(!f.app.is_busy());
    }

    #[test]
    fn a_path_on_another_machine_or_inside_an_archive_is_refused_by_name() {
        let inside = shared::mount_root(Path::new("/tmp/some.zip")).join("a.txt");
        let refusal = App::archive_refusal("compress", [inside.as_path()]).expect("refused");
        assert!(
            refusal.contains("a.txt") && refusal.contains("inside an archive"),
            "{refusal}"
        );
        let remote = Path::new("ssh://box/dir/x.zip");
        let refusal = App::archive_refusal("extract", [remote]).expect("refused");
        assert!(refusal.contains("a remote machine"), "{refusal}");
        assert_eq!(
            App::archive_refusal("extract", [Path::new("/home/me/x.zip")]),
            None
        );
    }

    #[test]
    fn a_plugin_that_stops_without_an_answer_ends_the_operation_after_a_moment() {
        let mut f = Fixture::new("archive-dies");
        let spec = PluginSpec {
            name: "archive-mortal".to_string(),
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "read line; exit 0".to_string()],
            on_keys: Vec::new(),
            commands: Vec::new(),
            events: Vec::new(),
            provides: vec!["archive".to_string()],
        };
        let handle = f._runtime.handle().clone();
        f.app = std::mem::replace(&mut f.app, App::new(handle)).with_plugins(vec![spec], &f.root);
        f.run("touch a.txt");
        select_named(&mut f, "a.txt");
        f.run("compress a.zip");
        assert!(f.app.is_busy());

        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while f.app.plugins.archiver_running() {
            assert!(
                Instant::now() < deadline,
                "the stand-in plugin never exited"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let now = Instant::now();
        f.app.check_archiver(now);
        assert!(
            f.app.is_busy(),
            "it is given a moment: its last words may be on the way"
        );
        f.app
            .check_archiver(now + ARCHIVER_GRACE + std::time::Duration::from_millis(1));
        f.wait_for_idle();

        assert_eq!(
            f.app.status.as_deref(),
            Some("compress failed: the archive plugin stopped before it finished")
        );
    }

    #[test]
    fn only_the_jobs_the_plugin_finished_have_their_originals_trashed() {
        let mut f = Fixture::new("archive-trash-finished").with_archiver();
        std::fs::write(f.root.join("a.txt"), b"alpha").unwrap();
        std::fs::write(f.root.join("b.txt"), b"beta").unwrap();
        let mut req = request(&f, &["a.txt", "b.txt"], compress_popup::Layout::PerItem);
        req.delete_originals = true;

        f.app.start_compress(req).unwrap();
        // The first archive was written; the second job failed.
        f.archive_ends(
            1,
            &[0],
            ArchiveStatus::Failed {
                message: "disk full".into(),
            },
        );
        f.wait_for_idle();

        assert!(
            !f.root.join("a.txt").exists(),
            "a.txt was archived, so it is in the trash"
        );
        assert!(
            f.root.join("b.txt").exists(),
            "b.txt was not archived, so it stays"
        );
        assert_eq!(f.app.status.as_deref(), Some("compress failed: disk full"));
    }

    #[test]
    fn extract_and_compress_report_what_is_wrong_instead_of_running() {
        let mut f = Fixture::new("archive-errors");
        f.run("touch plain.txt");
        select_named(&mut f, "plain.txt");

        f.run("extract");
        assert_eq!(
            f.app.status.as_deref(),
            Some("extract: no zip, tar or tar.gz marked — mark with v first")
        );
        f.run("compress plain.rar");
        assert_eq!(
            f.app.status.as_deref(),
            Some("compress: name must end in .zip, .tar or .tar.gz")
        );
        assert!(!f.app.is_busy());
    }

    #[test]
    fn compress_asks_before_replacing_an_existing_archive_and_abort_leaves_it() {
        let mut f = Fixture::new("archive-exists");
        f.run("touch a.txt out.zip");
        select_named(&mut f, "a.txt");

        f.run("compress out.zip");
        assert!(matches!(
            f.app.prompt,
            Some(Prompt::Conflict(ConflictSource::Compress { .. }))
        ));
        assert!(
            !f.app.is_busy(),
            "nothing runs until the question is answered"
        );
        f.press(KeyCode::Char('a'));

        assert_eq!(f.app.status.as_deref(), Some("aborted"));
        assert_eq!(std::fs::read(f.root.join("out.zip")).unwrap(), b"");
    }

    #[test]
    fn answering_overwrite_asks_the_plugin_to_replace_the_existing_archive() {
        let mut f = Fixture::new("archive-overwrite").with_archiver();
        f.run("touch a.txt out.zip");
        select_named(&mut f, "a.txt");

        f.run("compress out.zip");
        f.press(KeyCode::Char('o'));

        assert_eq!(f.request(1)["work"]["policy"], "overwrite");
        f.archive_ends(1, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
        assert_eq!(f.app.status.as_deref(), Some("compress complete"));
    }

    #[test]
    fn answering_skip_keeps_the_existing_archive_and_the_originals() {
        let mut f = Fixture::new("archive-skip");
        f.run("touch a.txt out.zip");
        select_named(&mut f, "a.txt");

        f.run("compress out.zip");
        f.press(KeyCode::Char('s'));

        assert_eq!(f.app.status.as_deref(), Some("compress skipped"));
        assert!(!f.app.is_busy());
        assert_eq!(std::fs::read(f.root.join("out.zip")).unwrap(), b"");
        assert!(f.root.join("a.txt").exists());
    }

    #[test]
    fn extracting_into_a_folder_that_is_there_asks_and_skip_is_passed_to_the_plugin() {
        let mut f = fixture_with_archive("extract-ask-skip");
        std::fs::create_dir_all(f.root.join("proj")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        select_named(&mut f, "proj.tar.gz");

        f.run("extract");
        assert!(matches!(
            f.app.prompt,
            Some(Prompt::Conflict(ConflictSource::Extract { .. }))
        ));
        f.press(KeyCode::Char('s'));

        assert_eq!(f.request(1)["work"]["policy"], "skip");
        f.archive_ends(1, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
    }

    #[test]
    fn extracting_into_a_folder_that_is_there_and_aborting_sends_nothing_to_the_plugin() {
        let mut f = fixture_with_archive("extract-ask-abort");
        std::fs::create_dir(f.root.join("proj")).unwrap();

        f.run("extract");
        f.press(KeyCode::Char('a'));

        assert_eq!(f.app.status.as_deref(), Some("aborted"));
        assert!(!f.app.is_busy());
        assert!(f.recorded("archive").is_empty());
    }

    #[test]
    fn compress_with_p_asks_twice_and_sends_the_password_with_the_zip() {
        let mut f = Fixture::new("compress-p").with_archiver();
        f.run("touch a.txt");
        select_named(&mut f, "a.txt");

        f.run("compress -p sealed.zip");
        assert!(matches!(
            f.app.prompt,
            Some(Prompt::CompressPassword { .. })
        ));
        f.type_text("hunter2");
        f.press(KeyCode::Enter);
        let shown = f.app.prompt.as_ref().unwrap().display();
        assert!(shown.starts_with("confirm password"), "{shown}");
        assert!(
            f.recorded("archive").is_empty(),
            "nothing is sent before it is confirmed"
        );
        f.type_text("hunter2");
        f.press(KeyCode::Enter);

        let request = f.request(1);
        assert_eq!(request["work"]["format"], "zip");
        assert_eq!(request["work"]["password"], "hunter2");
        f.archive_ends(1, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
        assert_eq!(f.app.status.as_deref(), Some("compress complete"));
    }

    #[test]
    fn a_mismatched_confirmation_starts_the_password_over_and_writes_nothing() {
        let mut f = Fixture::new("compress-p-mismatch");
        f.run("touch a.txt");
        select_named(&mut f, "a.txt");

        f.run("compress -p sealed.zip");
        f.type_text("one");
        f.press(KeyCode::Enter);
        f.type_text("two");
        f.press(KeyCode::Enter);

        let Some(Prompt::CompressPassword { first, error, .. }) = &f.app.prompt else {
            panic!("the prompt stays open");
        };
        assert!(first.is_none(), "it starts over from the first password");
        assert_eq!(error.as_deref(), Some("the passwords do not match"));
        assert!(!f.app.is_busy());
        assert!(!f.root.join("sealed.zip").exists());
    }

    #[test]
    fn compress_with_p_refuses_a_tar_name_and_escape_cancels_the_prompt() {
        let mut f = Fixture::new("compress-p-tar");
        f.run("touch a.txt");
        select_named(&mut f, "a.txt");

        f.run("compress -p out.tar.gz");
        assert_eq!(
            f.app.status.as_deref(),
            Some("compress: only a zip can be encrypted")
        );
        assert!(f.app.prompt.is_none());

        f.run("compress -p out.zip");
        f.press(KeyCode::Esc);
        assert_eq!(f.app.status.as_deref(), Some("compress cancelled"));
        assert!(!f.root.join("out.zip").exists());
    }

    #[test]
    fn the_compress_form_can_save_into_another_folder_and_refuses_one_that_is_not_there() {
        let mut f = Fixture::new("compress-save-in").with_archiver();
        std::fs::create_dir(f.root.join("backups")).unwrap();
        std::fs::write(f.root.join("a.txt"), b"a").unwrap();
        let mut req = request(
            &f,
            &["a.txt"],
            compress_popup::Layout::One {
                file_name: "a.tar.gz".into(),
            },
        );
        req.dir = f.root.join("backups");

        f.app.start_compress(req).unwrap();

        assert_eq!(
            f.request(1)["work"]["jobs"][0]["out"],
            f.root.join("backups/a.tar.gz").to_str().unwrap()
        );
        f.archive_ends(1, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();

        let mut missing = request(
            &f,
            &["a.txt"],
            compress_popup::Layout::One {
                file_name: "b.tar.gz".into(),
            },
        );
        missing.dir = f.root.join("nowhere");
        let error = f.app.start_compress(missing).unwrap_err();
        assert!(error.ends_with("nowhere is not a folder"), "{error}");
    }

    #[test]
    fn the_compress_form_warns_live_about_a_name_already_taken_and_a_missing_folder() {
        let mut f = Fixture::new("compress-warn");
        f.run("touch a.txt a.zip");
        select_named(&mut f, "a.txt");
        let mut form = f.app.begin_compress_form(&f.browser).unwrap();
        assert_eq!(form.warning(), Some("a.zip already exists"));

        // A different name clears it; a folder that is not there is reported instead.
        form.key(KeyCode::Char('2'));
        f.app.refresh_compress_warning(&mut form);
        assert_eq!(form.warning(), None);
        form.key(KeyCode::Down);
        for c in "nope".chars() {
            form.key(KeyCode::Char(c));
        }
        f.app.refresh_compress_warning(&mut form);
        assert!(form.warning().unwrap().ends_with("nope is not a folder"));
    }

    #[test]
    fn the_extract_form_warns_live_about_a_folder_that_is_there_until_existing_files_are_handled() {
        let mut f = fixture_with_archive("extract-warn");
        std::fs::create_dir(f.root.join("proj")).unwrap();
        let mut form = f.app.begin_extract_form(&f.browser).unwrap();
        assert!(form.warning().unwrap().starts_with("proj already exists"));

        // Existing files -> skip: merging is now what is asked for, so nothing to warn about.
        form.key(KeyCode::Down);
        form.key(KeyCode::Right);
        f.app.refresh_extract_warning(&mut form);
        assert_eq!(form.warning(), None);
    }

    fn request(
        f: &Fixture,
        names: &[&str],
        layout: compress_popup::Layout,
    ) -> compress_popup::Request {
        compress_popup::Request {
            sources: names.iter().map(|n| f.root.join(n)).collect(),
            dir: f.root.clone(),
            format: Format::TarGz,
            level: Level::Best,
            password: None,
            layout,
            delete_originals: false,
        }
    }

    /// A fixture holding `proj.tar.gz`, made from a folder that held `inner/a.txt`.
    /// A fixture holding `proj.tar.gz` and a recording archive plugin. The file is an empty
    /// stand-in: the interface goes by its name, and opening it is the plugin's business.
    fn fixture_with_archive(name: &str) -> Fixture {
        let mut f = Fixture::new(name).with_archiver();
        std::fs::write(f.root.join("proj.tar.gz"), b"").unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        select_named(&mut f, "proj.tar.gz");
        f
    }

    fn extract_request(
        f: &Fixture,
        destination: extract_popup::Destination,
        policy: ConflictPolicy,
        delete_archives: bool,
    ) -> extract_popup::Request {
        extract_popup::Request {
            archives: vec![f.root.join("proj.tar.gz")],
            dir: f.root.clone(),
            destination,
            policy,
            delete_archives,
        }
    }

    #[test]
    fn the_extract_form_opens_only_for_archives() {
        let mut f = Fixture::new("extract-form-none");
        f.run("touch plain.txt");
        select_named(&mut f, "plain.txt");
        assert!(f.app.begin_extract_form(&f.browser).is_none());
        assert_eq!(
            f.app.status.as_deref(),
            Some("extract: no zip, tar or tar.gz marked — mark with v first")
        );

        let mut f = fixture_with_archive("extract-form-some");
        assert!(f.app.begin_extract_form(&f.browser).is_some());
    }

    #[test]
    fn the_extract_form_can_unpack_into_a_named_folder() {
        let mut f = fixture_with_archive("extract-named");
        let request = extract_request(
            &f,
            extract_popup::Destination::Folder("out".into()),
            ConflictPolicy::Abort,
            false,
        );
        f.app.start_extract(request).unwrap();

        assert_eq!(
            f.request(1)["work"]["jobs"][0]["dest"],
            f.root.join("out").to_str().unwrap()
        );
        f.archive_ends(1, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
        assert_eq!(f.app.status.as_deref(), Some("extract complete"));
        // The archive stays unless asked otherwise.
        assert!(f.root.join("proj.tar.gz").exists());
    }

    #[test]
    fn the_extract_form_can_unpack_here_and_trash_the_archive() {
        let mut f = fixture_with_archive("extract-here");
        let request = extract_request(
            &f,
            extract_popup::Destination::Here,
            ConflictPolicy::Abort,
            true,
        );
        f.app.start_extract(request).unwrap();

        assert_eq!(
            f.request(1)["work"]["jobs"][0]["dest"],
            f.root.to_str().unwrap()
        );
        assert!(
            f.root.join("proj.tar.gz").exists(),
            "the archive is trashed once it has been extracted, not before"
        );
        f.archive_ends(1, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
        assert_eq!(f.app.status.as_deref(), Some("extract complete"));
        assert!(!f.root.join("proj.tar.gz").exists());
    }

    #[test]
    fn stopping_on_existing_files_will_not_enter_a_folder_that_is_there() {
        let mut f = fixture_with_archive("extract-taken");
        std::fs::create_dir(f.root.join("out")).unwrap();
        let request = extract_request(
            &f,
            extract_popup::Destination::Folder("out".into()),
            ConflictPolicy::Abort,
            false,
        );
        assert_eq!(
            f.app.start_extract(request),
            Err("out already exists: set Existing files to skip or replace".into())
        );
        assert!(!f.app.is_busy());
        assert!(f.recorded("archive").is_empty());

        // "Replace" is how a folder that is there gets filled.
        let request = extract_request(
            &f,
            extract_popup::Destination::Folder("out".into()),
            ConflictPolicy::Overwrite,
            false,
        );
        f.app.start_extract(request).unwrap();
        assert_eq!(f.request(1)["work"]["policy"], "overwrite");
        f.archive_ends(1, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
    }

    #[test]
    fn the_compress_form_makes_one_archive_of_the_marked_items() {
        let mut f = Fixture::new("form-one").with_archiver();
        std::fs::write(f.root.join("a.txt"), b"alpha").unwrap();
        std::fs::write(f.root.join("b.txt"), b"beta").unwrap();
        f.browser.reload(&LocalVfs).unwrap();

        let layout = compress_popup::Layout::One {
            file_name: "both.tar.gz".into(),
        };
        f.app
            .start_compress(request(&f, &["a.txt", "b.txt"], layout))
            .unwrap();
        assert!(f.app.is_busy());

        let request = f.request(1);
        let work = &request["work"];
        assert_eq!(work["level"], "best");
        assert_eq!(
            work["jobs"],
            serde_json::json!([{
                "sources": [f.root.join("a.txt"), f.root.join("b.txt")],
                "out": f.root.join("both.tar.gz"),
            }])
        );
        f.archive_ends(1, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
        assert_eq!(f.app.status.as_deref(), Some("compress complete"));
        // Nothing was deleted: the originals stay unless asked otherwise.
        assert!(f.root.join("a.txt").exists() && f.root.join("b.txt").exists());
    }

    #[test]
    fn the_compress_form_can_make_one_archive_per_item() {
        let mut f = Fixture::new("form-per-item").with_archiver();
        std::fs::write(f.root.join("a.txt"), b"alpha").unwrap();
        std::fs::create_dir(f.root.join("dir")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();

        f.app
            .start_compress(request(
                &f,
                &["a.txt", "dir"],
                compress_popup::Layout::PerItem,
            ))
            .unwrap();

        let request = f.request(1);
        assert_eq!(
            request["work"]["jobs"],
            serde_json::json!([
                { "sources": [f.root.join("a.txt")], "out": f.root.join("a.txt.tar.gz") },
                { "sources": [f.root.join("dir")], "out": f.root.join("dir.tar.gz") },
            ])
        );
        f.archive_ends(1, &[0, 1], ArchiveStatus::Completed);
        f.wait_for_idle();
        assert_eq!(f.app.status.as_deref(), Some("compress complete"));
    }

    #[test]
    fn the_compress_form_refuses_a_name_that_is_taken_without_starting_anything() {
        let mut f = Fixture::new("form-taken");
        std::fs::write(f.root.join("a.txt"), b"alpha").unwrap();
        std::fs::write(f.root.join("a.txt.tar.gz"), b"precious").unwrap();

        let error = f
            .app
            .start_compress(request(&f, &["a.txt"], compress_popup::Layout::PerItem))
            .unwrap_err();
        assert_eq!(error, "a.txt.tar.gz already exists");
        assert!(!f.app.is_busy());
        assert_eq!(
            std::fs::read(f.root.join("a.txt.tar.gz")).unwrap(),
            b"precious"
        );
    }

    #[test]
    fn opening_the_compress_form_with_nothing_selected_says_so() {
        let mut f = Fixture::new("form-empty");
        assert!(f.app.begin_compress_form(&f.browser).is_none());
        assert_eq!(
            f.app.status.as_deref(),
            Some("compress: nothing marked — mark with v first")
        );
    }

    #[test]
    fn a_second_compress_cannot_start_while_one_runs() {
        let mut f = Fixture::new("form-busy").with_archiver();
        std::fs::write(f.root.join("a.txt"), b"alpha").unwrap();
        let first = compress_popup::Layout::One {
            file_name: "one.zip".into(),
        };
        let second = compress_popup::Layout::One {
            file_name: "two.zip".into(),
        };
        f.app
            .start_compress(request(&f, &["a.txt"], first))
            .unwrap();
        let refused = f.app.start_compress(request(&f, &["a.txt"], second));
        assert_eq!(refused, Err("an operation is already in progress".into()));
        f.archive_ends(1, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
        assert_eq!(
            f.recorded("archive").len(),
            1,
            "the second never reached the plugin"
        );
    }

    #[test]
    fn mkdir_and_touch_create_things_and_show_them_at_once() {
        let mut f = Fixture::new("builtins");

        f.run("mkdir -p deep/er");
        f.run("touch notes.txt 'two words.txt'");

        assert!(f.root.join("deep/er").is_dir());
        assert!(f.root.join("two words.txt").is_file());
        assert_eq!(f.listed(), vec!["deep", "notes.txt", "two words.txt"]);
        assert_eq!(f.app.status.as_deref(), Some("touch: 2 done"));
    }

    #[test]
    fn a_failing_name_is_reported_but_does_not_stop_the_rest() {
        let mut f = Fixture::new("partial");
        std::fs::write(f.root.join("taken"), b"x").unwrap();

        f.run("mkdir taken fresh");

        assert!(f.root.join("fresh").is_dir());
        let status = f.app.status.clone().unwrap();
        assert!(status.starts_with("mkdir taken: "), "{status}");
        assert!(status.ends_with("(1 of 2 done)"), "{status}");
    }

    #[test]
    fn plugin_for_an_unknown_name_says_so_instead_of_running_a_shell_command() {
        let mut f = Fixture::new("plugin-unknown");

        f.run("plugin blame a.rs");

        assert_eq!(
            f.app.status.as_deref(),
            Some("plugin blame: no plugins are running")
        );
        assert!(!f.app.is_busy());
    }

    #[test]
    fn cd_and_quit_still_work() {
        let mut f = Fixture::new("cd");
        std::fs::create_dir(f.root.join("sub")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();

        f.run("cd sub");
        f.wait_for_idle();
        assert_eq!(f.browser.current_dir(), f.root.join("sub"));
        assert_eq!(f.app.status.as_deref(), Some("cd sub"));
        assert_eq!(f.flow("quit"), ControlFlow::Break(()));
    }

    fn press_key(f: &mut Fixture, code: KeyCode, modifiers: crossterm::event::KeyModifiers) {
        let _ = f
            .app
            .handle_prompt_event(KeyEvent::new(code, modifiers), &LocalVfs, &mut f.browser)
            .unwrap();
    }

    fn type_text(f: &mut Fixture, text: &str) {
        for c in text.chars() {
            press_key(f, KeyCode::Char(c), crossterm::event::KeyModifiers::NONE);
        }
    }

    #[test]
    fn a_rename_can_be_fixed_in_the_middle_of_the_name() {
        use crossterm::event::KeyModifiers as M;
        let mut f = Fixture::new("edit-rename");
        std::fs::write(f.root.join("reprot.txt"), b"x").unwrap();
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_rename(&f.browser);
        // "reprot.txt" -> "report.txt": go back to after "repro", delete "ro", type "or".
        for _ in 0..5 {
            press_key(&mut f, KeyCode::Left, M::NONE);
        }
        press_key(&mut f, KeyCode::Backspace, M::NONE);
        press_key(&mut f, KeyCode::Backspace, M::NONE);
        type_text(&mut f, "or");
        let shown = f.app.status_line_with(Some("|"));
        assert_eq!(shown, "rename: repor|t.txt");
        press_key(&mut f, KeyCode::Enter, M::NONE);

        assert!(f.root.join("report.txt").exists(), "{:?}", f.app.status);
        assert!(!f.root.join("reprot.txt").exists());
    }

    #[test]
    fn the_command_bar_edits_with_home_end_and_control_keys() {
        use crossterm::event::KeyModifiers as M;
        let mut f = Fixture::new("edit-command");
        f.app.begin_command();
        type_text(&mut f, "mkdir old name");
        press_key(&mut f, KeyCode::Home, M::NONE);
        for _ in 0..5 {
            press_key(&mut f, KeyCode::Right, M::NONE);
        }
        assert_eq!(f.app.status_line_with(Some("|")), ":mkdir| old name");
        press_key(&mut f, KeyCode::Char('k'), M::CONTROL);
        assert_eq!(f.app.status_line_with(Some("|")), ":mkdir|");
        type_text(&mut f, " fresh");
        press_key(&mut f, KeyCode::Char('w'), M::CONTROL);
        type_text(&mut f, "tidy");
        press_key(&mut f, KeyCode::Enter, M::NONE);
        f.wait_for_idle();
        assert!(f.root.join("tidy").is_dir(), "{:?}", f.app.status);
    }

    #[test]
    fn moving_the_cursor_in_a_search_does_not_start_another_search() {
        use crossterm::event::KeyModifiers as M;
        let mut f = Fixture::new("edit-search");
        std::fs::create_dir(f.root.join("alpha")).unwrap();
        std::fs::create_dir(f.root.join("beta")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        type_text(&mut f, "bet");
        assert_eq!(
            f.browser.selected_entry().map(|e| e.name.as_str()),
            Some("beta")
        );
        // Select something else by hand, then only move the cursor: the search must not jump back.
        f.browser.select_index(0);
        press_key(&mut f, KeyCode::Left, M::NONE);
        press_key(&mut f, KeyCode::Home, M::NONE);
        assert_eq!(f.browser.selected_index(), 0);
        // Changing the text does search again.
        press_key(&mut f, KeyCode::Delete, M::NONE);
        assert_eq!(f.app.status_line_with(Some("|")), "/|et");
    }

    #[test]
    fn a_folder_opens_in_the_background_and_the_old_one_stays_until_it_is_ready() {
        let mut f = Fixture::new("nav-bg");
        f.app.nav_grace = Duration::ZERO;
        std::fs::create_dir(f.root.join("sub")).unwrap();

        f.run("cd sub");
        assert!(f.app.is_busy(), "the move is running");
        assert_eq!(
            f.browser.current_dir(),
            f.root,
            "nothing moves until the result is applied"
        );
        assert_eq!(f.app.progress().map(|p| p.label), Some("opening"));

        f.wait_for_idle();
        assert_eq!(f.browser.current_dir(), f.root.join("sub"));
    }

    #[test]
    fn a_local_folder_is_open_by_the_time_begin_move_returns() {
        let mut f = Fixture::new("nav-fast");
        std::fs::create_dir(f.root.join("sub")).unwrap();

        f.run("cd sub");
        assert!(!f.app.is_busy(), "no key would be held back");
        assert_eq!(f.browser.current_dir(), f.root.join("sub"));
    }

    #[test]
    fn a_folder_that_cannot_be_opened_says_so_and_leaves_the_browser_alone() {
        let mut f = Fixture::new("nav-fail");

        f.run("cd missing");
        f.wait_for_idle();
        assert_eq!(f.browser.current_dir(), f.root);
        let status = f.app.status.clone().unwrap();
        assert!(status.starts_with("cd failed: "), "{status}");
    }

    #[test]
    fn cancelling_drops_a_move_still_loading() {
        let mut f = Fixture::new("nav-cancel");
        f.app.nav_grace = Duration::ZERO;
        std::fs::create_dir(f.root.join("sub")).unwrap();

        f.run("cd sub");
        f.app.cancel_bulk();
        assert!(!f.app.is_busy());
        assert_eq!(f.app.status.as_deref(), Some("cancelled"));
        // A result that arrives anyway has nobody to apply it.
        std::thread::sleep(std::time::Duration::from_millis(50));
        f.app.poll_nav(&mut f.browser);
        assert_eq!(f.browser.current_dir(), f.root);
    }

    #[test]
    fn a_second_move_is_ignored_while_one_is_loading() {
        let mut f = Fixture::new("nav-second");
        f.app.nav_grace = Duration::ZERO;
        std::fs::create_dir(f.root.join("a")).unwrap();
        std::fs::create_dir(f.root.join("b")).unwrap();

        f.run("cd a");
        f.run("cd b");
        f.wait_for_idle();
        assert_eq!(f.browser.current_dir(), f.root.join("a"));
    }

    #[test]
    fn any_other_command_runs_in_the_shell_and_its_effects_appear() {
        let mut f = Fixture::new("shell");

        f.run("echo hi > made-by-sh.txt && echo done");
        assert!(f.app.is_busy());
        f.wait_for_idle();

        assert_eq!(f.app.status.as_deref(), Some("done"));
        assert_eq!(f.listed(), vec!["made-by-sh.txt"]);
    }

    #[test]
    fn a_failed_shell_command_reports_its_exit_code() {
        let mut f = Fixture::new("shell-fail");

        f.run("exit 4");
        f.wait_for_idle();

        assert_eq!(f.app.status.as_deref(), Some("exit 4: exit 4"));
    }

    #[test]
    fn escape_cancels_a_running_shell_command() {
        let mut f = Fixture::new("shell-cancel");

        f.run("sleep 30");
        assert!(f.app.is_busy());
        f.app.cancel_bulk();
        f.wait_for_idle();

        assert_eq!(f.app.status.as_deref(), Some("cancelled: sleep 30"));
    }

    #[test]
    fn a_listed_program_asks_for_the_terminal_instead_of_running_captured() {
        let mut f = Fixture::new("handover-listed");

        f.run("nvim ROADMAP.md");

        assert!(
            !f.app.is_busy(),
            "nothing runs until main hands the terminal over"
        );
        assert_eq!(
            f.app.take_handover(),
            Some(Handover {
                line: "nvim ROADMAP.md".into(),
                cwd: f.root.clone(),
            })
        );
        assert_eq!(f.app.take_handover(), None, "a handover is taken once");
    }

    #[test]
    fn a_bang_asks_for_the_terminal_for_any_program() {
        let mut f = Fixture::new("handover-bang");

        f.run("!python3 -i");

        let handover = f.app.take_handover().expect("a handover was requested");
        assert_eq!(handover.line, "python3 -i");
    }

    #[test]
    fn a_handover_is_refused_while_another_operation_runs() {
        let mut f = Fixture::new("handover-busy");

        f.run("sleep 30");
        f.run("nvim notes.md");

        assert_eq!(f.app.take_handover(), None);
        assert!(
            f.app
                .status
                .clone()
                .unwrap()
                .contains("already in progress")
        );
        f.app.cancel_bulk();
        f.wait_for_idle();
    }

    #[test]
    fn finishing_a_handover_reports_how_it_ended_and_shows_what_it_changed() {
        use std::os::unix::process::ExitStatusExt;
        let mut f = Fixture::new("handover-finish");
        f.run("nvim x.md");
        let handover = f.app.take_handover().unwrap();
        std::fs::write(f.root.join("x.md"), b"saved by the editor").unwrap();

        f.app
            .finish_handover(
                &mut f.browser,
                &LocalVfs,
                &handover,
                &Ok(ExitStatus::from_raw(0)),
            )
            .unwrap();
        assert_eq!(f.app.status.as_deref(), Some("nvim x.md: done"));
        assert_eq!(f.listed(), vec!["x.md"]);

        f.app
            .finish_handover(
                &mut f.browser,
                &LocalVfs,
                &handover,
                &Ok(ExitStatus::from_raw(3 << 8)),
            )
            .unwrap();
        assert_eq!(
            f.app.status.as_deref(),
            Some("nvim x.md: exited with code 3")
        );

        let missing = Err(std::io::Error::from(std::io::ErrorKind::NotFound));
        f.app
            .finish_handover(&mut f.browser, &LocalVfs, &handover, &missing)
            .unwrap();
        assert!(
            f.app
                .status
                .clone()
                .unwrap()
                .starts_with("could not run 'nvim x.md'")
        );
    }

    fn three_files(label: &str) -> Fixture {
        let mut f = Fixture::new(label);
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(f.root.join(name), b"x").unwrap();
        }
        f.browser.reload(&LocalVfs).unwrap();
        f
    }

    #[test]
    fn with_nothing_marked_the_archive_operations_still_need_marks() {
        let mut f = three_files("marks-only");
        f.browser.select_index(1);

        assert!(f.app.begin_compress_form(&f.browser).is_none());
        assert!(f.app.begin_extract_form(&f.browser).is_none());
        for line in ["compress out.zip", "extract"] {
            f.app.status = None;
            f.run(line);
            assert!(f.app.prompt.is_none(), "{line}");
            assert!(!f.app.is_busy(), "{line}");
            let status = f.app.status.clone().unwrap_or_default();
            assert!(status.contains("mark with v first"), "{line}: {status}");
        }
        assert!(f.root.join("b.txt").exists(), "nothing was touched");
        assert!(!f.root.join("out.zip").exists());
    }

    #[test]
    fn with_nothing_marked_the_keys_act_on_the_cursor_row() {
        let mut f = three_files("cursor-row");
        f.browser.select_index(1);

        f.app.yank_key(&mut f.browser);
        let held = f.app.clipboard.as_ref().expect("yanked");
        assert_eq!(held.paths, [f.root.join("b.txt")]);
        assert_eq!(held.mode, ClipboardMode::Copy);

        f.app.cut_key(&mut f.browser);
        let held = f.app.clipboard.as_ref().expect("cut");
        assert_eq!(held.paths, [f.root.join("b.txt")]);
        assert_eq!(held.mode, ClipboardMode::Move);

        f.app.begin_trash(&f.browser);
        assert!(matches!(
            &f.app.prompt,
            Some(Prompt::ConfirmTrash { targets }) if *targets == [f.root.join("b.txt")]
        ));
        f.press(KeyCode::Char('n'));

        f.app.begin_delete_permanently(&f.browser);
        assert!(matches!(
            &f.app.prompt,
            Some(Prompt::ConfirmDelete { targets }) if *targets == [f.root.join("b.txt")]
        ));
    }

    #[test]
    fn marks_win_over_the_cursor_row_for_a_delete() {
        let mut f = three_files("marks-win");
        select_named(&mut f, "a.txt");
        f.browser.select_index(2);

        f.app.begin_delete_permanently(&f.browser);

        assert!(matches!(
            &f.app.prompt,
            Some(Prompt::ConfirmDelete { targets }) if *targets == [f.root.join("a.txt")]
        ));
    }

    /// A fixture whose root holds `dest/` besides the three files, with `a.txt` and `c.txt`
    /// marked and the browser then moved into `dest/`, where the keys are pressed.
    fn marked_then_moved_to_dest(label: &str) -> Fixture {
        let mut f = three_files(label);
        std::fs::create_dir(f.root.join("dest")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        select_named(&mut f, "a.txt");
        select_named(&mut f, "c.txt");
        f.run("cd dest");
        f.wait_for_idle();
        assert_eq!(f.browser.current_dir(), f.root.join("dest"));
        f
    }

    #[test]
    fn the_yank_key_copies_the_marks_into_the_browsed_folder_at_once() {
        let mut f = marked_then_moved_to_dest("yank-here");

        f.app.yank_key(&mut f.browser);
        f.wait_for_idle();

        assert!(f.root.join("dest/a.txt").exists());
        assert!(f.root.join("dest/c.txt").exists());
        assert!(f.root.join("a.txt").exists(), "a copy leaves the original");
        assert!(f.browser.marked_paths().is_empty(), "the marks are spent");
        assert!(f.app.clipboard.is_none(), "no paste is needed");
    }

    #[test]
    fn the_cut_key_moves_the_marks_into_the_browsed_folder_at_once() {
        let mut f = marked_then_moved_to_dest("cut-here");

        f.app.cut_key(&mut f.browser);
        f.wait_for_idle();

        assert!(f.root.join("dest/a.txt").exists());
        assert!(f.root.join("dest/c.txt").exists());
        assert!(!f.root.join("a.txt").exists());
        assert!(!f.root.join("c.txt").exists());
        assert!(f.root.join("b.txt").exists(), "unmarked entries stay");
        assert!(f.browser.marked_paths().is_empty());
        assert!(f.app.clipboard.is_none());
    }

    #[test]
    fn cutting_marks_that_are_already_in_the_folder_says_so_and_keeps_them() {
        let mut f = three_files("cut-in-place");
        select_named(&mut f, "b.txt");

        f.app.cut_key(&mut f.browser);

        assert_eq!(f.app.status.as_deref(), Some("already in this folder"));
        assert!(!f.app.is_busy());
        assert_eq!(f.browser.marked_paths().len(), 1);
        assert!(f.root.join("b.txt").exists());
    }

    #[test]
    fn yanking_marks_that_are_already_in_the_folder_never_damages_them() {
        let mut f = three_files("yank-in-place");
        select_named(&mut f, "b.txt");

        f.app.yank_key(&mut f.browser);
        f.wait_for_idle();

        assert!(f.root.join("b.txt").exists());
        assert_eq!(std::fs::read(f.root.join("b.txt")).unwrap(), b"x");
        assert!(f.app.prompt.is_none(), "no overwrite is offered on itself");
    }

    #[test]
    fn a_capital_y_confirms_a_delete_and_a_trash_like_a_lowercase_one() {
        let mut f = three_files("capital-y");
        select_named(&mut f, "b.txt");
        f.app.begin_delete_permanently(&f.browser);
        f.press(KeyCode::Char('Y'));
        f.wait_for_idle();
        assert!(!f.root.join("b.txt").exists());

        // A name that does not exist, so a confirmed trash cannot reach the user's real trash.
        f.app.prompt = Some(Prompt::ConfirmTrash {
            targets: vec![f.root.join("ghost.txt")],
        });
        f.press(KeyCode::Char('Y'));
        assert!(f.app.prompt.is_none());
        assert_ne!(f.app.status.as_deref(), Some("trash cancelled"));
        f.wait_for_idle();
    }

    #[test]
    fn a_yank_takes_the_marks_and_nothing_else_and_leaves_none_behind() {
        let mut f = three_files("yank-spends");
        select_named(&mut f, "a.txt");
        select_named(&mut f, "c.txt");
        f.browser.select_index(1); // the cursor rests on b.txt, which is not marked

        f.app.yank(&mut f.browser);

        let held = f.app.clipboard.as_ref().expect("yanked");
        assert_eq!(held.paths, [f.root.join("a.txt"), f.root.join("c.txt")]);
        assert_eq!(held.mode, ClipboardMode::Copy);
        assert!(
            f.browser.marked_paths().is_empty(),
            "y spends the selection"
        );
        // So a following operation falls back to the cursor row, not to the spent marks.
        f.app.begin_trash(&f.browser);
        assert!(matches!(
            &f.app.prompt,
            Some(Prompt::ConfirmTrash { targets }) if *targets == [f.root.join("b.txt")]
        ));
    }

    #[test]
    fn a_cut_takes_the_marks_too() {
        let mut f = three_files("cut-spends");
        select_named(&mut f, "b.txt");

        f.app.cut(&mut f.browser);

        let held = f.app.clipboard.as_ref().expect("cut");
        assert_eq!(held.paths, [f.root.join("b.txt")]);
        assert_eq!(held.mode, ClipboardMode::Move);
        assert!(f.browser.marked_paths().is_empty());
    }

    #[test]
    fn confirming_a_delete_spends_the_marks_and_declining_keeps_them() {
        // The permanent delete is used for the confirming half: a trash would send a real file to
        // the user's desktop trash from a test.
        let mut f = three_files("confirm-spends");
        select_named(&mut f, "b.txt");
        f.app.begin_trash(&f.browser);
        f.press(KeyCode::Char('n'));
        assert_eq!(
            f.browser.marked_paths().len(),
            1,
            "declining a trash keeps the selection"
        );

        f.app.begin_delete_permanently(&f.browser);
        f.press(KeyCode::Char('n'));
        assert_eq!(
            f.browser.marked_paths().len(),
            1,
            "so does declining a delete"
        );
        assert!(f.root.join("b.txt").exists());

        f.app.begin_delete_permanently(&f.browser);
        f.press(KeyCode::Char('y'));
        assert!(f.browser.marked_paths().is_empty(), "confirming spends it");
        f.wait_for_idle();
        assert!(!f.root.join("b.txt").exists());
        assert!(
            f.root.join("c.txt").exists(),
            "unmarked entries are never touched"
        );
    }

    #[test]
    fn compress_and_extract_spend_the_marks_they_use() {
        let mut f = three_files("archive-spends");
        f = f.with_archiver();
        select_named(&mut f, "a.txt");
        f.run("compress out.zip");
        assert!(f.browser.marked_paths().is_empty());
        f.archive_ends(1, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();

        std::fs::write(f.root.join("out.zip"), b"").unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        select_named(&mut f, "out.zip");
        f.run("extract");
        assert!(f.browser.marked_paths().is_empty());
        f.archive_ends(2, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
    }

    #[test]
    fn cancel_forgets_a_cut_and_the_marks_from_any_directory() {
        let mut f = Fixture::new("cancel-all");
        for name in ["a", "b", "c"] {
            std::fs::write(f.root.join(name), b"x").unwrap();
        }
        std::fs::create_dir(f.root.join("elsewhere")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        f.browser.select_index(1);
        f.browser.toggle_mark();
        f.browser.select_index(2);
        f.browser.toggle_mark();
        f.app.cut(&mut f.browser);
        assert!(f.app.clipboard.is_some());
        assert!(
            f.browser.marked_paths().is_empty(),
            "a cut spends the marks it took"
        );
        // A later, separate selection is what `cancel` forgets along with the clipboard.
        f.browser.select_index(3);
        f.browser.toggle_mark();

        // Somewhere else entirely, as the request says: "no matter the directory I am".
        f.browser
            .goto(&LocalVfs, &f.root.join("elsewhere"))
            .unwrap();
        f.app.cancel_all(&mut f.browser);

        assert!(f.app.clipboard.is_none());
        assert!(f.browser.marked_paths().is_empty());
        let status = f.app.status.clone().unwrap();
        assert!(status.starts_with("cancelled: cut of "), "{status}");
        assert!(status.ends_with("1 mark"), "{status}");
        for name in ["a", "b", "c"] {
            assert!(
                f.root.join(name).exists(),
                "a cancelled cut must not touch {name}"
            );
        }
    }

    #[test]
    fn cancel_says_so_when_there_is_nothing_to_cancel() {
        let mut f = Fixture::new("cancel-nothing");
        f.app.cancel_all(&mut f.browser);
        assert_eq!(f.app.status.as_deref(), Some("nothing to cancel"));
    }

    #[test]
    fn cancel_also_stops_a_running_command() {
        let mut f = Fixture::new("cancel-running");
        f.run("sleep 30");
        assert!(f.app.is_busy());

        f.app.cancel_all(&mut f.browser);
        f.wait_for_idle();

        assert_eq!(f.app.status.as_deref(), Some("cancelled: sleep 30"));
    }

    fn deep_tree(f: &Fixture) {
        std::fs::create_dir_all(f.root.join("a").join("b")).unwrap();
        std::fs::write(f.root.join("a").join("b").join("aerend.md"), b"").unwrap();
        std::fs::write(f.root.join("notes.txt"), b"").unwrap();
        std::fs::write(f.root.join("zzz.txt"), b"").unwrap();
    }

    #[test]
    fn a_search_walks_into_subdirectories_and_opens_the_hit() {
        let mut f = Fixture::new("search-deep");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("aerend");
        f.wait_for_search();

        assert_eq!(f.browser.current_dir(), f.root.join("a").join("b"));
        assert_eq!(f.selected_name(), "aerend.md");
    }

    #[test]
    fn a_match_in_the_directory_already_open_is_selected_without_a_walk() {
        let mut f = Fixture::new("search-local");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("notes");

        assert!(f.app.search_job.is_none());
        assert_eq!(f.browser.current_dir(), f.root);
        assert_eq!(f.selected_name(), "notes.txt");
    }

    #[test]
    fn escape_returns_to_the_directory_and_row_the_search_started_from() {
        let mut f = Fixture::new("search-escape");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();
        f.browser.select_index(2);
        let started_on = f.selected_name();

        f.app.begin_search(&f.browser);
        f.type_text("aerend");
        f.wait_for_search();
        assert_ne!(f.browser.current_dir(), f.root);

        f.press(KeyCode::Esc);
        assert_eq!(f.browser.current_dir(), f.root);
        assert_eq!(f.selected_name(), started_on);
        assert_eq!(f.app.status.as_deref(), Some("search cancelled"));
        assert!(f.app.prompt.is_none());
    }

    #[test]
    fn enter_keeps_the_cursor_where_the_search_landed() {
        let mut f = Fixture::new("search-enter");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("aerend");
        f.wait_for_search();
        f.press(KeyCode::Enter);

        assert!(f.app.prompt.is_none());
        assert_eq!(f.browser.current_dir(), f.root.join("a").join("b"));
        assert_eq!(f.selected_name(), "aerend.md");
    }

    fn touch_files(f: &Fixture, names: &[&str]) {
        for name in names {
            std::fs::write(f.root.join(name), b"").unwrap();
        }
    }

    #[test]
    fn letters_in_order_find_a_name_without_them_being_side_by_side() {
        let mut f = Fixture::new("search-fuzzy-here");
        touch_files(&f, &["aerend.txt", "zzz.txt"]);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("aernd");
        assert!(
            f.app.search_job.is_none(),
            "it is in the folder already open"
        );
        assert_eq!(f.selected_name(), "aerend.txt");

        // And a walk finds one further down the same way.
        let mut f = Fixture::new("search-fuzzy-deep");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();
        f.app.begin_search(&f.browser);
        f.type_text("aernd");
        f.wait_for_search();
        assert_eq!(f.selected_name(), "aerend.md");
    }

    #[test]
    fn the_best_match_in_a_folder_wins_over_one_listed_earlier() {
        let mut f = Fixture::new("search-ranked");
        // Listed in this order, so a first-match search would stop on the scattered one.
        touch_files(&f, &["a_e_r_e_n_d.txt", "aerend.txt"]);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("aerend");
        assert_eq!(f.selected_name(), "aerend.txt");
    }

    fn note(f: &Fixture) -> String {
        f.app
            .status_line()
            .split(' ')
            .next()
            .unwrap_or("")
            .to_string()
    }

    #[test]
    fn the_arrows_step_through_the_matches_and_wrap_round() {
        use crossterm::event::KeyModifiers as M;
        let mut f = Fixture::new("search-step");
        touch_files(
            &f,
            &["other.txt", "report-1.txt", "report-2.txt", "report-3.txt"],
        );
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("report");
        assert_eq!(f.selected_name(), "report-1.txt");
        assert_eq!(note(&f), "1/3");

        press_key(&mut f, KeyCode::Down, M::NONE);
        assert_eq!(
            (f.selected_name(), note(&f)),
            ("report-2.txt".into(), "2/3".into())
        );
        press_key(&mut f, KeyCode::Down, M::NONE);
        press_key(&mut f, KeyCode::Down, M::NONE);
        assert_eq!(f.selected_name(), "report-1.txt", "wrapped past the last");
        press_key(&mut f, KeyCode::Up, M::NONE);
        assert_eq!(
            f.selected_name(),
            "report-3.txt",
            "wrapped back past the first"
        );

        press_key(&mut f, KeyCode::Char('p'), M::CONTROL);
        assert_eq!(f.selected_name(), "report-2.txt");
        press_key(&mut f, KeyCode::Char('n'), M::CONTROL);
        assert_eq!(f.selected_name(), "report-3.txt");
        assert!(f.app.prompt.is_some(), "stepping leaves the prompt open");
    }

    #[test]
    fn a_single_match_has_nothing_to_step_to_and_says_nothing() {
        use crossterm::event::KeyModifiers as M;
        let mut f = Fixture::new("search-single");
        touch_files(&f, &["only-this.txt", "zzz.txt"]);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("only");
        press_key(&mut f, KeyCode::Down, M::NONE);
        press_key(&mut f, KeyCode::Up, M::NONE);
        assert_eq!(f.selected_name(), "only-this.txt");
        assert!(
            f.app.status_line().starts_with("/only"),
            "{}",
            f.app.status_line()
        );
    }

    #[test]
    fn stepping_can_cross_folders_and_escape_still_goes_home() {
        use crossterm::event::KeyModifiers as M;
        let mut f = Fixture::new("search-step-dirs");
        std::fs::create_dir_all(f.root.join("a")).unwrap();
        std::fs::create_dir_all(f.root.join("z")).unwrap();
        std::fs::write(f.root.join("a").join("target-a"), b"").unwrap();
        std::fs::write(f.root.join("z").join("target-z"), b"").unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        let started_on = f.selected_name();

        f.app.begin_search(&f.browser);
        f.type_text("target");
        f.wait_for_search();
        assert_eq!(f.browser.current_dir(), f.root.join("a"));
        press_key(&mut f, KeyCode::Down, M::NONE);
        assert_eq!(f.browser.current_dir(), f.root.join("z"));
        assert_eq!(f.selected_name(), "target-z");

        f.press(KeyCode::Esc);
        assert_eq!(f.browser.current_dir(), f.root);
        assert_eq!(f.selected_name(), started_on);
    }

    #[test]
    fn typing_more_after_stepping_searches_again_from_where_it_began() {
        use crossterm::event::KeyModifiers as M;
        let mut f = Fixture::new("search-step-retype");
        touch_files(&f, &["report-1.txt", "report-2.txt", "notes.txt"]);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("report");
        press_key(&mut f, KeyCode::Down, M::NONE);
        assert_eq!(f.selected_name(), "report-2.txt");
        f.type_text("-1");
        assert_eq!(
            f.selected_name(),
            "report-1.txt",
            "the new text starts a new ranking"
        );
        assert_eq!(note(&f), "/report-1", "one match left, so no step counter");
    }

    #[test]
    fn a_name_that_exists_nowhere_leaves_the_cursor_and_says_no_match() {
        let mut f = Fixture::new("search-miss");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("nothing-like-this");
        f.wait_for_search();

        assert_eq!(f.browser.current_dir(), f.root);
        assert!(
            f.app.status_line().starts_with("no match "),
            "{}",
            f.app.status_line()
        );
    }

    #[test]
    fn emptying_the_query_puts_the_browser_back() {
        let mut f = Fixture::new("search-backspace");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("aerend");
        f.wait_for_search();
        assert_ne!(f.browser.current_dir(), f.root);
        for _ in 0.."aerend".len() {
            f.press(KeyCode::Backspace);
        }

        assert!(f.app.search_job.is_none(), "an empty query starts no walk");
        assert_eq!(f.browser.current_dir(), f.root);
        assert_eq!(f.browser.selected_index(), 0);
        assert!(f.app.prompt.is_some(), "the prompt stays open");
    }

    #[test]
    fn a_new_keystroke_replaces_the_walk_before_it() {
        let mut f = Fixture::new("search-replace");
        deep_tree(&f);
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_search(&f.browser);
        f.type_text("aer");
        f.type_text("end");
        f.wait_for_search();

        assert_eq!(f.selected_name(), "aerend.md");
    }

    /// Waits for `record` to hold something, which a detached program writes.
    fn wait_for_record(record: &Path) -> String {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let text = std::fs::read_to_string(record).unwrap_or_default();
            if !text.is_empty() {
                return text;
            }
            assert!(Instant::now() < deadline, "the program never ran");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// A desktop that knows one type, `application/x-zzz` (`*.zzz`), and one program for it: a
    /// script that writes the file it was given to `record`. `extra` goes in its `.desktop`.
    fn install_zzz_opener(f: &mut Fixture, record: &Path, extra: &str) {
        use std::os::unix::fs::PermissionsExt;
        let sys = f.root.join("xdg/sys");
        std::fs::create_dir_all(sys.join("applications")).unwrap();
        std::fs::create_dir_all(sys.join("mime")).unwrap();
        let script = f.root.join("rec.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf %s \"$1\" > {}\n",
                open::shell_quote(&record.to_string_lossy())
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            sys.join("applications/rec.desktop"),
            format!(
                "[Desktop Entry]\nType=Application\nName=Recorder\nExec={} %f\n{extra}\n",
                script.display()
            ),
        )
        .unwrap();
        std::fs::write(
            sys.join("applications/mimeinfo.cache"),
            "[MIME Cache]\napplication/x-zzz=rec.desktop;\n",
        )
        .unwrap();
        std::fs::write(sys.join("mime/globs2"), "50:application/x-zzz:*.zzz\n").unwrap();
        let xdg = crate::mime_type::XdgDirs {
            data_home: f.root.join("xdg/none"),
            data_dirs: vec![sys],
            config_home: f.root.join("xdg/none"),
            ..Default::default()
        };
        assert!(f.app.associations.set(Associations::load(xdg)).is_ok());
    }

    #[test]
    fn opening_a_file_uses_the_program_the_desktop_registered_for_its_type() {
        let mut f = Fixture::new("open-desktop-default");
        let record = f.root.join("record");
        install_zzz_opener(&mut f, &record, "");
        let file = f.root.join("it's a file.zzz");
        std::fs::write(&file, b"").unwrap();

        f.app.open_default(&f.browser, &file);

        assert_eq!(wait_for_record(&record), file.to_string_lossy());
    }

    #[test]
    fn an_open_rule_beats_the_desktops_default() {
        let mut f = Fixture::new("open-rule-wins");
        let (desktop_record, rule_record) = (f.root.join("desktop"), f.root.join("rule"));
        install_zzz_opener(&mut f, &desktop_record, "");
        f.app.open_rules = vec![OpenRule {
            matches: vec!["zzz".into()],
            command: format!(
                "sh -c 'printf %s \"$0\" > \"$1\"' {{}} {}",
                open::shell_quote(&rule_record.to_string_lossy())
            ),
            name: None,
            terminal: false,
        }];
        let file = f.root.join("a.zzz");
        std::fs::write(&file, b"").unwrap();

        f.app.open_default(&f.browser, &file);

        assert_eq!(wait_for_record(&rule_record), file.to_string_lossy());
        assert!(
            !desktop_record.exists(),
            "the desktop's program ran as well"
        );
    }

    #[test]
    fn a_terminal_program_takes_over_the_terminal_whatever_its_name() {
        let mut f = Fixture::new("open-terminal-entry");
        let record = f.root.join("record");
        install_zzz_opener(&mut f, &record, "Terminal=true");
        let file = f.root.join("a.zzz");
        std::fs::write(&file, b"").unwrap();

        f.app.open_default(&f.browser, &file);

        let handover = f
            .app
            .take_handover()
            .expect("Terminal=true asks for the terminal");
        assert!(handover.line.contains("rec.sh"), "{}", handover.line);
        assert!(
            !record.exists(),
            "it must not be started behind the interface"
        );
    }

    #[test]
    fn a_file_the_desktop_knows_nothing_about_falls_back_to_the_system_opener() {
        let mut f = Fixture::new("open-fallback");
        let file = f.root.join("a.unknownkind");
        std::fs::write(&file, [0u8, 1, 2]).unwrap();
        f.app.associations.set(Associations::default()).unwrap();

        f.app.open_default(&f.browser, &file);

        // Nothing registered and nothing ruled: the status names `xdg-open` (or says it is not
        // installed here), rather than opening nothing silently.
        let status = f.app.status.clone().unwrap_or_default();
        assert!(status.contains(open::DEFAULT_OPENER), "{status}");
    }

    #[test]
    fn the_menu_lists_rules_then_configured_then_installed_programs() {
        let mut f = Fixture::new("open-choices");
        let record = f.root.join("record");
        install_zzz_opener(&mut f, &record, "");
        f.app.open_rules = vec![OpenRule {
            matches: vec!["zzz".into()],
            command: "ruled".into(),
            name: Some("Ruled".into()),
            terminal: false,
        }];
        f.app.open_with = vec![OpenWith {
            name: "Configured".into(),
            command: "configured".into(),
            terminal: false,
        }];
        let names: Vec<String> = f
            .app
            .open_choices(&f.root.join("a.zzz"))
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["Ruled", "Configured", "Recorder"]);
        // A `.txt` is not a `.zzz`: the rule stays out of it.
        let plain: Vec<String> = f
            .app
            .open_choices(&f.root.join("a.txt"))
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert!(!plain.contains(&"Ruled".to_string()), "{plain:?}");
    }

    #[test]
    fn other_asks_for_a_command_and_opens_the_file_with_it() {
        let mut f = Fixture::new("open-other");
        let record = f.root.join("record");
        let file = f.root.join("x y.zzz");
        std::fs::write(&file, b"").unwrap();
        f.app.begin_open_other(file.clone());
        assert!(f.app.prompt.as_ref().is_some_and(Prompt::is_text_input));

        let command = format!(
            "sh -c 'printf %s \"$0\" > \"$1\"' {{}} {}",
            open::shell_quote(&record.to_string_lossy())
        );
        for c in command.chars() {
            let _ = f
                .app
                .handle_prompt_key(KeyCode::Char(c), &LocalVfs, &mut f.browser)
                .unwrap();
        }
        let _ = f
            .app
            .handle_prompt_key(KeyCode::Enter, &LocalVfs, &mut f.browser)
            .unwrap();

        assert!(f.app.prompt.is_none());
        assert_eq!(wait_for_record(&record), file.to_string_lossy());
    }

    #[test]
    fn other_can_be_cancelled_or_left_empty_without_running_anything() {
        let mut f = Fixture::new("open-other-cancel");
        let file = f.root.join("a.zzz");
        std::fs::write(&file, b"").unwrap();
        for keys in [vec![KeyCode::Esc], vec![KeyCode::Char(' '), KeyCode::Enter]] {
            f.app.begin_open_other(file.clone());
            for key in keys {
                let _ = f
                    .app
                    .handle_prompt_key(key, &LocalVfs, &mut f.browser)
                    .unwrap();
            }
            assert!(f.app.prompt.is_none());
            assert_eq!(f.app.status.as_deref(), Some("open with cancelled"));
            assert!(f.app.take_handover().is_none());
        }
    }

    #[test]
    fn open_with_an_interactive_program_asks_for_the_terminal_with_the_path_quoted() {
        let mut f = Fixture::new("open-interactive");
        let file = f.root.join("my notes.md");
        std::fs::write(&file, b"").unwrap();

        f.app.open_with(&f.browser, "sh", &file);
        assert_eq!(
            f.app.take_handover(),
            None,
            "sh is not on the interactive list"
        );

        f.app.interactive.push("sh".into());
        f.app.open_with(&f.browser, "sh -e", &file);
        let handover = f.app.take_handover().expect("a handover was requested");
        assert_eq!(handover.line, format!("sh -e '{}'", file.display()));
        assert_eq!(handover.cwd, f.root);
    }

    #[test]
    fn open_with_a_missing_program_says_so_instead_of_failing_silently() {
        let mut f = Fixture::new("open-missing");
        f.app
            .open_with(&f.browser, "minuteman-no-such-program", &f.root);
        assert_eq!(
            f.app.status.as_deref(),
            Some("minuteman-no-such-program: command not found")
        );
        assert_eq!(f.app.take_handover(), None);
    }

    #[test]
    fn a_file_on_a_remote_machine_is_not_opened_by_a_local_program() {
        let mut f = Fixture::new("open-remote");

        f.app
            .open_with(&f.browser, "cat", Path::new("ssh://box/etc/hosts"));

        let status = f.app.status.as_deref().unwrap_or_default();
        assert!(status.contains("remote machine"), "{status}");
        assert_eq!(f.app.take_handover(), None);
    }

    #[test]
    fn a_remote_path_is_never_sent_to_the_local_trash() {
        let mut f = Fixture::new("trash-remote");
        let local = f.root.join("keep.txt");
        std::fs::write(&local, b"x").unwrap();

        f.app
            .spawn_trash(vec![local.clone(), PathBuf::from("ssh://box/etc/hosts")]);

        let status = f.app.status.as_deref().unwrap_or_default();
        assert!(status.contains("no trash"), "{status}");
        assert!(!f.app.is_busy());
        assert!(
            local.exists(),
            "nothing at all is trashed when one target cannot be"
        );
    }

    #[test]
    fn open_with_a_window_program_starts_it_detached_on_the_quoted_path() {
        let mut f = Fixture::new("open-detached");
        let file = f.root.join("it's here.txt");
        std::fs::write(&file, b"").unwrap();
        let record = f.root.join("record");

        // `sh` stands in for a viewer: it writes the one file argument it was given (`$0`) to the
        // path it was given after it (`$1`).
        f.app.open_with(
            &f.browser,
            &format!(
                "sh -c 'printf %s \"$0\" > \"$1\"' {{}} {}",
                open::shell_quote(&record.to_string_lossy())
            ),
            &file,
        );

        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while std::fs::read_to_string(&record)
            .unwrap_or_default()
            .is_empty()
        {
            assert!(Instant::now() < deadline, "the program never ran");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            std::fs::read_to_string(&record).unwrap(),
            file.to_string_lossy()
        );
        assert!(!f.app.is_busy(), "a detached program is never waited on");
    }

    #[test]
    fn the_operation_keys_need_marks_and_ignore_the_cursor() {
        let mut f = Fixture::new("need-marks");
        std::fs::write(f.root.join("a.txt"), b"a").unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        f.browser
            .select_index(f.listed().iter().position(|n| n == "a.txt").unwrap());

        assert!(!f.app.require_marks(&f.browser));
        assert_eq!(
            f.app.status.as_deref(),
            Some("nothing marked — mark with v first")
        );

        f.browser.toggle_mark();
        assert!(f.app.require_marks(&f.browser));
    }

    fn password(text: &str) -> Password {
        let mut password = Password::new();
        text.chars().for_each(|c| password.push(c));
        password
    }

    /// A fixture holding `sealed.zip`, made from a folder that held `inner/a.txt`, with the
    /// password `open sesame`.
    /// A fixture holding `sealed.zip` (an empty stand-in; the plugin is what knows it is
    /// encrypted) and a recording archive plugin.
    fn fixture_with_sealed_zip(name: &str) -> Fixture {
        let mut f = Fixture::new(name).with_archiver();
        std::fs::write(f.root.join("sealed.zip"), b"").unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        select_named(&mut f, "sealed.zip");
        f
    }

    #[test]
    fn a_compress_form_with_a_password_sends_it_with_the_zip_format() {
        let mut f = Fixture::new("seal-compress").with_archiver();
        std::fs::write(f.root.join("a.txt"), b"alpha").unwrap();
        let mut request = request(
            &f,
            &["a.txt"],
            compress_popup::Layout::One {
                file_name: "sealed.zip".into(),
            },
        );
        request.format = Format::Zip;
        request.password = Some(password("open sesame"));

        f.app.start_compress(request).unwrap();

        let sent = f.request(1);
        assert_eq!(sent["work"]["format"], "zip");
        assert_eq!(sent["work"]["password"], "open sesame");
        f.archive_ends(1, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
    }

    #[test]
    fn extracting_an_encrypted_zip_asks_for_the_password_and_then_extracts() {
        let mut f = fixture_with_sealed_zip("seal-extract");

        f.run("extract");
        assert!(f.request(1)["work"].get("password").is_none());
        f.archive_ends(
            1,
            &[],
            ArchiveStatus::PasswordNeeded {
                archive: f.root.join("sealed.zip"),
            },
        );
        assert!(matches!(f.app.prompt, Some(Prompt::ExtractPassword { .. })));
        assert!(
            !f.app.is_busy(),
            "it waits for the user, not for the plugin"
        );
        f.type_text("open sesame");
        f.press(KeyCode::Enter);

        let sent = f.request(2);
        assert_eq!(sent["work"]["password"], "open sesame");
        assert_eq!(
            sent["work"]["jobs"][0]["archive"],
            f.root.join("sealed.zip").to_str().unwrap()
        );
        f.archive_ends(2, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
        assert_eq!(f.app.status.as_deref(), Some("extract complete"));
    }

    #[test]
    fn a_wrong_password_asks_again() {
        let mut f = fixture_with_sealed_zip("seal-wrong");
        let sealed = f.root.join("sealed.zip");

        f.run("extract");
        f.archive_ends(
            1,
            &[],
            ArchiveStatus::PasswordNeeded {
                archive: sealed.clone(),
            },
        );
        f.type_text("guess");
        f.press(KeyCode::Enter);
        f.archive_ends(2, &[], ArchiveStatus::WrongPassword { archive: sealed });

        let Some(Prompt::ExtractPassword { buffer, error, .. }) = &f.app.prompt else {
            panic!("the prompt comes back");
        };
        assert!(buffer.is_empty(), "the wrong attempt is cleared");
        assert!(error.as_deref().unwrap().contains("wrong password"));
        assert!(!f.app.is_busy());
    }

    #[test]
    fn the_password_prompt_shows_dots_and_escape_cancels_the_extract() {
        let mut f = fixture_with_sealed_zip("seal-prompt");

        f.run("extract");
        f.archive_ends(
            1,
            &[],
            ArchiveStatus::PasswordNeeded {
                archive: f.root.join("sealed.zip"),
            },
        );
        f.type_text("abc");
        let shown = f.app.prompt.as_ref().unwrap().display();
        assert!(shown.ends_with("•••") && !shown.contains("abc"), "{shown}");

        f.press(KeyCode::Esc);
        assert!(f.app.prompt.is_none());
        assert_eq!(f.app.status.as_deref(), Some("extract cancelled"));
        assert_eq!(f.recorded("archive").len(), 1, "no second request was sent");
    }

    #[test]
    fn an_empty_password_is_not_submitted() {
        let mut f = fixture_with_sealed_zip("seal-empty");

        f.run("extract");
        f.archive_ends(
            1,
            &[],
            ArchiveStatus::PasswordNeeded {
                archive: f.root.join("sealed.zip"),
            },
        );
        f.press(KeyCode::Enter);

        assert!(matches!(f.app.prompt, Some(Prompt::ExtractPassword { .. })));
        assert_eq!(f.recorded("archive").len(), 1);
    }

    #[test]
    fn extracting_a_plain_archive_never_asks_for_a_password() {
        let mut f = fixture_with_archive("seal-plain");
        f.run("extract");
        assert!(f.app.prompt.is_none());
        f.archive_ends(1, &[0], ArchiveStatus::Completed);
        f.wait_for_idle();
        assert!(f.app.prompt.is_none());
    }

    #[test]
    fn a_mkdir_of_several_names_is_one_undo_step_and_redo_makes_them_again() {
        let mut f = Fixture::new("undo-mkdir");
        f.run("mkdir one two");

        f.app.undo(&mut f.browser, &LocalVfs).unwrap();

        assert!(f.listed().is_empty());
        assert_eq!(f.app.status.as_deref(), Some("undid create 2 items"));
        f.app.redo(&mut f.browser, &LocalVfs).unwrap();
        assert_eq!(f.listed(), vec!["one", "two"]);
    }

    #[test]
    fn a_rename_is_undone_by_the_undo_key() {
        let mut f = Fixture::new("undo-rename");
        std::fs::write(f.root.join("old.txt"), b"x").unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        f.app.begin_rename(&f.browser);
        for _ in 0..7 {
            f.press(KeyCode::Backspace);
        }
        f.type_text("new.txt");
        f.press(KeyCode::Enter);
        assert!(f.root.join("new.txt").exists());

        f.app.undo(&mut f.browser, &LocalVfs).unwrap();

        assert!(f.root.join("old.txt").exists());
        assert!(!f.root.join("new.txt").exists());
    }

    #[test]
    fn a_dropped_batch_of_moves_is_undone_in_one_step() {
        let mut f = Fixture::new("undo-drop");
        for name in ["a.txt", "b.txt"] {
            std::fs::write(f.root.join(name), name).unwrap();
        }
        std::fs::create_dir(f.root.join("dest")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        f.app.begin_drop(
            vec![f.root.join("a.txt"), f.root.join("b.txt")],
            ClipboardMode::Move,
            f.root.join("dest"),
        );
        f.wait_for_idle();
        assert!(f.root.join("dest/a.txt").exists() && f.root.join("dest/b.txt").exists());

        f.app.undo(&mut f.browser, &LocalVfs).unwrap();

        assert!(f.root.join("a.txt").exists() && f.root.join("b.txt").exists());
        assert!(!f.root.join("dest/a.txt").exists());
    }

    #[test]
    fn a_trashed_batch_is_restored_by_one_undo() {
        let mut f = Fixture::new("undo-trash");
        for name in ["a.txt", "b.txt"] {
            std::fs::write(f.root.join(name), name).unwrap();
        }
        f.app
            .spawn_trash(vec![f.root.join("a.txt"), f.root.join("b.txt")]);
        f.wait_for_idle();
        assert!(!f.root.join("a.txt").exists() && !f.root.join("b.txt").exists());

        f.app.undo(&mut f.browser, &LocalVfs).unwrap();

        assert_eq!(std::fs::read(f.root.join("a.txt")).unwrap(), b"a.txt");
        assert_eq!(std::fs::read(f.root.join("b.txt")).unwrap(), b"b.txt");
    }

    #[test]
    fn a_permanent_delete_is_not_recorded_so_undo_says_there_is_nothing() {
        let mut f = Fixture::new("undo-delete");
        std::fs::write(f.root.join("x.txt"), b"x").unwrap();
        f.app.spawn_delete(vec![f.root.join("x.txt")]);
        f.wait_for_idle();

        f.app.undo(&mut f.browser, &LocalVfs).unwrap();

        assert_eq!(f.app.status.as_deref(), Some("nothing to undo"));
    }

    #[test]
    fn paste_into_a_folder_puts_the_copy_there_and_not_beside_the_source() {
        let mut f = Fixture::new("paste-into");
        std::fs::write(f.root.join("a.txt"), b"a").unwrap();
        std::fs::create_dir(f.root.join("dest")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        select_named(&mut f, "a.txt");
        f.app.yank(&mut f.browser);

        f.app.begin_paste_into(f.root.join("dest"));
        f.wait_for_idle();

        assert!(f.root.join("dest").join("a.txt").exists());
        assert_eq!(f.listed().iter().filter(|n| *n == "a.txt").count(), 1);
    }

    #[test]
    fn a_dropped_move_lands_in_the_folder_and_leaves_the_users_clipboard_alone() {
        let mut f = Fixture::new("drop-move");
        std::fs::write(f.root.join("a.txt"), b"a").unwrap();
        std::fs::write(f.root.join("keep.txt"), b"k").unwrap();
        std::fs::create_dir(f.root.join("dest")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        select_named(&mut f, "keep.txt");
        f.app.yank(&mut f.browser);

        f.app.begin_drop(
            vec![f.root.join("a.txt")],
            ClipboardMode::Move,
            f.root.join("dest"),
        );
        f.wait_for_idle();

        assert!(f.root.join("dest").join("a.txt").exists());
        assert!(!f.root.join("a.txt").exists());
        let held = f
            .app
            .clipboard
            .as_ref()
            .expect("the yank survived the drop");
        assert_eq!(held.paths, [f.root.join("keep.txt")]);
    }

    #[test]
    fn a_dropped_copy_keeps_the_original_and_a_moved_mark_is_forgotten() {
        let mut f = Fixture::new("drop-copy");
        std::fs::write(f.root.join("a.txt"), b"a").unwrap();
        std::fs::write(f.root.join("b.txt"), b"b").unwrap();
        std::fs::create_dir(f.root.join("dest")).unwrap();
        f.browser.reload(&LocalVfs).unwrap();
        f.browser
            .select_index(f.listed().iter().position(|n| n == "b.txt").unwrap());
        f.browser.toggle_mark();

        f.app.begin_drop(
            vec![f.root.join("a.txt")],
            ClipboardMode::Copy,
            f.root.join("dest"),
        );
        f.wait_for_idle();
        assert!(f.root.join("a.txt").exists() && f.root.join("dest/a.txt").exists());

        f.app.begin_drop(
            vec![f.root.join("b.txt")],
            ClipboardMode::Move,
            f.root.join("dest"),
        );
        f.wait_for_idle();
        assert!(!f.root.join("b.txt").exists());
        assert!(
            f.browser.marked_paths().is_empty(),
            "the mark on a moved file outlived it"
        );
    }

    #[test]
    fn a_drop_onto_an_existing_name_asks_before_overwriting() {
        let mut f = Fixture::new("drop-conflict");
        std::fs::write(f.root.join("a.txt"), b"new").unwrap();
        std::fs::create_dir(f.root.join("dest")).unwrap();
        std::fs::write(f.root.join("dest/a.txt"), b"old").unwrap();
        f.browser.reload(&LocalVfs).unwrap();

        f.app.begin_drop(
            vec![f.root.join("a.txt")],
            ClipboardMode::Move,
            f.root.join("dest"),
        );
        f.wait_for_idle();

        assert!(f.app.prompt.is_some(), "a conflict must raise the prompt");
        assert_eq!(std::fs::read(f.root.join("dest/a.txt")).unwrap(), b"old");
        assert!(f.root.join("a.txt").exists());
    }

    #[test]
    fn caps_lock_starts_off_and_tracks_the_last_key_event() {
        let mut f = Fixture::new("caps-lock");
        assert!(!f.app.caps_lock());

        f.app.set_caps_lock(true);
        assert!(f.app.caps_lock());

        f.app.set_caps_lock(false);
        assert!(!f.app.caps_lock());
    }

    // ---- an app browsing a backend that is not the local disk ----

    /// An app and browser over an in-memory filesystem holding `/w`.
    struct MemoryApp {
        _runtime: tokio::runtime::Runtime,
        mem: Arc<shared::MemVfs>,
        app: App,
        browser: BrowserState,
    }

    impl MemoryApp {
        fn new(build: impl FnOnce(&shared::MemVfs)) -> Self {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let mem = Arc::new(shared::MemVfs::new());
            mem.add_dir("/w");
            build(&mem);
            let app = App::new(runtime.handle().clone()).with_vfs(mem.clone());
            let browser = BrowserState::new(mem.as_ref(), PathBuf::from("/w")).unwrap();
            Self {
                _runtime: runtime,
                mem,
                app,
                browser,
            }
        }

        fn wait_for_idle(&mut self) {
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            while self.app.is_busy() && Instant::now() < deadline {
                self.app
                    .poll_bulk(&mut self.browser, self.mem.as_ref())
                    .unwrap();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(!self.app.is_busy(), "the job did not finish in time");
        }
    }

    #[test]
    fn a_permanent_delete_runs_against_the_injected_backend() {
        let mut m = MemoryApp::new(|mem| {
            mem.add_file("/w/a.txt", "a");
            mem.add_file("/w/tree/inner/b.txt", "b");
        });

        m.app
            .spawn_delete(vec![PathBuf::from("/w/a.txt"), PathBuf::from("/w/tree")]);
        m.wait_for_idle();

        assert!(!m.mem.exists(Path::new("/w/a.txt")));
        assert!(!m.mem.exists(Path::new("/w/tree")));
        assert_eq!(m.app.status.as_deref(), Some("delete complete"));
    }

    #[test]
    fn a_copy_and_a_move_run_against_the_injected_backend() {
        let mut m = MemoryApp::new(|mem| {
            mem.add_file("/w/keep.txt", "keep");
            mem.add_file("/w/move.txt", "move");
            mem.add_dir("/w/dest");
        });

        m.app.begin_drop(
            vec![PathBuf::from("/w/keep.txt")],
            ClipboardMode::Copy,
            PathBuf::from("/w/dest"),
        );
        m.wait_for_idle();
        m.app.begin_drop(
            vec![PathBuf::from("/w/move.txt")],
            ClipboardMode::Move,
            PathBuf::from("/w/dest"),
        );
        m.wait_for_idle();

        assert!(
            m.mem.exists(Path::new("/w/keep.txt")),
            "a copy leaves the original"
        );
        assert!(m.mem.exists(Path::new("/w/dest/keep.txt")));
        assert!(!m.mem.exists(Path::new("/w/move.txt")));
        assert!(m.mem.exists(Path::new("/w/dest/move.txt")));
    }

    #[test]
    fn undo_reverses_a_move_on_the_injected_backend() {
        let mut m = MemoryApp::new(|mem| {
            mem.add_file("/w/a.txt", "a");
            mem.add_dir("/w/dest");
        });
        m.app.begin_drop(
            vec![PathBuf::from("/w/a.txt")],
            ClipboardMode::Move,
            PathBuf::from("/w/dest"),
        );
        m.wait_for_idle();

        m.app.undo(&mut m.browser, m.mem.as_ref()).unwrap();

        assert!(m.mem.exists(Path::new("/w/a.txt")));
        assert!(!m.mem.exists(Path::new("/w/dest/a.txt")));
    }

    #[test]
    fn the_compress_form_checks_the_injected_backend_for_a_taken_name_and_a_missing_folder() {
        let m = MemoryApp::new(|mem| {
            mem.add_file("/w/a.txt", "a");
            mem.add_file("/w/a.zip", "already");
        });
        let mut request = compress_popup::Request {
            sources: vec![PathBuf::from("/w/a.txt")],
            dir: PathBuf::from("/w"),
            format: Format::Zip,
            level: Level::Normal,
            password: None,
            layout: compress_popup::Layout::One {
                file_name: "a.zip".into(),
            },
            delete_originals: false,
        };

        assert_eq!(
            App::compress_problem(m.mem.as_ref(), &request),
            Some("a.zip already exists".into())
        );
        request.dir = PathBuf::from("/nowhere");
        assert_eq!(
            App::compress_problem(m.mem.as_ref(), &request),
            Some("/nowhere is not a folder".into())
        );
    }

    #[test]
    fn inspect_without_a_plugin_says_how_to_get_it_and_disk_usage_opens_on_the_injected_backend() {
        let mut m = MemoryApp::new(|mem| mem.add_file("/w/f.txt", vec![0u8; 100]));

        assert!(m.app.begin_inspect(Path::new("/w/f.txt")).is_none());
        assert!(
            m.app
                .status
                .as_deref()
                .is_some_and(|s| s.contains("cargo install --path plugins/inspect")),
            "{:?}",
            m.app.status
        );
        let mut usage = m.app.begin_disk_usage(PathBuf::from("/w"));
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while usage.end().is_none() && Instant::now() < deadline {
            usage.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(usage.rows()[0].name, "f.txt");
    }

    #[test]
    fn find_with_no_plugin_to_answer_it_says_how_to_get_the_finder() {
        let mut fx = Fixture::new("find-missing");
        fx.app.begin_find(&fx.browser);
        let status = fx.app.status.clone().expect("it says something");
        assert!(status.contains("mman-finder"), "{status}");
        assert!(!fx.app.awaiting_picker(), "nothing is on its way");
    }

    /// A plugin that provides `find` and then never says anything, standing in for a slow one.
    fn silent_finder() -> PluginSpec {
        PluginSpec {
            name: "silent".to_string(),
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "cat > /dev/null".to_string()],
            on_keys: Vec::new(),
            commands: Vec::new(),
            events: Vec::new(),
            provides: vec!["find".to_string()],
        }
    }

    #[test]
    fn find_hands_the_action_to_its_plugin_once_and_waits_for_the_picker() {
        let mut fx = Fixture::new("find-provided");
        fx.app = std::mem::replace(&mut fx.app, App::new(fx._runtime.handle().clone()))
            .with_plugins(vec![silent_finder()], &fx.root);
        fx.app.begin_find(&fx.browser);
        assert_eq!(fx.app.status.as_deref(), Some("opening the finder…"));
        assert!(fx.app.awaiting_picker());

        fx.app.status = None;
        fx.app.begin_find(&fx.browser);
        assert_eq!(
            fx.app.status, None,
            "a second press while one is on its way does nothing"
        );

        let spec = PickerSpec {
            picker: 1,
            title: "find".to_string(),
            preview: true,
        };
        let _popup = fx.app.begin_picker("silent", &spec);
        assert!(!fx.app.awaiting_picker(), "it opened");
    }
}
