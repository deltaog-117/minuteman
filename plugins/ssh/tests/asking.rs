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

//! Connecting when `ssh` has to ask: an unknown host key, a key with a passphrase. The plugin's
//! `Server` is run against a real `sshd` on a loopback port, with a scripted stand-in for the user
//! and this package's own executable as the helper `ssh` runs to ask. A machine without `sshd`
//! skips these (and says so).

#[path = "../../../crates/vfs_remote/tests/harness/sshd.rs"]
mod sshd;

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use plugin_api::{AskAnswer, AskKind, AskSpec, VfsFault, VfsRequest};
use remoter::{Ask, ConnectOptions, HostKeys, Server};
use serde_json::json;
use sshd::{Sshd, scratch};

/// Answers each question with the next of a prepared list, and keeps the questions.
#[derive(Debug, Default)]
struct Scripted {
    asked: Mutex<Vec<AskSpec>>,
    answers: Mutex<VecDeque<Result<AskAnswer, String>>>,
}

impl Scripted {
    fn answering(answers: impl IntoIterator<Item = Result<AskAnswer, String>>) -> Arc<Self> {
        Arc::new(Self {
            asked: Mutex::default(),
            answers: Mutex::new(answers.into_iter().collect()),
        })
    }

    fn asked(&self) -> Vec<AskSpec> {
        self.asked.lock().unwrap().clone()
    }
}

impl Ask for Scripted {
    fn ask(&self, spec: AskSpec) -> Result<AskAnswer, String> {
        self.asked.lock().unwrap().push(spec);
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err("nobody to ask".to_owned()))
    }
}

fn yes(yes: bool) -> Result<AskAnswer, String> {
    Ok(AskAnswer::Confirm { yes })
}

fn secret(text: &str) -> Result<AskAnswer, String> {
    Ok(AskAnswer::Secret {
        secret: serde_json::from_value(json!(text)).expect("a password"),
    })
}

fn server(sshd: &Sshd, host_keys: HostKeys, key: &str, asker: Option<Arc<Scripted>>) -> Server {
    let server = Server::new(ConnectOptions {
        host_keys,
        known_hosts_file: Some(sshd.dir.join("known_hosts")),
        identity: Some(sshd.dir.join(key)),
        connect_timeout: Duration::from_secs(10),
        asker: remoter::Asker::default(),
        askpass_program: Some(PathBuf::from(env!("CARGO_BIN_EXE_mman-ssh"))),
    })
    .expect("a server");
    if let Some(asker) = asker {
        server.asker().install(asker);
    }
    server
}

fn list(server: &Server, sshd: &Sshd, folder: &std::path::Path) -> Result<(), VfsFault> {
    server
        .answer(&VfsRequest::List {
            path: sshd.remote(folder),
        })
        .map(|_| ())
}

/// A second key for the same user, locked with `passphrase`, and authorized.
fn locked_key(sshd: &Sshd, passphrase: &str) {
    let made = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", passphrase, "-f"])
        .arg(sshd.dir.join("locked_key"))
        .status()
        .expect("ssh-keygen runs");
    assert!(made.success());
    let mut authorized = fs::read_to_string(sshd.dir.join("authorized_keys")).unwrap();
    authorized.push_str(&fs::read_to_string(sshd.dir.join("locked_key.pub")).unwrap());
    fs::write(sshd.dir.join("authorized_keys"), authorized).unwrap();
}

fn message(fault: &VfsFault) -> String {
    format!("{fault:?}")
}

#[test]
fn an_unknown_host_key_is_shown_and_trusted_when_the_user_says_yes() {
    let Some(sshd) = Sshd::start("ask-yes") else {
        return;
    };
    let folder = scratch("ask-yes");
    let user = Scripted::answering([yes(true)]);
    let server = server(&sshd, HostKeys::Ask, "user_key", Some(user.clone()));

    list(&server, &sshd, &folder).expect("connects once the key is trusted");

    let asked = user.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].kind, AskKind::Confirm);
    assert!(
        asked[0].prompt.contains("fingerprint"),
        "{}",
        asked[0].prompt
    );
    let known = fs::read_to_string(sshd.dir.join("known_hosts")).unwrap_or_default();
    assert!(
        known.contains("ssh-ed25519"),
        "the key is remembered: {known:?}"
    );
}

#[test]
fn an_unknown_host_key_the_user_refuses_is_not_connected_to_or_remembered() {
    let Some(sshd) = Sshd::start("ask-no") else {
        return;
    };
    let folder = scratch("ask-no");
    let user = Scripted::answering([yes(false)]);
    let server = server(&sshd, HostKeys::Ask, "user_key", Some(user));

    let refused = list(&server, &sshd, &folder).expect_err("a refused key does not connect");

    assert!(
        message(&refused).contains("Host key verification failed"),
        "{refused:?}"
    );
    let known = fs::read_to_string(sshd.dir.join("known_hosts")).unwrap_or_default();
    assert!(known.is_empty(), "{known:?}");
}

#[test]
fn a_question_the_user_backs_out_of_fails_the_connection() {
    let Some(sshd) = Sshd::start("ask-esc") else {
        return;
    };
    let folder = scratch("ask-esc");
    let user = Scripted::answering([Err("declined".to_owned())]);
    let server = server(&sshd, HostKeys::Ask, "user_key", Some(user));

    assert!(list(&server, &sshd, &folder).is_err());
}

#[test]
fn nobody_to_ask_means_an_unknown_key_is_refused_at_once() {
    let Some(sshd) = Sshd::start("ask-none") else {
        return;
    };
    let folder = scratch("ask-none");
    let server = server(&sshd, HostKeys::Ask, "user_key", None);

    let started = Instant::now();
    let refused = list(&server, &sshd, &folder).expect_err("no one can trust the key");

    assert!(
        message(&refused).contains("Host key verification failed"),
        "{refused:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(8));
}

#[test]
fn a_key_passphrase_is_asked_for_and_typed_without_being_shown() {
    let Some(sshd) = Sshd::start("ask-passphrase") else {
        return;
    };
    let folder = scratch("ask-passphrase");
    locked_key(&sshd, "open sesame");
    let user = Scripted::answering([secret("open sesame")]);
    let server = server(&sshd, HostKeys::AcceptNew, "locked_key", Some(user.clone()));

    list(&server, &sshd, &folder).expect("the right passphrase unlocks the key");

    let asked = user.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].kind, AskKind::Secret);
    assert!(
        asked[0].prompt.to_lowercase().contains("passphrase"),
        "{}",
        asked[0].prompt
    );
}

#[test]
fn a_wrong_passphrase_fails_after_ssh_gives_up_asking() {
    let Some(sshd) = Sshd::start("ask-wrong") else {
        return;
    };
    let folder = scratch("ask-wrong");
    locked_key(&sshd, "open sesame");
    let user = Scripted::answering([secret("nope"), secret("nope"), secret("nope")]);
    let server = server(&sshd, HostKeys::AcceptNew, "locked_key", Some(user.clone()));

    assert!(list(&server, &sshd, &folder).is_err());
    assert!(!user.asked().is_empty());
}
