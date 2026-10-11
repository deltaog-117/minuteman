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

//! Whether what the user is carrying (the yanked or cut files, else the marked ones) would fit on a
//! device, worked out before anything is copied rather than discovered halfway through. The deck
//! shows the answer on each card and refuses to send a load that cannot fit.
//!
//! Pure: sizes and a device in, a verdict out. It is a prediction, not a promise: another program
//! can fill the disk between the check and the copy, and a disk with many small files uses more
//! room than their sizes add up to, which `Tight` allows for with a margin.

use std::path::PathBuf;

use shared::devices::Device;

use crate::hud::format_size;

/// FAT32 holds a file of at most 4 GiB less one byte.
const FAT_MAX_FILE: u64 = 4 * 1024 * 1024 * 1024 - 1;

/// A load that leaves less than this (or 2% of itself) free is called tight, not fitting: a
/// filesystem rounds every file up to a whole block, so many small files take more than their
/// lengths.
const TIGHT_FLOOR: u64 = 16 * 1024 * 1024;

/// What happens to the files at the other end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Carry {
    Copy,
    /// Moved: the originals go once they are there, which on the same disk takes no room at all.
    Move,
}

/// What is being carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Load {
    pub bytes: u64,
    /// `false` when measuring stopped at its limit, so `bytes` and `largest` are lower bounds.
    pub exact: bool,
    /// The biggest single file in the load.
    pub largest: u64,
    pub carry: Carry,
    /// Where the load comes from, to tell which device it already sits on.
    pub source: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Fits with `left` bytes to spare.
    Fits { left: u64 },
    /// Fits, but with so little to spare that block rounding could tip it over.
    Tight { left: u64 },
    /// Does not fit: `need` more bytes are wanted.
    Short { need: u64 },
    /// One file is bigger than the filesystem allows, whatever room there is.
    TooBig { limit: u64, fs: String },
    /// A move within one disk: nothing is copied, so nothing is needed.
    SameDisk,
    /// Not mounted, so its free space is not known.
    Unmounted,
    /// Free space is not known, or the load was too big to measure fully and what was measured
    /// fits: it may or may not.
    Unsure,
}

impl Verdict {
    /// Whether sending the load there should be allowed.
    pub fn allows(&self) -> bool {
        matches!(
            self,
            Verdict::Fits { .. } | Verdict::Tight { .. } | Verdict::SameDisk | Verdict::Unsure
        )
    }
}

/// The biggest file the filesystem called `fs` can hold, where it has a limit worth warning about.
pub fn max_file_size(fs: &str) -> Option<u64> {
    matches!(fs, "vfat" | "fat" | "fat32" | "msdos").then_some(FAT_MAX_FILE)
}

/// Whether `load` fits on `device`. `same_device` says the load already sits on it.
pub fn verdict(device: &Device, load: &Load, same_device: bool) -> Verdict {
    if !device.is_mounted() {
        return Verdict::Unmounted;
    }
    if load.carry == Carry::Move && same_device {
        return Verdict::SameDisk;
    }
    if let (Some(fs), Some(limit)) = (
        device.fs.as_deref(),
        device.fs.as_deref().and_then(max_file_size),
    ) && load.largest > limit
    {
        return Verdict::TooBig {
            limit,
            fs: fs.to_string(),
        };
    }
    if device.total == 0 {
        return Verdict::Unsure;
    }
    if load.bytes > device.free {
        return Verdict::Short {
            need: load.bytes - device.free,
        };
    }
    if !load.exact {
        return Verdict::Unsure;
    }
    let left = device.free - load.bytes;
    if left < TIGHT_FLOOR.max(load.bytes / 50) {
        Verdict::Tight { left }
    } else {
        Verdict::Fits { left }
    }
}

/// What a card says in the one line it can spare: a mark, then the answer.
pub fn card_text(verdict: &Verdict, ascii: bool) -> String {
    let (yes, no, dot) = if ascii {
        ("+", "x", "-")
    } else {
        ("✓", "✗", "·")
    };
    match verdict {
        Verdict::Fits { left } => format!("{yes} fits {dot} {} left", format_size(*left)),
        Verdict::Tight { left } => format!("~ tight {dot} {} left", format_size(*left)),
        Verdict::Short { need } => format!("{no} needs {} more", format_size(*need)),
        Verdict::TooBig { limit, .. } => {
            format!(
                "{no} a file is over {}",
                format_size(limit.saturating_add(1))
            )
        }
        Verdict::SameDisk => format!("{yes} same disk {dot} instant"),
        Verdict::Unmounted => "mount to check (Enter)".to_string(),
        Verdict::Unsure => "? cannot tell".to_string(),
    }
}

/// Why sending to `title` was refused, in a sentence for the status line.
pub fn refusal(verdict: &Verdict, title: &str) -> String {
    match verdict {
        Verdict::Short { need } => format!("{title}: needs {} more room", format_size(*need)),
        Verdict::TooBig { limit, fs } => format!(
            "{title}: a file is over {}, the most {} holds",
            format_size(limit.saturating_add(1)),
            fs.to_uppercase()
        ),
        Verdict::Unmounted => format!("{title} is not mounted: press m to mount it first"),
        _ => format!("{title}: cannot send there"),
    }
}

#[cfg(test)]
mod tests {
    use shared::devices::Kind;

    use super::*;

    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;

    fn device(fs: &str, total: u64, free: u64, mounted: bool) -> Device {
        Device {
            node: "sdb1".into(),
            disk: "sdb".into(),
            label: None,
            fs: Some(fs.into()),
            mount: mounted.then(|| PathBuf::from("/run/media/me/STICK")),
            total,
            free,
            kind: Kind::Removable,
        }
    }

    fn load(bytes: u64, largest: u64) -> Load {
        Load {
            bytes,
            exact: true,
            largest,
            carry: Carry::Copy,
            source: "/home/me/photos".into(),
        }
    }

    #[test]
    fn a_load_that_leaves_plenty_fits_and_says_how_much_is_left() {
        let verdict = verdict(
            &device("ext4", 32 * GIB, 20 * GIB, true),
            &load(5 * GIB, GIB),
            false,
        );
        assert_eq!(verdict, Verdict::Fits { left: 15 * GIB });
        assert!(verdict.allows());
    }

    #[test]
    fn a_load_bigger_than_the_free_space_is_short_by_exactly_the_difference() {
        let verdict = verdict(
            &device("ext4", 32 * GIB, 3 * GIB, true),
            &load(7 * GIB, GIB),
            false,
        );
        assert_eq!(verdict, Verdict::Short { need: 4 * GIB });
        assert!(!verdict.allows());
    }

    #[test]
    fn a_load_that_just_fits_is_tight_and_still_allowed() {
        let free = 10 * GIB;
        for bytes in [free, free - MIB, free - 15 * MIB] {
            let v = verdict(
                &device("ext4", 32 * GIB, free, true),
                &load(bytes, MIB),
                false,
            );
            assert!(matches!(v, Verdict::Tight { .. }), "{bytes}: {v:?}");
            assert!(v.allows());
        }
        let roomy = verdict(
            &device("ext4", 32 * GIB, free, true),
            &load(free - 17 * MIB - free / 50, MIB),
            false,
        );
        assert!(matches!(roomy, Verdict::Fits { .. }), "{roomy:?}");
    }

    #[test]
    fn a_file_over_the_fat32_limit_is_refused_however_much_room_there_is() {
        let v = verdict(
            &device("vfat", 64 * GIB, 60 * GIB, true),
            &load(6 * GIB, 5 * GIB),
            false,
        );
        assert_eq!(
            v,
            Verdict::TooBig {
                limit: 4 * GIB - 1,
                fs: "vfat".into()
            }
        );
        assert!(!v.allows());
        let just_under = verdict(
            &device("vfat", 64 * GIB, 60 * GIB, true),
            &load(6 * GIB, 4 * GIB - 1),
            false,
        );
        assert!(just_under.allows(), "exactly the limit is allowed");
        let exfat = verdict(
            &device("exfat", 64 * GIB, 60 * GIB, true),
            &load(6 * GIB, 5 * GIB),
            false,
        );
        assert!(exfat.allows(), "exFAT has no such limit");
    }

    #[test]
    fn a_move_within_one_disk_needs_no_room() {
        let mut moved = load(50 * GIB, GIB);
        moved.carry = Carry::Move;
        let v = verdict(&device("ext4", 100 * GIB, MIB, true), &moved, true);
        assert_eq!(v, Verdict::SameDisk);
        assert!(v.allows());
        let across = verdict(&device("ext4", 100 * GIB, MIB, true), &moved, false);
        assert!(
            matches!(across, Verdict::Short { .. }),
            "to another disk it is a copy then a delete"
        );
        let copied = verdict(
            &device("ext4", 100 * GIB, MIB, true),
            &load(50 * GIB, GIB),
            true,
        );
        assert!(
            matches!(copied, Verdict::Short { .. }),
            "a copy within one disk still needs room"
        );
    }

    #[test]
    fn an_unmounted_device_and_an_unknown_size_cannot_be_judged() {
        let unmounted = verdict(&device("ext4", 32 * GIB, 0, false), &load(MIB, MIB), false);
        assert_eq!(unmounted, Verdict::Unmounted);
        assert!(!unmounted.allows(), "it has to be mounted first");
        assert_eq!(
            verdict(&device("ext4", 0, 0, true), &load(MIB, MIB), false),
            Verdict::Unsure
        );
    }

    #[test]
    fn a_load_too_big_to_measure_is_short_if_what_was_measured_is_already_too_much_and_unsure_otherwise()
     {
        let mut partial = load(7 * GIB, GIB);
        partial.exact = false;
        assert!(matches!(
            verdict(&device("ext4", 32 * GIB, 3 * GIB, true), &partial, false),
            Verdict::Short { .. }
        ));
        assert_eq!(
            verdict(&device("ext4", 32 * GIB, 30 * GIB, true), &partial, false),
            Verdict::Unsure
        );
    }

    #[test]
    fn every_verdict_has_a_short_line_for_a_card_and_the_refusals_say_why() {
        let lines: Vec<String> = [
            Verdict::Fits { left: 3 * GIB },
            Verdict::Tight { left: MIB },
            Verdict::Short { need: 4 * GIB },
            Verdict::TooBig {
                limit: 4 * GIB - 1,
                fs: "vfat".into(),
            },
            Verdict::SameDisk,
            Verdict::Unmounted,
            Verdict::Unsure,
        ]
        .iter()
        .map(|v| card_text(v, false))
        .collect();
        assert_eq!(lines[0], "✓ fits · 3.0G left");
        assert_eq!(lines[2], "✗ needs 4.0G more");
        assert_eq!(lines[3], "✗ a file is over 4.0G");
        assert!(lines.iter().all(|l| l.chars().count() <= 24), "{lines:?}");
        for verdict in [
            Verdict::Fits { left: 1 },
            Verdict::Tight { left: 1 },
            Verdict::Short { need: 1 },
            Verdict::SameDisk,
            Verdict::Unmounted,
            Verdict::Unsure,
        ] {
            assert!(card_text(&verdict, true).is_ascii(), "{verdict:?}");
        }
        assert_eq!(
            refusal(&Verdict::Short { need: 4 * GIB }, "STICK"),
            "STICK: needs 4.0G more room"
        );
        assert!(
            refusal(
                &Verdict::TooBig {
                    limit: 4 * GIB - 1,
                    fs: "vfat".into()
                },
                "STICK"
            )
            .contains("VFAT")
        );
        assert!(refusal(&Verdict::Unmounted, "STICK").contains("press m"));
    }

    #[test]
    fn only_the_fat_family_has_a_file_size_limit_worth_warning_about() {
        for fs in ["vfat", "fat", "fat32", "msdos"] {
            assert_eq!(max_file_size(fs), Some(4 * GIB - 1), "{fs}");
        }
        for fs in ["ext4", "btrfs", "xfs", "exfat", "ntfs", "ntfs3", "f2fs"] {
            assert_eq!(max_file_size(fs), None, "{fs}");
        }
    }
}
