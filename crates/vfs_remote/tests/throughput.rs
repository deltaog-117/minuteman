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

//! How much slower a big copy is through the plugin than through the in-process backend it
//! replaces. Both talk to the same loopback `sshd`, so the SFTP and encryption cost is the same
//! and the difference is what the wire between the interface and the plugin adds: JSON lines,
//! hex-encoded bytes, a window of 256 KiB per round trip.
//!
//! Ignored by default because it moves a few hundred megabytes; run it with
//! `cargo test -p vfs_remote --release --test throughput -- --ignored --nocapture`.

mod harness;

use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use harness::{Sshd, scratch};
use shared::{LocalVfs, Vfs};
use vfs_ssh::{ConnectOptions, HostKeys, RoutedVfs};

const SIZE: usize = 256 << 20;

fn copy_time(vfs: &dyn Vfs, from: &std::path::Path, to: &std::path::Path) -> Duration {
    let start = Instant::now();
    vfs.copy_file(from, to).expect("the copy works");
    start.elapsed()
}

fn rate(elapsed: Duration) -> f64 {
    SIZE as f64 / (1 << 20) as f64 / elapsed.as_secs_f64()
}

#[test]
#[ignore = "moves hundreds of megabytes; run on request"]
fn a_large_copy_through_the_plugin_against_the_in_process_backend() {
    let Some(server) = Sshd::start("throughput") else {
        return;
    };
    let work = scratch("throughput");
    // Not all zeros: a compressing link would otherwise flatter whichever side it favours.
    let body: Vec<u8> = (0..SIZE)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    fs::write(work.join("big.bin"), &body).unwrap();

    let in_process = RoutedVfs::with_options(
        Arc::new(LocalVfs),
        ConnectOptions {
            host_keys: HostKeys::AcceptNew,
            known_hosts_file: Some(server.dir.join("known_hosts")),
            identity: Some(server.dir.join("user_key")),
            connect_timeout: Duration::from_secs(10),
        },
    )
    .expect("a router");
    let plugin = server.vfs();
    let remote_src = server.remote(&work.join("big.bin"));

    // Connect both sessions first, so the time measured is the copy and not the handshake.
    for vfs in [&in_process as &dyn Vfs, &plugin] {
        vfs.metadata(&remote_src).expect("a first request");
    }

    let rows: Vec<(&str, Duration, Duration)> = vec![
        (
            "download",
            copy_time(&in_process, &remote_src, &work.join("down_a.bin")),
            copy_time(&plugin, &remote_src, &work.join("down_b.bin")),
        ),
        (
            "upload",
            copy_time(
                &in_process,
                &work.join("big.bin"),
                &server.remote(&work.join("up_a.bin")),
            ),
            copy_time(
                &plugin,
                &work.join("big.bin"),
                &server.remote(&work.join("up_b.bin")),
            ),
        ),
    ];
    for (name, a, b) in &rows {
        eprintln!(
            "{name}: in-process {:.0} MiB/s ({a:.2?}), plugin {:.0} MiB/s ({b:.2?}), plugin is {:.2}x the time",
            rate(*a),
            rate(*b),
            b.as_secs_f64() / a.as_secs_f64()
        );
    }
    for file in ["down_a.bin", "down_b.bin", "up_a.bin", "up_b.bin"] {
        assert_eq!(
            fs::read(work.join(file)).unwrap(),
            body,
            "{file} arrived intact"
        );
    }
}
