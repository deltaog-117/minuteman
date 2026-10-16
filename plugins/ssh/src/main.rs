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

use std::io::{self, BufReader};
use std::process::ExitCode;

use remoter::{ConnectOptions, Server};

fn main() -> ExitCode {
    // `ssh` runs this executable to ask for a password; it is not the plugin then.
    if let Some(code) = remoter::run_as_helper() {
        return code;
    }
    let server = match Server::new(ConnectOptions::from_env()) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("mman-ssh: {e}");
            return ExitCode::FAILURE;
        }
    };
    match remoter::run(BufReader::new(io::stdin()), io::stdout(), server) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mman-ssh: {e}");
            ExitCode::FAILURE
        }
    }
}
