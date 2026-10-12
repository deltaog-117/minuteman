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

//! How fast a big copy runs through the plugin, started and reached the way the interface does it:
//! by the plugin manager, over the `ServeLink` that `SshVfs` is built with in `main`. The loopback
//! `sshd` makes the SFTP and encryption cost the same on every run, so what moves the figure is
//! the wire between the interface and the plugin: JSON lines, hex-encoded bytes, windows of
//! 256 KiB with four in flight.
//!
//! Measured when the engine left the core (256 MiB, release, loopback): download 16 MiB/s
//! in-process against 22 MiB/s through the plugin, upload 53 against 122. A figure well below
//! those is a regression in the wire.
//!
//! Ignored by default because it moves a few hundred megabytes; run it with
//! `cargo test -p vfs_remote --release --test throughput -- --ignored --nocapture`.

mod harness;

use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use harness::{Sshd, scratch};
use shared::{LocalVfs, Vfs};

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
fn a_large_copy_through_the_plugin_over_the_real_link() {
    let Some(server) = Sshd::start("throughput") else {
        return;
    };
    let work = scratch("throughput");
    // Not all zeros: a compressing link would otherwise flatter the figure.
    let body: Vec<u8> = (0..SIZE)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    fs::write(work.join("big.bin"), &body).unwrap();

    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    let mut manager = plugin_host::PluginManager::new();
    let link = plugin_host::ServeLink::new();
    manager.set_serve_ssh_link(Arc::clone(&link));
    // `env` sets the connection options for the plugin without touching this process's own.
    let mut args: Vec<String> = server
        .env(true)
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    args.push(harness::plugin_binary().display().to_string());
    manager
        .spawn(
            runtime.handle(),
            "ssh",
            "env",
            &args,
            plugin_host::Triggers {
                actions: vec![plugin_api::PROVIDES_VFS_SSH.to_string()],
                ..Default::default()
            },
            &work,
        )
        .expect("the plugin starts");
    let vfs = vfs_remote::SshVfs::new(Arc::new(LocalVfs), link);
    let remote_src = server.remote(&work.join("big.bin"));

    // Connect first, so the time measured is the copy and not the handshake.
    vfs.metadata(&remote_src).expect("a first request");

    let down = copy_time(&vfs, &remote_src, &work.join("down.bin"));
    let up = copy_time(
        &vfs,
        &work.join("big.bin"),
        &server.remote(&work.join("up.bin")),
    );
    eprintln!(
        "download {:.0} MiB/s ({down:.2?}), upload {:.0} MiB/s ({up:.2?})",
        rate(down),
        rate(up)
    );
    for file in ["down.bin", "up.bin"] {
        assert_eq!(
            fs::read(work.join(file)).unwrap(),
            body,
            "{file} arrived intact"
        );
    }
}
