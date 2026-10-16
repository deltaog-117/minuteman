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

//! How an answer gets from the user to `ssh`. `ssh` has no way to be given a password, but it runs
//! the program named by `SSH_ASKPASS` with the question as its one argument and reads the answer
//! from that program's output. That program is this plugin's own executable, which runs as the
//! helper when `MMAN_SSH_ASKPASS` names a socket: it forwards the question there, and prints what
//! comes back.
//!
//! The socket lives in a folder only the user can open and exists only while a connection is being
//! made, so nothing else can ask through it or listen to the answers.
//!
//! The wire is one JSON line each way: `{"kind":"secret"|"confirm","prompt":"..."}` and then
//! `{"answer":"..."}` or `{"error":"..."}`.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use plugin_api::{AskAnswer, AskKind, AskSpec};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader as AsyncBufReader};
use tokio::net::{UnixListener, UnixStream as AsyncStream};
use tokio::task::AbortHandle;
use zeroize::Zeroizing;

use crate::ask::Ask;

/// The variable that makes this executable the helper, and names the socket it talks to.
pub const ENV_SOCKET: &str = "MMAN_SSH_ASKPASS";
/// What `ssh` sets to say the question wants a yes or a no (`confirm`), or only to be shown
/// (`none`); anything else wants a typed answer.
const ENV_PROMPT_KIND: &str = "SSH_ASKPASS_PROMPT";
/// How `ssh` ends the question about an unknown host key, which it does not mark as a yes or no
/// question: "Are you sure you want to continue connecting (yes/no/[fingerprint])?".
const YES_NO_MARK: &str = "(yes/no";
/// A question or an answer longer than this is not one.
const MAX_LINE: u64 = 16 * 1024;
const SOCKET_NAME: &str = "askpass";

#[derive(Serialize, Deserialize)]
struct Question {
    kind: AskKind,
    prompt: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Reply {
    Answer(String),
    Error(String),
}

/// The socket `ssh`'s helper asks through, served until dropped.
pub(crate) struct Relay {
    path: PathBuf,
    task: AbortHandle,
}

impl Relay {
    /// Listens in `dir` and relays every question to `asker`. Must be called inside the runtime.
    pub(crate) fn start(dir: &Path, asker: Arc<dyn Ask>) -> io::Result<Self> {
        let path = dir.join(SOCKET_NAME);
        let listener = UnixListener::bind(&path)?;
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(relay_one(stream, Arc::clone(&asker)));
            }
        })
        .abort_handle();
        Ok(Self { path, task })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn relay_one(stream: AsyncStream, asker: Arc<dyn Ask>) {
    let (read, mut write) = stream.into_split();
    let mut line = String::new();
    let read_ok = AsyncBufReader::new(read.take(MAX_LINE))
        .read_line(&mut line)
        .await
        .is_ok();
    let reply = match serde_json::from_str::<Question>(&line) {
        Ok(question) if read_ok => {
            let spec = AskSpec {
                kind: question.kind,
                prompt: question.prompt,
            }
            .checked();
            match tokio::task::spawn_blocking(move || asker.ask(spec)).await {
                Ok(Ok(AskAnswer::Secret { secret })) => Reply::Answer(secret.as_str().to_owned()),
                Ok(Ok(AskAnswer::Confirm { yes })) => {
                    Reply::Answer(if yes { "yes" } else { "no" }.to_owned())
                }
                Ok(Err(why)) => Reply::Error(why),
                Err(_) => Reply::Error("the question could not be asked".to_owned()),
            }
        }
        _ => Reply::Error("not a question".to_owned()),
    };
    // The answer may be a password: the line it travels in is wiped once written.
    let text = Zeroizing::new(serde_json::to_string(&reply).unwrap_or_default() + "\n");
    let _ = write.write_all(text.as_bytes()).await;
    if let Reply::Answer(mut answer) = reply {
        zeroize::Zeroize::zeroize(&mut answer);
    }
}

/// `ssh`'s helper: asks `prompt` through `socket` and returns the answer to print.
fn ask_through(socket: &Path, kind: AskKind, prompt: &str) -> Result<Zeroizing<String>, String> {
    let mut stream =
        UnixStream::connect(socket).map_err(|e| format!("cannot reach the plugin: {e}"))?;
    let question = serde_json::to_string(&Question {
        kind,
        prompt: prompt.to_owned(),
    })
    .map_err(|e| e.to_string())?;
    writeln!(stream, "{question}").map_err(|e| e.to_string())?;
    let mut line = Zeroizing::new(String::new());
    BufReader::new(io::Read::take(&stream, MAX_LINE))
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    match serde_json::from_str::<Reply>(&line).map_err(|e| e.to_string())? {
        Reply::Answer(answer) => Ok(Zeroizing::new(answer)),
        Reply::Error(why) => Err(why),
    }
}

/// Runs as `ssh`'s helper if this process was started as one, and returns how it ended.
pub fn run_as_helper() -> Option<ExitCode> {
    let socket = std::env::var_os(ENV_SOCKET)?;
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let kind = match std::env::var(ENV_PROMPT_KIND).as_deref() {
        // Something to show, not to answer: there is nowhere to show it, and nothing to wait for.
        Ok("none") => return Some(ExitCode::SUCCESS),
        Ok("confirm") => AskKind::Confirm,
        _ if prompt.contains(YES_NO_MARK) => AskKind::Confirm,
        _ => AskKind::Secret,
    };
    Some(match ask_through(Path::new(&socket), kind, &prompt) {
        Ok(answer) => {
            let mut out = io::stdout().lock();
            if writeln!(out, "{}", answer.as_str())
                .and_then(|()| out.flush())
                .is_ok()
            {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(_) => ExitCode::FAILURE,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use plugin_api::AskKind;

    use super::*;

    #[derive(Debug)]
    struct Fake(Mutex<Vec<AskSpec>>, Result<AskAnswer, String>);

    impl Ask for Fake {
        fn ask(&self, spec: AskSpec) -> Result<AskAnswer, String> {
            self.0.lock().unwrap().push(spec);
            self.1.clone()
        }
    }

    fn secret(text: &str) -> AskAnswer {
        AskAnswer::Secret {
            secret: serde_json::from_str(&format!("\"{text}\"")).unwrap(),
        }
    }

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mman-askpass-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn a_question_reaches_the_asker_and_its_answer_comes_back() {
        let dir = scratch("answer");
        let fake = Arc::new(Fake(Mutex::default(), Ok(secret("hunter2"))));
        let rt = runtime();
        let _guard = rt.enter();
        let relay = Relay::start(&dir, fake.clone()).unwrap();
        let answer = ask_through(relay.path(), AskKind::Secret, "pw:").unwrap();
        assert_eq!(answer.as_str(), "hunter2");
        let asked = fake.0.lock().unwrap();
        assert_eq!(
            (asked[0].kind, asked[0].prompt.as_str()),
            (AskKind::Secret, "pw:")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_yes_or_no_is_spelled_the_way_ssh_reads_it() {
        let dir = scratch("confirm");
        let rt = runtime();
        let _guard = rt.enter();
        for (yes, word) in [(true, "yes"), (false, "no")] {
            let fake = Arc::new(Fake(Mutex::default(), Ok(AskAnswer::Confirm { yes })));
            let relay = Relay::start(&dir, fake).unwrap();
            let answer = ask_through(relay.path(), AskKind::Confirm, "trust?").unwrap();
            assert_eq!(answer.as_str(), word);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_declined_question_is_an_error_for_the_helper() {
        let dir = scratch("declined");
        let fake = Arc::new(Fake(Mutex::default(), Err("declined".to_owned())));
        let rt = runtime();
        let _guard = rt.enter();
        let relay = Relay::start(&dir, fake).unwrap();
        assert_eq!(
            ask_through(relay.path(), AskKind::Secret, "pw:").unwrap_err(),
            "declined"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_socket_goes_away_with_the_relay() {
        let dir = scratch("gone");
        let rt = runtime();
        let _guard = rt.enter();
        let fake = Arc::new(Fake(Mutex::default(), Err(String::new())));
        let relay = Relay::start(&dir, fake).unwrap();
        let path = relay.path().to_path_buf();
        assert!(path.exists());
        drop(relay);
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
