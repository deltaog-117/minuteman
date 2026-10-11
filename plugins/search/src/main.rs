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

//! `mman-search`: the structured search, run by Minuteman as a plugin. It speaks the plugin
//! protocol on its own stdin and stdout and does nothing else; see the `search` library for what
//! it does.

use std::io::{self, BufReader};
use std::process::ExitCode;

fn main() -> ExitCode {
    match search::run(BufReader::new(io::stdin()), io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mman-search: {e}");
            ExitCode::FAILURE
        }
    }
}
