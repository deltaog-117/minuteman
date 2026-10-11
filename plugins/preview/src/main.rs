//! `mman-preview`: the preview column, run by Minuteman as a plugin. It speaks the plugin protocol
//! on its own stdin and stdout and does nothing else; see the `previewer` library for what it
//! does. `--text-hook <json>` (any number of times) adds a text-extracting command.

use std::io::{self, BufReader};
use std::process::ExitCode;

use previewer::Config;

fn main() -> ExitCode {
    let config = Config::from_args(std::env::args().skip(1));
    match previewer::run(BufReader::new(io::stdin()), io::stdout().lock(), config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mman-preview: {e}");
            ExitCode::FAILURE
        }
    }
}
