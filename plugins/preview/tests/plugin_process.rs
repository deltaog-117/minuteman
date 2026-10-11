//! Runs the real `mman-preview` program with this test standing in for the host: it sends the
//! events a host sends, answers the plugin's `read_file` requests from a scratch folder on disk and
//! reads the documents that come back. This is the whole path — argument parsing, the reader and
//! writer threads, the workers, the cancellation — which the unit tests only see in pieces.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread;
use std::time::Duration;

use plugin_api::{
    API_VERSION, ChunkKind, FileChunk, ImageSource, PreviewContent, ReadFile, SetPreview, Token,
    is_handoff_path,
};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(20);

struct Plugin {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Value>,
}

impl Plugin {
    fn start(args: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mman-preview"))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the plugin program starts");
        let stdout = child.stdout.take().expect("piped");
        let stdin = child.stdin.take();
        let (tx, lines) = channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let value: Value = serde_json::from_str(&line).expect("each line is JSON");
                if tx.send(value).is_err() {
                    return;
                }
            }
        });
        let mut plugin = Self {
            child,
            stdin,
            lines,
        };
        plugin.send(&event("init", json!({})));
        plugin
    }

    fn send(&mut self, value: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        writeln!(stdin, "{value}").expect("the plugin is listening");
        stdin.flush().expect("flushed");
    }

    fn preview(&mut self, path: Option<&Path>) {
        let target = json!({ "path": path });
        self.send(&event("preview", json!({ "preview": target })));
    }

    fn next(&self) -> Option<Value> {
        match self.lines.recv_timeout(WAIT) {
            Ok(value) => Some(value),
            Err(RecvTimeoutError::Timeout) => panic!("the plugin said nothing for {WAIT:?}"),
            Err(RecvTimeoutError::Disconnected) => None,
        }
    }

    /// Answers `read_file` requests from the disk until a `set_preview` arrives, and returns it.
    fn document(&mut self) -> SetPreview {
        loop {
            let line = self.next().expect("the plugin is still running");
            match line["method"].as_str() {
                Some("read_file") => self.answer(&line),
                Some("set_preview") => {
                    return serde_json::from_value(line["params"].clone()).expect("a document");
                }
                other => panic!("unexpected line from the plugin: {other:?} {line}"),
            }
        }
    }

    fn answer(&mut self, request: &Value) {
        let reply = match read_from_disk(&request["params"]) {
            Ok(chunk) => json!({ "jsonrpc": "2.0", "id": request["id"], "result": chunk }),
            Err(message) => {
                json!({ "jsonrpc": "2.0", "id": request["id"], "error": { "message": message } })
            }
        };
        self.send(&reply);
    }

    /// Closes the plugin's input and returns every line it wrote before it ended.
    fn finish(mut self) -> Vec<Value> {
        drop(self.stdin.take());
        let mut rest = Vec::new();
        while let Some(line) = self.next() {
            rest.push(line);
        }
        let status = self.child.wait().expect("it ends");
        assert!(status.success(), "the plugin ended badly: {status}");
        rest
    }
}

fn event(name: &str, extra: Value) -> Value {
    let mut params = json!({
        "name": name,
        "api_version": API_VERSION,
        "cwd": "/",
        "selection": [],
    });
    params
        .as_object_mut()
        .expect("an object")
        .extend(extra.as_object().expect("an object").clone());
    json!({ "jsonrpc": "2.0", "method": "event", "params": params })
}

/// What the host's `read_file` does, for a plain folder on disk.
fn read_from_disk(params: &Value) -> Result<FileChunk, String> {
    let request: ReadFile = serde_json::from_value(params.clone()).map_err(|e| e.to_string())?;
    let meta = std::fs::metadata(&request.path).map_err(|e| e.to_string())?;
    let kind = if meta.is_file() {
        ChunkKind::File
    } else if meta.is_dir() {
        ChunkKind::Dir
    } else {
        ChunkKind::Other
    };
    let mut data = Vec::new();
    if kind == ChunkKind::File && request.len > 0 {
        let mut file = std::fs::File::open(&request.path).map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(request.offset))
            .map_err(|e| e.to_string())?;
        file.take(request.len)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
    }
    Ok(FileChunk {
        kind,
        size: meta.len(),
        local_path: Some(request.path),
        data,
    })
}

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "minuteman-preview-plugin-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    dir
}

#[test]
fn a_source_file_comes_back_as_colored_text() {
    let dir = scratch("text");
    let path = dir.join("main.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();

    let mut plugin = Plugin::start(&[]);
    plugin.preview(Some(&path));
    let set = plugin.document();

    assert_eq!(set.path, path);
    let PreviewContent::Text { lines } = set.content else {
        panic!("expected text, got {:?}", set.content);
    };
    assert!(
        lines[0].contains(&(Token::Keyword, "fn".to_string())),
        "{lines:?}"
    );
    assert!(plugin.finish().is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_binary_file_comes_back_as_its_first_bytes_and_a_zip_as_a_listing() {
    let dir = scratch("kinds");
    let blob = dir.join("blob.bin");
    std::fs::write(&blob, [0u8, 1, 2, 255].repeat(40_000)).unwrap();
    let archive = dir.join("a.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
    writer
        .start_file("inside.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    Write::write_all(&mut writer, b"hi").unwrap();
    writer.finish().unwrap();

    let mut plugin = Plugin::start(&[]);
    plugin.preview(Some(&blob));
    let PreviewContent::Hex(head) = plugin.document().content else {
        panic!("expected a hex head");
    };
    assert_eq!(head.bytes.len(), 64 * 1024);
    assert_eq!(head.total, 160_000);
    assert!(head.is_cut());

    plugin.preview(Some(&archive));
    let PreviewContent::Archive(listing) = plugin.document().content else {
        panic!("expected a listing");
    };
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].name, "inside.txt");
    plugin.finish();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_folder_and_a_missing_file_have_nothing_to_show() {
    let dir = scratch("nothing");

    let mut plugin = Plugin::start(&[]);
    for path in [dir.clone(), dir.join("missing")] {
        plugin.preview(Some(&path));
        let set = plugin.document();
        assert_eq!(set.path, path);
        assert_eq!(set.content, PreviewContent::Empty, "{path:?}");
    }
    plugin.finish();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_picture_is_named_for_the_interface_to_read_itself() {
    let dir = scratch("picture");
    let picture = dir.join("pic.png");
    std::fs::write(&picture, b"not really a png").unwrap();

    let mut plugin = Plugin::start(&[]);
    plugin.preview(Some(&picture));
    let set = plugin.document();
    assert_eq!(set.path, picture);
    assert_eq!(
        set.content,
        PreviewContent::Image {
            source: ImageSource::Original
        }
    );
    plugin.finish();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_picture_hook_given_on_the_command_line_hands_over_the_file_it_made() {
    let dir = scratch("image-hook");
    let path = dir.join("clip.mp4");
    std::fs::write(&path, b"\0 not really a video").unwrap();
    let hook = json!({
        "extensions": ["mp4"],
        "command": "printf FRAME > {out}",
    })
    .to_string();

    let mut plugin = Plugin::start(&["--image-hook", &hook]);
    plugin.preview(Some(&path));
    let PreviewContent::Image {
        source: ImageSource::Handoff { file },
    } = plugin.document().content
    else {
        panic!("expected a handed-over picture");
    };
    assert!(is_handoff_path(&file), "{file:?}");
    assert_eq!(std::fs::read(&file).unwrap(), b"FRAME");
    std::fs::remove_file(&file).unwrap();
    plugin.finish();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_the_cursor_has_left_is_never_sent() {
    let dir = scratch("stale");
    let slow = dir.join("slow.rs");
    let quick = dir.join("quick.rs");
    std::fs::write(&slow, "fn slow() {}\n").unwrap();
    std::fs::write(&quick, "fn quick() {}\n").unwrap();

    let mut plugin = Plugin::start(&[]);
    plugin.preview(Some(&slow));
    // Hold the first file's first request unanswered while the cursor moves on.
    let held = loop {
        let line = plugin.next().expect("running");
        if line["method"] == "read_file" {
            break line;
        }
    };
    plugin.preview(Some(&quick));
    plugin.answer(&held);

    let set = plugin.document();
    assert_eq!(set.path, quick, "only the newest file is answered");
    let rest = plugin.finish();
    assert!(
        rest.iter().all(|line| line["method"] != "set_preview"),
        "a stale document was sent: {rest:?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn moving_to_no_file_cancels_what_was_loading_and_sends_nothing() {
    let dir = scratch("none");
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    let mut plugin = Plugin::start(&[]);
    plugin.preview(Some(&file));
    let held = loop {
        let line = plugin.next().expect("running");
        if line["method"] == "read_file" {
            break line;
        }
    };
    plugin.preview(None);
    plugin.answer(&held);
    let rest = plugin.finish();
    assert!(
        rest.iter().all(|line| line["method"] != "set_preview"),
        "{rest:?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_text_hook_given_on_the_command_line_extracts_a_file_the_plugin_cannot_read() {
    let dir = scratch("hook");
    let path = dir.join("report.pdf");
    std::fs::write(&path, b"%PDF-1.4 \0\0 binary").unwrap();
    let hook = json!({
        "extensions": ["pdf"],
        "command": "printf 'the extracted words' > {out}",
    })
    .to_string();

    let mut plugin = Plugin::start(&["--text-hook", &hook]);
    plugin.preview(Some(&path));
    let PreviewContent::Text { lines } = plugin.document().content else {
        panic!("expected the hook's text");
    };
    assert_eq!(
        lines,
        vec![vec![(Token::Plain, "the extracted words".to_string())]]
    );
    plugin.finish();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_bad_argument_is_reported_to_the_host_after_init_and_does_not_stop_the_plugin() {
    let plugin = Plugin::start(&["--text-hook", "nonsense"]);
    let line = plugin.next().expect("a log line");
    assert_eq!(line["method"], "log");
    assert_eq!(line["params"]["level"], "warn");
    plugin.finish();
}

#[test]
fn a_host_built_for_another_api_version_is_refused_with_a_logged_error() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mman-preview"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let wrong = r#"{"jsonrpc":"2.0","method":"event","params":{"name":"init","api_version":999,"cwd":"/","selection":[]}}"#;
    writeln!(child.stdin.as_mut().unwrap(), "{wrong}").unwrap();
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    assert!(child.wait().unwrap().success());
    let line: Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(line["method"], "log");
    assert_eq!(line["params"]["level"], "error");
}
