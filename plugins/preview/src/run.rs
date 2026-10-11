//! Drives the preview from the lines on stdin. A reader thread turns each line into a [`Message`];
//! the loop below owns the output, so every line to the host goes through one writer. A `preview`
//! event starts a worker thread that loads the file (asking the host for its bytes with
//! `read_file` and waiting for each answer), and the loop sends the document on only if no newer
//! event arrived while it was working. The loop never waits on a file, so a slow disk or a far
//! host never keeps the plugin from noticing the cursor moved.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use plugin_api::client::{Incoming, notification_line, parse_incoming, request_line};
use plugin_api::{
    API_VERSION, EVENT_INIT, EVENT_PREVIEW, EventParams, METHOD_SET_PREVIEW, PreviewContent,
    SetPreview,
};
use serde_json::{Value, json};

use crate::hook::{self, Hook, Kind};
use crate::load;
use crate::remote::{Remote, RemoteVfs};

/// How long a worker waits for the host to answer one request before giving up on the file. The
/// host answers from a thread of its own, so this only matters when the disk or the network has
/// stalled.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// What the plugin was started with.
#[derive(Debug, Default)]
pub struct Config {
    pub hooks: Vec<Hook>,
    /// Command-line arguments that could not be used, to be reported to the host once it is
    /// listening.
    pub problems: Vec<String>,
}

impl Config {
    /// Reads `--text-hook <json>` and `--image-hook <json>` (any number of times) from the
    /// arguments; anything else is a problem to report, not a reason to refuse to start.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        let mut config = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let (flag, kind) = match arg.as_str() {
                "--text-hook" => ("--text-hook", Kind::Text),
                "--image-hook" => ("--image-hook", Kind::Image),
                _ => {
                    config.problems.push(format!("ignored argument '{arg}'"));
                    continue;
                }
            };
            match args.next().map(|json| serde_json::from_str::<Hook>(&json)) {
                // The flag decides the kind, whatever the json says.
                Some(Ok(hook)) => config.hooks.push(Hook { kind, ..hook }),
                Some(Err(e)) => config.problems.push(format!("ignored a {flag}: {e}")),
                None => config.problems.push(format!("{flag} needs a value")),
            }
        }
        config
    }
}

enum Message {
    /// A line a worker wants written to the host (a `read_file` request).
    Send(String),
    Event(Box<EventParams>),
    /// A worker finished the file of that `generation`.
    Loaded {
        generation: u64,
        path: PathBuf,
        content: PreviewContent,
    },
    /// The host closed its end.
    Gone,
}

/// The requests workers have sent and are waiting on, by id.
#[derive(Default)]
struct Rpc {
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, Sender<Result<Value, String>>>>,
}

impl Rpc {
    fn expect(&self) -> (u64, Receiver<Result<Value, String>>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = channel();
        self.waiting.lock().expect("not poisoned").insert(id, tx);
        (id, rx)
    }

    fn forget(&self, id: u64) {
        self.waiting.lock().expect("not poisoned").remove(&id);
    }

    fn answer(&self, id: u64, reply: Result<Value, String>) {
        if let Some(waiter) = self.waiting.lock().expect("not poisoned").remove(&id) {
            let _ = waiter.send(reply);
        }
    }

    /// The host is gone: nothing more will be answered, so wake every worker with an error.
    fn fail_all(&self) {
        for (_, waiter) in self.waiting.lock().expect("not poisoned").drain() {
            let _ = waiter.send(Err("the host went away".to_string()));
        }
    }
}

/// One worker's way to the host, which stops asking once its file is no longer wanted.
struct WorkerRemote {
    rpc: Arc<Rpc>,
    messages: Sender<Message>,
    stop: Arc<AtomicBool>,
}

impl Remote for WorkerRemote {
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let cancelled = || "the file is no longer being previewed".to_string();
        if self.stop.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let (id, reply) = self.rpc.expect();
        if self
            .messages
            .send(Message::Send(request_line(id, method, &params)))
            .is_err()
        {
            self.rpc.forget(id);
            return Err("the plugin is shutting down".to_string());
        }
        let answer = reply.recv_timeout(REPLY_TIMEOUT).map_err(|_| {
            self.rpc.forget(id);
            "the host did not answer in time".to_string()
        })?;
        if self.stop.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        answer
    }
}

/// The newest file asked for, and how to tell its worker the answer is no longer wanted.
struct Current {
    generation: u64,
    stop: Arc<AtomicBool>,
}

/// Reads the host's lines from `input` and answers on `output` until the host closes the input.
///
/// # Errors
///
/// Only a failure to write to `output`, which means the host is gone.
pub fn run(
    input: impl BufRead + Send + 'static,
    mut output: impl Write,
    config: Config,
) -> io::Result<()> {
    let (messages, queue) = channel();
    let rpc = Arc::new(Rpc::default());
    {
        let (messages, rpc) = (messages.clone(), Arc::clone(&rpc));
        thread::spawn(move || read_lines(input, &messages, &rpc));
    }

    let hooks = Arc::new(config.hooks);
    let mut problems = config.problems;
    let mut current = Current {
        generation: 0,
        stop: Arc::new(AtomicBool::new(false)),
    };
    while let Ok(message) = queue.recv() {
        match message {
            Message::Send(line) => {
                output.write_all(line.as_bytes())?;
                output.flush()?;
            }
            Message::Event(params) => match params.name.as_str() {
                EVENT_INIT if params.api_version != API_VERSION => {
                    let message = format!(
                        "the host speaks plugin API {} and this was built for {API_VERSION}",
                        params.api_version
                    );
                    output.write_all(log_line("error", &message).as_bytes())?;
                    output.flush()?;
                    return Ok(());
                }
                EVENT_INIT => {
                    for problem in problems.drain(..) {
                        output.write_all(log_line("warn", &problem).as_bytes())?;
                    }
                    output.flush()?;
                }
                EVENT_PREVIEW => {
                    current.stop.store(true, Ordering::Relaxed);
                    current = Current {
                        generation: current.generation + 1,
                        stop: Arc::new(AtomicBool::new(false)),
                    };
                    if let Some(path) = params.preview.and_then(|target| target.path) {
                        start_worker(
                            path,
                            &current,
                            &hooks,
                            WorkerRemote {
                                rpc: Arc::clone(&rpc),
                                messages: messages.clone(),
                                stop: Arc::clone(&current.stop),
                            },
                        );
                    }
                }
                _ => {}
            },
            Message::Loaded {
                generation,
                path,
                content,
            } => {
                // An answer for a file the cursor has left is not worth the host's time.
                if generation == current.generation {
                    let set = SetPreview { path, content };
                    output.write_all(notification_line(METHOD_SET_PREVIEW, &set).as_bytes())?;
                    output.flush()?;
                } else {
                    hook::discard(&content);
                }
            }
            Message::Gone => return Ok(()),
        }
    }
    Ok(())
}

fn log_line(level: &str, message: &str) -> String {
    notification_line("log", &json!({ "level": level, "message": message }))
}

/// Loads `path` on a thread of its own and reports the document, or that it could not be made.
fn start_worker(path: PathBuf, current: &Current, hooks: &Arc<Vec<Hook>>, remote: WorkerRemote) {
    let generation = current.generation;
    let stop = Arc::clone(&current.stop);
    let hooks = Arc::clone(hooks);
    let messages = remote.messages.clone();
    thread::spawn(move || {
        let vfs = RemoteVfs::new(Arc::new(remote));
        let wanted = || !stop.load(Ordering::Relaxed);
        // A panic in the parsers on some odd input must not leave the column on "loading…".
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            load::content(&vfs, &path, &hooks, &|| !wanted())
        }));
        let content = match outcome {
            Ok(Some(content)) => content,
            Ok(None) => return,
            Err(_) => PreviewContent::Failed,
        };
        if wanted() {
            let _ = messages.send(Message::Loaded {
                generation,
                path,
                content,
            });
        } else {
            hook::discard(&content);
        }
    });
}

fn read_lines(input: impl BufRead, messages: &Sender<Message>, rpc: &Rpc) {
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        match parse_incoming(&line) {
            Ok(Incoming::Event(params)) => {
                if messages.send(Message::Event(params)).is_err() {
                    return;
                }
            }
            Ok(Incoming::Reply { id, error, result }) => {
                if let Some(id) = id.as_u64() {
                    rpc.answer(
                        id,
                        match error {
                            Some(message) => Err(message),
                            None => result.ok_or_else(|| "an empty reply".to_string()),
                        },
                    );
                }
            }
            // A line this plugin cannot read is not worth stopping for.
            Err(_) => {}
        }
    }
    rpc.fail_all();
    let _ = messages.send(Message::Gone);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_string()).collect()
    }

    #[test]
    fn text_hooks_are_read_from_the_arguments_and_bad_ones_are_reported() {
        let config = Config::from_args(args(&[
            "--text-hook",
            r#"{"extensions":["pdf"],"command":"pdftotext {in} {out}","timeout_ms":5000}"#,
            "--text-hook",
            "not json",
            "--surprise",
            "--text-hook",
        ]));
        assert_eq!(config.hooks.len(), 1);
        assert_eq!(config.hooks[0].extensions, ["pdf"]);
        assert_eq!(config.hooks[0].timeout_ms, Some(5000));
        assert_eq!(config.problems.len(), 3, "{:?}", config.problems);
    }

    #[test]
    fn an_image_hook_argument_makes_a_picture_hook_whatever_its_json_says() {
        let config = Config::from_args(args(&[
            "--image-hook",
            r#"{"extensions":["mp4"],"command":"thumb {in} {out}","kind":"text"}"#,
            "--image-hook",
        ]));
        assert_eq!(config.hooks.len(), 1);
        assert_eq!(config.hooks[0].kind, Kind::Image);
        assert_eq!(config.problems, ["--image-hook needs a value"]);
    }

    #[test]
    fn with_no_arguments_there_are_no_hooks_and_no_problems() {
        let config = Config::from_args(Vec::new());
        assert!(config.hooks.is_empty() && config.problems.is_empty());
    }

    #[test]
    fn an_answer_reaches_the_worker_waiting_for_it_and_only_that_one() {
        let rpc = Rpc::default();
        let (first, first_rx) = rpc.expect();
        let (second, second_rx) = rpc.expect();
        assert_ne!(first, second);

        rpc.answer(second, Ok(json!({ "n": 2 })));
        assert_eq!(second_rx.try_recv().unwrap(), Ok(json!({ "n": 2 })));
        assert!(first_rx.try_recv().is_err(), "the other still waits");

        rpc.answer(first, Err("refused".into()));
        assert_eq!(first_rx.try_recv().unwrap(), Err("refused".to_string()));
        // An answer nobody waits for is dropped, not a panic.
        rpc.answer(99, Ok(Value::Null));
    }

    #[test]
    fn when_the_host_goes_away_every_waiting_worker_is_woken_with_an_error() {
        let rpc = Rpc::default();
        let (_, one) = rpc.expect();
        let (_, two) = rpc.expect();
        rpc.fail_all();
        assert!(one.try_recv().unwrap().is_err());
        assert!(two.try_recv().unwrap().is_err());
    }

    #[test]
    fn a_worker_whose_file_was_left_asks_the_host_nothing() {
        let (messages, queue) = channel();
        let remote = WorkerRemote {
            rpc: Arc::new(Rpc::default()),
            messages,
            stop: Arc::new(AtomicBool::new(true)),
        };
        assert!(remote.call("read_file", json!({})).is_err());
        assert!(queue.try_recv().is_err(), "no request was sent");
    }
}
