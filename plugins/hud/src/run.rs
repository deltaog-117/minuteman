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

//! Drives the pill from the lines on stdin. A reader thread turns each line into a message; the
//! loop below owns the output, rereads `/proc` every [`REFRESH_EVERY`] and sends the pill only
//! when its words changed, so a quiet machine costs the host nothing.

use std::io::{self, BufRead, Write};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use plugin_api::client::{Incoming, notification_line, parse_incoming};
use plugin_api::{API_VERSION, EVENT_INIT, METHOD_SET_VIEW, SetView, Slot, Span, Tone, View};
use serde_json::json;

use crate::figures::Figures;

/// Time between rereads of the load average and memory. They do not change fast enough to be
/// worth more, and each change redraws the header.
pub const REFRESH_EVERY: Duration = Duration::from_secs(2);

/// What the host's lines came to.
enum Message {
    /// An `init` from a host built for another protocol version.
    Incompatible(u32),
    Gone,
}

/// The pill for `words`.
pub fn pill(words: String) -> SetView {
    SetView {
        slot: Slot::HeaderPill,
        view: Some(View::line(vec![Span::text(words, Tone::Normal)])),
    }
}

/// Shows the pill at once, keeps it current, and answers on `output` until the host closes the
/// input.
///
/// # Errors
///
/// Only a failure to write to `output`, which means the host is gone.
pub fn run(input: impl BufRead + Send + 'static, mut output: impl Write) -> io::Result<()> {
    let (messages, queue) = channel();
    thread::spawn(move || read_lines(input, &messages));

    let started = Instant::now();
    let mut shown = None;
    loop {
        let words = Figures::read().summary(started.elapsed());
        if shown.as_ref() != Some(&words) {
            output
                .write_all(notification_line(METHOD_SET_VIEW, &pill(words.clone())).as_bytes())?;
            output.flush()?;
            shown = Some(words);
        }
        match queue.recv_timeout(REFRESH_EVERY) {
            Ok(Message::Incompatible(host)) => {
                let message = format!("built for api version {API_VERSION}, host speaks {host}");
                output.write_all(
                    notification_line("log", &json!({ "level": "error", "message": message }))
                        .as_bytes(),
                )?;
                output.flush()?;
                return Ok(());
            }
            Ok(Message::Gone) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

fn read_lines(input: impl BufRead, messages: &Sender<Message>) {
    for line in input.lines() {
        let Ok(line) = line else { break };
        // A line this plugin cannot read, or an event it has no use for, is not worth stopping for.
        if let Ok(Incoming::Event(params)) = parse_incoming(&line)
            && params.name == EVENT_INIT
            && params.api_version != API_VERSION
            && messages
                .send(Message::Incompatible(params.api_version))
                .is_err()
        {
            return;
        }
    }
    let _ = messages.send(Message::Gone);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    /// A writer the test can read after the plugin has finished with it.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("not poisoned").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn lines_of(shared: &Shared) -> Vec<serde_json::Value> {
        let bytes = shared.0.lock().expect("not poisoned").clone();
        String::from_utf8(bytes)
            .expect("utf-8")
            .lines()
            .map(|line| serde_json::from_str(line).expect("each line is JSON"))
            .collect()
    }

    fn init(version: u32) -> String {
        format!(
            r#"{{"jsonrpc":"2.0","method":"event","params":{{"name":"init","api_version":{version},"cwd":"/","selection":[]}}}}"#
        )
    }

    #[test]
    fn the_pill_is_shown_before_anything_is_asked() {
        let out = Shared::default();
        run(Cursor::new(format!("{}\n", init(API_VERSION))), out.clone())
            .expect("the host closing the input ends it");
        let lines = lines_of(&out);
        assert_eq!(lines[0]["method"], "set_view");
        assert_eq!(lines[0]["params"]["slot"], "header_pill");
        let words = lines[0]["params"]["view"]["lines"][0]["spans"][0]["text"]
            .as_str()
            .expect("text");
        assert!(words.starts_with("up "), "{words:?}");
    }

    #[test]
    fn an_incompatible_host_gets_a_logged_refusal_and_the_plugin_stops() {
        let out = Shared::default();
        run(Cursor::new(format!("{}\n", init(999))), out.clone()).expect("a clean stop");
        let lines = lines_of(&out);
        let log = lines.last().expect("something was written");
        assert_eq!(log["method"], "log");
        assert_eq!(log["params"]["level"], "error");
    }

    #[test]
    fn unreadable_lines_do_not_stop_it() {
        let out = Shared::default();
        run(Cursor::new("not json\n\n{}\n"), out.clone()).expect("a clean stop at end of input");
        assert_eq!(lines_of(&out).len(), 1);
    }
}
