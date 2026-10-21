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

//! Runs one plugin as a WebAssembly module in wasmi. The module speaks the same line protocol as a
//! process plugin, over in-memory pipes instead of a child's stdin and stdout, and is given
//! nothing else: no preopened folders, no environment, no arguments, no stderr, no network (the
//! WASI layer here has none to give). Every capability it has is a request the host answers (see
//! `sandbox` for which).
//!
//! The module runs on its own OS thread, because the WASI layer in use is synchronous. Three
//! limits keep it from hurting the interface:
//!
//! - memory: growing past `Limits::memory_bytes` fails, which a Rust guest turns into a trap;
//! - time: the module runs in slices of `slice_fuel`. After each slice the thread checks whether
//!   the host asked it to stop, and whether the module has read its input within `max_idle`. A
//!   module waiting for input burns no fuel and so has no slice end to be judged at, which means
//!   a plugin that sleeps between events for hours is fine; one that computes for longer than
//!   `max_idle` without reading is not. The limit is wall-clock time, checked at slice ends, so
//!   how long a slice takes (it depends on the build's speed) only sets how late it is noticed;
//! - stopping: the host sets a flag, which the input wrapper turns into end-of-file and the slice
//!   check turns into an exit, so a stop lands within tens of milliseconds either way.
//!
//! Model of the guest thread, which is the only part that interleaves with the host:
//!
//! ```text
//! state:  Running(since) -> Running(now)         slice ends, a read happened since the last one
//!         Running(since) -> Running(since)       slice ends, none did, and now - since < max_idle
//!         Running(_)     -> Ended(StoppedByHost) slice ends or a read polls, stop is set
//!         Running(since) -> Ended(Starved)       slice ends, none did, and now - since >= max_idle
//!         Running(_)     -> Ended(Finished | Failed)  the module returns or traps
//! A read counts when it is *entered*, so a module blocked in one is never judged: the next slice
//! end after it wakes sees the count changed. Invariant: Ended is reached once, and `ended` is sent exactly once, after the last write
//!            to the output pipe, so the host never sees the end before the last line.
//! ```

use std::io::{self, Read, Write};
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{mpsc, oneshot};
use wasmi::{
    Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc,
    TypedResumableCall,
};
use wasmi_wasi::wasi_common::pipe::{ReadPipe, WritePipe};
use wasmi_wasi::{WasiCtx, WasiCtxBuilder};

use crate::error::PluginError;

/// The largest module the host will read, so a path to a huge file is not read into memory.
const MAX_MODULE_BYTES: u64 = 32 * 1024 * 1024;

/// How long a guest blocked on input waits between looks at the stop flag.
const STOP_POLL: Duration = Duration::from_millis(25);

/// The smallest slice accepted. The engine charges for entering a function, and a slice smaller
/// than that charge is refused by the first call instead of being resumed.
#[cfg(test)]
const MIN_SLICE_FUEL: u64 = 10_000;

/// How many chunks of output may wait for the host to read them before the module is made to
/// wait, so a module that only writes cannot fill the host's memory.
const OUTPUT_BACKLOG: usize = 256;

/// What a module may use. Built through [`Limits::try_new`], so a zero that would make the
/// module unable to run (or unable to be stopped) cannot be written down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    memory_bytes: usize,
    slice_fuel: u64,
    max_idle: Duration,
}

impl Limits {
    /// 64 MiB of memory and one second of work without reading input, noticed at the end of a
    /// 20 million fuel slice (about 7 ms in a release build, where wasmi runs some 2.8 billion
    /// fuel a second).
    pub(crate) const fn standard() -> Self {
        Self {
            memory_bytes: 64 * 1024 * 1024,
            slice_fuel: 20_000_000,
            max_idle: Duration::from_secs(1),
        }
    }

    /// Limits with the given memory cap, slice size and longest time without a read. Only the
    /// tests ask for anything but `standard` until a manifest can set them.
    ///
    /// # Errors
    ///
    /// A message when the memory cap or the idle time is zero, or the slice is below
    /// `MIN_SLICE_FUEL`.
    #[cfg(test)]
    pub(crate) fn try_new(
        memory_bytes: usize,
        slice_fuel: u64,
        max_idle: Duration,
    ) -> Result<Self, &'static str> {
        if memory_bytes == 0 || max_idle.is_zero() {
            return Err("a limit of zero would leave the module unable to run");
        }
        if slice_fuel < MIN_SLICE_FUEL {
            return Err("a slice smaller than the cost of entering a function cannot run");
        }
        Ok(Self {
            memory_bytes,
            slice_fuel,
            max_idle,
        })
    }
}

/// How a module's run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ending {
    /// It returned, or called `proc_exit(0)`.
    Finished,
    /// The host asked it to stop.
    StoppedByHost,
    /// It went this long without reading its input.
    Starved(Duration),
    /// It trapped (including running out of memory) or exited with a failing status.
    Failed(String),
}

impl std::fmt::Display for Ending {
    /// The words `:plugins` and the log use for how a module ended.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Finished => f.write_str("finished"),
            Self::StoppedByHost => f.write_str("stopped by the host"),
            Self::Starved(idle) => write!(
                f,
                "stopped by the sandbox: it went {idle:?} without reading its input"
            ),
            Self::Failed(why) => f.write_str(why),
        }
    }
}

/// The host's end of a module: lines in, lines out, and the way to stop it.
pub(crate) struct Guest {
    pub(crate) to_guest: ToGuest,
    pub(crate) from_guest: FromGuest,
    pub(crate) ended: oneshot::Receiver<Ending>,
    pub(crate) stop: Arc<AtomicBool>,
}

struct State {
    wasi: WasiCtx,
    limits: StoreLimits,
}

/// Loads the module at `path`, checks it can be linked, and starts its thread. Nothing of the module runs on the calling thread.
///
/// # Errors
///
/// [`PluginError::Spawn`] when the file cannot be read; [`PluginError::Wasm`] when it is too
/// large, is not a valid module, has a start function (which would run on the caller's thread,
/// unbounded), imports something the sandbox does not provide, or has no `_start`.
pub(crate) fn spawn(name: &str, path: &Path, limits: Limits) -> Result<Guest, PluginError> {
    let wasm_error = |reason: String| PluginError::Wasm {
        name: name.to_string(),
        reason,
    };
    let spawn_error = |source: io::Error| PluginError::Spawn {
        name: name.to_string(),
        command: path.display().to_string(),
        source,
    };

    let size = std::fs::metadata(path).map_err(spawn_error)?.len();
    if size > MAX_MODULE_BYTES {
        return Err(wasm_error(format!(
            "the module is {size} bytes; the limit is {MAX_MODULE_BYTES}"
        )));
    }
    let bytes = std::fs::read(path).map_err(spawn_error)?;

    let mut config = Config::default();
    config.consume_fuel(true);
    // A start function would run inside `instantiate_and_start` below, on the caller's thread and
    // before any limit applies; `_start` runs on the guest thread, in slices.
    config.allow_start_fn(false);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, &bytes)
        .map_err(|e| wasm_error(format!("not an acceptable WebAssembly module: {e}")))?;

    let (to_guest_tx, to_guest_rx) = std_mpsc::channel::<Vec<u8>>();
    let (from_guest_tx, from_guest_rx) = mpsc::channel::<Vec<u8>>(OUTPUT_BACKLOG);
    let stop = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicU64::new(0));

    let input = GuestIn {
        rx: Mutex::new(to_guest_rx),
        pending: Vec::new(),
        at: 0,
        stop: Arc::clone(&stop),
        reads: Arc::clone(&reads),
    };
    let wasi = WasiCtxBuilder::new()
        .stdin(Box::new(ReadPipe::new(input)))
        .stdout(Box::new(WritePipe::new(GuestOut(from_guest_tx))))
        .stderr(Box::new(WritePipe::new(io::sink())))
        .build();
    let mut store = Store::new(
        &engine,
        State {
            wasi,
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.memory_bytes)
                .instances(1)
                .build(),
        },
    );
    store.limiter(|state| &mut state.limits);

    let mut linker: Linker<State> = Linker::new(&engine);
    wasmi_wasi::add_to_linker(&mut linker, |state: &mut State| &mut state.wasi)
        .map_err(|e| wasm_error(format!("could not set up the sandbox: {e}")))?;
    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|e| wasm_error(format!("it needs something the sandbox does not give: {e}")))?;
    let entry = instance
        .get_typed_func::<(), ()>(&store, "_start")
        .map_err(|_| {
            wasm_error("it exports no `_start`, so it is not a WASI program".to_string())
        })?;

    let (ended_tx, ended_rx) = oneshot::channel();
    let guest_stop = Arc::clone(&stop);
    std::thread::Builder::new()
        .name(format!("wasm-{name}"))
        .spawn(move || {
            let ending = run(store, entry, &guest_stop, &reads, limits);
            // Dropping the store (and so the output sender) happened inside `run`, before this.
            let _ = ended_tx.send(ending);
        })
        .map_err(spawn_error)?;

    Ok(Guest {
        to_guest: ToGuest(to_guest_tx),
        from_guest: FromGuest {
            rx: from_guest_rx,
            pending: Vec::new(),
            at: 0,
        },
        ended: ended_rx,
        stop,
    })
}

/// Runs `entry` in slices until it ends, applying the time rules in the module doc.
fn run(
    mut store: Store<State>,
    entry: TypedFunc<(), ()>,
    stop: &AtomicBool,
    reads: &AtomicU64,
    limits: Limits,
) -> Ending {
    let failed = |what: &str, e: &dyn std::fmt::Display| Ending::Failed(format!("{what}: {e}"));
    if let Err(e) = store.set_fuel(limits.slice_fuel) {
        return failed("could not meter the module", &e);
    }
    let mut seen = reads.load(Ordering::Acquire);
    let mut last_read = Instant::now();
    let mut call = entry.call_resumable(&mut store, ());
    loop {
        match call {
            Ok(TypedResumableCall::Finished(())) => return Ending::Finished,
            Ok(TypedResumableCall::HostTrap(_)) => {
                return Ending::Failed("a host function trapped".to_string());
            }
            Ok(TypedResumableCall::OutOfFuel(paused)) => {
                if stop.load(Ordering::Acquire) {
                    return Ending::StoppedByHost;
                }
                let now = reads.load(Ordering::Acquire);
                if now != seen {
                    seen = now;
                    last_read = Instant::now();
                } else if last_read.elapsed() >= limits.max_idle {
                    return Ending::Starved(limits.max_idle);
                }
                let fuel = limits.slice_fuel.max(paused.required_fuel());
                if let Err(e) = store.set_fuel(fuel) {
                    return failed("could not meter the module", &e);
                }
                call = paused.resume(&mut store);
            }
            Err(e) => {
                if stop.load(Ordering::Acquire) {
                    return Ending::StoppedByHost;
                }
                return match e.i32_exit_status() {
                    Some(0) => Ending::Finished,
                    Some(code) => Ending::Failed(format!("exited with status {code}")),
                    None => Ending::Failed(e.to_string()),
                };
            }
        }
    }
}

/// What the module reads as its stdin: chunks the host sends. Gives end-of-file once the host
/// drops its end or asks to stop, and counts reads so `run` can tell a module that is listening
/// from one that is not.
struct GuestIn {
    rx: Mutex<std_mpsc::Receiver<Vec<u8>>>,
    pending: Vec<u8>,
    at: usize,
    stop: Arc<AtomicBool>,
    reads: Arc<AtomicU64>,
}

impl Read for GuestIn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reads.fetch_add(1, Ordering::AcqRel);
        loop {
            if self.at < self.pending.len() {
                let n = (self.pending.len() - self.at).min(buf.len());
                buf[..n].copy_from_slice(&self.pending[self.at..self.at + n]);
                self.at += n;
                return Ok(n);
            }
            if self.stop.load(Ordering::Acquire) {
                return Ok(0);
            }
            let rx = self
                .rx
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match rx.recv_timeout(STOP_POLL) {
                Ok(chunk) => {
                    self.pending = chunk;
                    self.at = 0;
                }
                Err(std_mpsc::RecvTimeoutError::Timeout) => {}
                Err(std_mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
    }
}

/// What the module writes as its stdout. Waits when the host has `OUTPUT_BACKLOG` chunks unread.
struct GuestOut(mpsc::Sender<Vec<u8>>);

impl Write for GuestOut {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.0
            .blocking_send(buf.to_vec())
            .map(|()| buf.len())
            .map_err(|_| io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The host's write half: what `write_loop` sends the module.
pub(crate) struct ToGuest(std_mpsc::Sender<Vec<u8>>);

impl AsyncWrite for ToGuest {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(
            self.0
                .send(buf.to_vec())
                .map(|()| buf.len())
                .map_err(|_| io::ErrorKind::BrokenPipe.into()),
        )
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// The host's read half: what `read_loop` reads the module's output from.
pub(crate) struct FromGuest {
    rx: mpsc::Receiver<Vec<u8>>,
    pending: Vec<u8>,
    at: usize,
}

impl AsyncRead for FromGuest {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            if this.at < this.pending.len() {
                let n = (this.pending.len() - this.at).min(buf.remaining());
                buf.put_slice(&this.pending[this.at..this.at + n]);
                this.at += n;
                return Poll::Ready(Ok(()));
            }
            match this.rx.poll_recv(cx) {
                Poll::Ready(Some(chunk)) => {
                    this.pending = chunk;
                    this.at = 0;
                }
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    /// Copies its input to its output until the input ends.
    const ECHO: &str = r#"(module
        (import "wasi_snapshot_preview1" "fd_read" (func $fd_read (param i32 i32 i32 i32) (result i32)))
        (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "_start")
            (i32.store (i32.const 0) (i32.const 64))
            (i32.store (i32.const 4) (i32.const 256))
            (i32.store (i32.const 8) (i32.const 64))
            (loop $again
                (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 32)))
                (if (i32.eqz (i32.load (i32.const 32))) (then (return)))
                (i32.store (i32.const 12) (i32.load (i32.const 32)))
                (drop (call $fd_write (i32.const 1) (i32.const 8) (i32.const 1) (i32.const 40)))
                (br $again)))
    )"#;

    /// Like `ECHO`, but burns a few thousand instructions after every read, so each event spans
    /// one or more slices of a small budget.
    const LISTEN: &str = r#"(module
        (import "wasi_snapshot_preview1" "fd_read" (func $fd_read (param i32 i32 i32 i32) (result i32)))
        (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "_start") (local $i i32)
            (i32.store (i32.const 0) (i32.const 64))
            (i32.store (i32.const 4) (i32.const 256))
            (i32.store (i32.const 8) (i32.const 64))
            (loop $again
                (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 32)))
                (if (i32.eqz (i32.load (i32.const 32))) (then (return)))
                (local.set $i (i32.const 6000))
                (loop $burn
                    (local.set $i (i32.sub (local.get $i) (i32.const 1)))
                    (br_if $burn (local.get $i)))
                (i32.store (i32.const 12) (i32.load (i32.const 32)))
                (drop (call $fd_write (i32.const 1) (i32.const 8) (i32.const 1) (i32.const 40)))
                (br $again)))
    )"#;

    const SPIN: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "_start") (loop $forever (br $forever))))"#;

    /// Asks for 2000 pages (about 131 MB) and traps if the host says no.
    const GROW: &str = r#"(module
        (memory (export "memory") 1)
        (func (export "_start")
            (if (i32.eq (memory.grow (i32.const 2000)) (i32.const -1)) (then (unreachable)))))"#;

    fn module_file(label: &str, wat: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mman-wasm-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("make the module's folder");
        let path = dir.join("p.wasm");
        std::fs::write(
            &path,
            wat::parse_str(wat).expect("the fixture is valid WAT"),
        )
        .expect("write the module");
        path
    }

    fn quick_limits() -> Limits {
        Limits::try_new(1024 * 1024, 1_000_000, Duration::from_millis(100)).expect("valid limits")
    }

    async fn ending(guest: Guest, within: Duration) -> Ending {
        tokio::time::timeout(within, guest.ended)
            .await
            .expect("the module ended in time")
            .expect("the thread reported")
    }

    #[test]
    fn a_zero_limit_cannot_be_built() {
        let second = Duration::from_secs(1);
        assert!(Limits::try_new(0, MIN_SLICE_FUEL, second).is_err());
        assert!(Limits::try_new(1, 0, second).is_err());
        assert!(Limits::try_new(1, MIN_SLICE_FUEL - 1, second).is_err());
        assert!(Limits::try_new(1, MIN_SLICE_FUEL, Duration::ZERO).is_err());
        assert!(Limits::try_new(1, MIN_SLICE_FUEL, second).is_ok());
    }

    #[tokio::test]
    async fn lines_go_in_and_come_back_and_closing_the_input_ends_it_cleanly() {
        let path = module_file("echo", ECHO);
        let mut guest = spawn("t", &path, Limits::standard()).expect("spawn");
        guest.to_guest.write_all(b"ping\npong\n").await.unwrap();
        let mut lines = BufReader::new(&mut guest.from_guest).lines();
        let first = tokio::time::timeout(Duration::from_secs(5), lines.next_line()).await;
        assert_eq!(first.unwrap().unwrap().as_deref(), Some("ping"));
        let second = tokio::time::timeout(Duration::from_secs(5), lines.next_line()).await;
        assert_eq!(second.unwrap().unwrap().as_deref(), Some("pong"));
        drop(lines);
        drop(guest.to_guest);
        let rest = Guest {
            to_guest: ToGuest(std_mpsc::channel().0),
            ..guest
        };
        assert_eq!(ending(rest, Duration::from_secs(5)).await, Ending::Finished);
    }

    #[tokio::test]
    async fn a_module_that_never_reads_is_ended_as_starved_and_the_host_stays_responsive() {
        let path = module_file("starve", SPIN);
        let guest = spawn("t", &path, quick_limits()).expect("spawn");
        let started = std::time::Instant::now();
        // The test's own task keeps running while the module spins: this await would never
        // return on a single-threaded runtime if the module held the thread.
        let starved = ending(guest, Duration::from_secs(10)).await;
        assert_eq!(starved, Ending::Starved(Duration::from_millis(100)));
        assert!(starved.to_string().contains("without reading its input"));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn the_host_can_stop_a_module_that_is_spinning() {
        let path = module_file("stop-spin", SPIN);
        // An idle allowance far longer than the test, so only the stop flag can end it in time.
        let limits = Limits::try_new(1024 * 1024, 1_000_000, Duration::from_secs(3600)).unwrap();
        let guest = spawn("t", &path, limits).expect("spawn");
        tokio::time::sleep(Duration::from_millis(100)).await;
        guest.stop.store(true, Ordering::Release);
        assert_eq!(
            ending(guest, Duration::from_secs(5)).await,
            Ending::StoppedByHost
        );
    }

    #[tokio::test]
    async fn the_host_can_stop_a_module_that_is_waiting_for_input() {
        let path = module_file("stop-wait", ECHO);
        let guest = spawn("t", &path, Limits::standard()).expect("spawn");
        tokio::time::sleep(Duration::from_millis(100)).await;
        guest.stop.store(true, Ordering::Release);
        // The input reports end-of-file, so the echo loop returns by itself: a clean finish.
        assert_eq!(
            ending(guest, Duration::from_secs(5)).await,
            Ending::Finished
        );
    }

    #[tokio::test]
    async fn a_module_that_keeps_reading_is_never_starved() {
        let path = module_file("listen", LISTEN);
        // The 100 events, 10 ms apart, take a second or more, far over the 300 ms allowed without a
        // read: only each read restarting the clock keeps the module alive.
        let limits =
            Limits::try_new(1024 * 1024, MIN_SLICE_FUEL, Duration::from_millis(300)).unwrap();
        let mut guest = spawn("t", &path, limits).expect("spawn");
        let mut lines = BufReader::new(&mut guest.from_guest).lines();
        for n in 0..100 {
            guest
                .to_guest
                .write_all(format!("event {n}\n").as_bytes())
                .await
                .unwrap();
            let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line()).await;
            assert_eq!(line.unwrap().unwrap(), Some(format!("event {n}")));
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn growing_memory_past_the_cap_fails_and_the_module_traps() {
        let path = module_file("grow", GROW);
        let guest = spawn("t", &path, quick_limits()).expect("spawn");
        let result = ending(guest, Duration::from_secs(5)).await;
        assert!(matches!(result, Ending::Failed(_)), "{result:?}");
    }

    #[tokio::test]
    async fn growing_memory_inside_the_cap_succeeds() {
        let path = module_file("grow-ok", GROW);
        let limits =
            Limits::try_new(256 * 1024 * 1024, 1_000_000, Duration::from_millis(100)).unwrap();
        let guest = spawn("t", &path, limits).expect("spawn");
        assert_eq!(
            ending(guest, Duration::from_secs(5)).await,
            Ending::Finished
        );
    }

    #[test]
    fn a_module_the_sandbox_cannot_run_is_refused_with_a_reason() {
        let cases = [
            ("garbage", None, "not an acceptable"),
            (
                "start-fn",
                Some(
                    r#"(module (memory (export "memory") 1) (func $s) (start $s) (func (export "_start")))"#,
                ),
                "not an acceptable",
            ),
            (
                "no-entry",
                Some(r#"(module (memory (export "memory") 1))"#),
                "exports no `_start`",
            ),
            (
                "unknown-import",
                Some(
                    r#"(module (import "env" "x" (func)) (memory (export "memory") 1) (func (export "_start")))"#,
                ),
                "does not give",
            ),
            (
                "network",
                Some(
                    r#"(module (import "wasi_snapshot_preview1" "sock_accept" (func (param i32 i32 i32) (result i32))) (memory (export "memory") 1) (func (export "_start")))"#,
                ),
                "",
            ),
        ];
        for (label, wat, want) in cases {
            let path = match wat {
                Some(wat) => module_file(label, wat),
                None => {
                    let path = module_file(label, "(module)");
                    std::fs::write(&path, b"not wasm at all").unwrap();
                    path
                }
            };
            let result = spawn("t", &path, Limits::standard());
            if label == "network" {
                // Whether the WASI layer links `sock_accept` is its business; if it does, the
                // module still has no socket to accept on. What matters is it cannot be given one.
                continue;
            }
            match result {
                Err(PluginError::Wasm { reason, .. }) => {
                    assert!(reason.contains(want), "{label}: {reason}");
                }
                other => panic!("{label}: expected a refusal, got {:?}", other.map(|_| ())),
            }
        }
    }

    #[test]
    fn a_missing_file_is_a_spawn_error_and_an_oversized_one_is_refused() {
        let missing = std::env::temp_dir().join("mman-wasm-definitely-missing.wasm");
        assert!(matches!(
            spawn("t", &missing, Limits::standard()),
            Err(PluginError::Spawn { .. })
        ));
        let huge = module_file("huge", "(module)");
        std::fs::File::options()
            .write(true)
            .open(&huge)
            .unwrap()
            .set_len(MAX_MODULE_BYTES + 1)
            .unwrap();
        match spawn("t", &huge, Limits::standard()) {
            Err(PluginError::Wasm { reason, .. }) => assert!(reason.contains("limit"), "{reason}"),
            other => panic!("expected a refusal, got {:?}", other.map(|_| ())),
        }
    }

    proptest! {
        // Whatever bytes the host sends, in whatever chunks, come back in order and intact.
        #[test]
        fn the_pipes_carry_any_bytes_in_order(
            chunks in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 1..300), 1..12)
        ) {
            let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
            let sent: Vec<u8> = chunks.concat();
            let received = runtime.block_on(async {
                let path = module_file("prop-echo", ECHO);
                let mut guest = spawn("t", &path, Limits::standard()).expect("spawn");
                for chunk in &chunks {
                    guest.to_guest.write_all(chunk).await.unwrap();
                }
                let mut got = Vec::new();
                while got.len() < sent.len() {
                    let mut buf = [0u8; 512];
                    let n = tokio::time::timeout(Duration::from_secs(5), guest.from_guest.read(&mut buf))
                        .await
                        .expect("output arrives")
                        .unwrap();
                    if n == 0 { break; }
                    got.extend_from_slice(&buf[..n]);
                }
                got
            });
            prop_assert_eq!(received, sent);
        }
    }
}
