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

//! One request, start to end: the jobs of an `ArchiveWork` in order on the local disk, stopping at
//! the first failure, with the password checks an extract makes before it writes anything. It never
//! reads or writes a stream; progress goes out through a callback and the ending comes back as a
//! value, so a whole request can be played in a test.

use std::ops::ControlFlow;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use plugin_api::{ArchiveStatus, ArchiveWork, ExtractJob, Password};
use shared::{LocalVfs, VfsError};

use crate::archive::{
    CompressOptions, ExtractLimits, check_password, compress_with_options, extract_with_password,
    needs_password,
};
use crate::error::{ArchiveError, Outcome};

/// How a request ended, and which of its jobs wrote something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ended {
    pub finished: Vec<usize>,
    pub status: ArchiveStatus,
}

impl Ended {
    fn of(finished: Vec<usize>, status: ArchiveStatus) -> Self {
        Self { finished, status }
    }
}

/// The words for the status line when `error` stopped a job. A name that is taken is said as what
/// is in the way, since that is what the user has to move.
fn describe(error: &ArchiveError) -> String {
    match error {
        ArchiveError::Vfs(VfsError::AlreadyExists(path)) => {
            format!("{} already exists", path.display())
        }
        other => other.to_string(),
    }
}

/// What ends a request early because of `error`.
fn stopped_by(error: &ArchiveError) -> ArchiveStatus {
    match error {
        ArchiveError::Cancelled => ArchiveStatus::Cancelled,
        other => ArchiveStatus::Failed {
            message: describe(other),
        },
    }
}

/// Runs `work`. `on_entry` is called with each entry reached, and the request stops with
/// `Cancelled` at the first entry reached after `cancel` is set.
pub fn execute(work: &ArchiveWork, cancel: &AtomicBool, on_entry: &mut dyn FnMut(&Path)) -> Ended {
    let mut on_progress = |path: &Path| -> ControlFlow<()> {
        on_entry(path);
        match cancel.load(Ordering::Relaxed) {
            true => ControlFlow::Break(()),
            false => ControlFlow::Continue(()),
        }
    };
    match work {
        ArchiveWork::Compress {
            jobs,
            format,
            level,
            policy,
            password,
        } => {
            let options = CompressOptions {
                format: *format,
                level: *level,
                password: password.as_ref(),
                policy: *policy,
            };
            let mut finished = Vec::new();
            for (index, job) in jobs.iter().enumerate() {
                match compress_with_options(
                    &LocalVfs,
                    &job.sources,
                    &job.out,
                    &options,
                    &mut on_progress,
                ) {
                    Ok(Outcome::Completed) => finished.push(index),
                    // Nothing was written, so the originals must not be trashed either.
                    Ok(Outcome::Skipped) => {}
                    Err(e) => return Ended::of(finished, stopped_by(&e)),
                }
            }
            Ended::of(finished, ArchiveStatus::Completed)
        }
        ArchiveWork::Extract {
            jobs,
            policy,
            password,
        } => {
            if let Some(stopped) = check_passwords(jobs, password.as_ref()) {
                return Ended::of(Vec::new(), stopped);
            }
            let mut finished = Vec::new();
            for (index, job) in jobs.iter().enumerate() {
                match extract_with_password(
                    &LocalVfs,
                    &job.archive,
                    &job.dest,
                    *policy,
                    ExtractLimits::default(),
                    password.as_ref(),
                    &mut on_progress,
                ) {
                    Ok(_) => finished.push(index),
                    Err(e) => return Ended::of(finished, stopped_by(&e)),
                }
            }
            Ended::of(finished, ArchiveStatus::Completed)
        }
    }
}

/// What to say before an extract writes anything, if its passwords are not in order. With none
/// typed, the first encrypted archive asks for one (an archive that cannot even be opened is left
/// for the extraction to report). With one typed, every encrypted archive of the batch must accept
/// it, so a wrong password costs nothing.
fn check_passwords(jobs: &[ExtractJob], password: Option<&Password>) -> Option<ArchiveStatus> {
    match password {
        None => jobs
            .iter()
            .find(|job| needs_password(&LocalVfs, &job.archive).unwrap_or(false))
            .map(|job| ArchiveStatus::PasswordNeeded {
                archive: job.archive.clone(),
            }),
        Some(password) => {
            jobs.iter().find_map(
                |job| match check_password(&LocalVfs, &job.archive, password) {
                    Ok(()) => None,
                    Err(ArchiveError::WrongPassword) => Some(ArchiveStatus::WrongPassword {
                        archive: job.archive.clone(),
                    }),
                    Err(e) => Some(stopped_by(&e)),
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;

    use plugin_api::{CompressJob, Conflict, Format, Level};
    use proptest::prelude::*;

    use super::*;

    fn scratch(label: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "minuteman-archive-work-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn secret(text: &str) -> Password {
        let mut password = Password::new();
        text.chars().for_each(|c| password.push(c));
        password
    }

    fn compress(
        jobs: Vec<CompressJob>,
        format: Format,
        policy: Conflict,
        password: Option<Password>,
    ) -> ArchiveWork {
        ArchiveWork::Compress {
            jobs,
            format,
            level: Level::default(),
            policy,
            password,
        }
    }

    fn extract(jobs: Vec<ExtractJob>, policy: Conflict, password: Option<Password>) -> ArchiveWork {
        ArchiveWork::Extract {
            jobs,
            policy,
            password,
        }
    }

    fn run(work: &ArchiveWork) -> Ended {
        execute(work, &AtomicBool::new(false), &mut |_| {})
    }

    #[test]
    fn a_compress_then_an_extract_round_trips_a_folder() {
        let dir = scratch("round-trip");
        fs::create_dir_all(dir.join("src/inner")).unwrap();
        fs::write(dir.join("src/a.txt"), "alpha").unwrap();
        fs::write(dir.join("src/inner/b.txt"), "beta").unwrap();
        let out = dir.join("src.tar.gz");

        let packed = run(&compress(
            vec![CompressJob {
                sources: vec![dir.join("src")],
                out: out.clone(),
            }],
            Format::TarGz,
            Conflict::Abort,
            None,
        ));
        assert_eq!(packed, Ended::of(vec![0], ArchiveStatus::Completed));

        let dest = dir.join("unpacked");
        let unpacked = run(&extract(
            vec![ExtractJob {
                archive: out,
                dest: dest.clone(),
            }],
            Conflict::Abort,
            None,
        ));
        assert_eq!(unpacked, Ended::of(vec![0], ArchiveStatus::Completed));
        assert_eq!(fs::read_to_string(dest.join("src/a.txt")).unwrap(), "alpha");
        assert_eq!(
            fs::read_to_string(dest.join("src/inner/b.txt")).unwrap(),
            "beta"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_job_left_alone_by_skip_is_not_finished_so_its_originals_are_kept() {
        let dir = scratch("skip");
        fs::write(dir.join("a.txt"), "a").unwrap();
        fs::write(dir.join("b.txt"), "b").unwrap();
        fs::write(dir.join("taken.zip"), "not really a zip").unwrap();
        let jobs = vec![
            CompressJob {
                sources: vec![dir.join("a.txt")],
                out: dir.join("taken.zip"),
            },
            CompressJob {
                sources: vec![dir.join("b.txt")],
                out: dir.join("fresh.zip"),
            },
        ];

        let ended = run(&compress(jobs, Format::Zip, Conflict::Skip, None));

        assert_eq!(ended, Ended::of(vec![1], ArchiveStatus::Completed));
        assert_eq!(
            fs::read_to_string(dir.join("taken.zip")).unwrap(),
            "not really a zip",
            "the archive that was in the way is untouched"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_name_that_is_taken_fails_the_job_saying_what_is_in_the_way() {
        let dir = scratch("taken");
        fs::write(dir.join("a.txt"), "a").unwrap();
        fs::write(dir.join("out.zip"), "x").unwrap();
        let work = compress(
            vec![CompressJob {
                sources: vec![dir.join("a.txt")],
                out: dir.join("out.zip"),
            }],
            Format::Zip,
            Conflict::Abort,
            None,
        );

        let ended = run(&work);

        let ArchiveStatus::Failed { message } = ended.status else {
            panic!("expected a failure, got {ended:?}");
        };
        assert_eq!(
            message,
            format!("{} already exists", dir.join("out.zip").display())
        );
        assert!(ended.finished.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failure_after_a_finished_job_still_lists_the_finished_one() {
        let dir = scratch("partial");
        fs::write(dir.join("a.txt"), "a").unwrap();
        let jobs = vec![
            CompressJob {
                sources: vec![dir.join("a.txt")],
                out: dir.join("one.zip"),
            },
            CompressJob {
                sources: vec![dir.join("missing.txt")],
                out: dir.join("two.zip"),
            },
        ];

        let ended = run(&compress(jobs, Format::Zip, Conflict::Abort, None));

        assert_eq!(ended.finished, [0]);
        assert!(
            matches!(ended.status, ArchiveStatus::Failed { .. }),
            "{ended:?}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_request_stops_at_the_first_entry_after_the_flag_is_raised() {
        let dir = scratch("cancel");
        fs::create_dir_all(dir.join("src")).unwrap();
        for n in 0..20 {
            fs::write(dir.join("src").join(format!("f{n}.txt")), "x").unwrap();
        }
        let work = compress(
            vec![CompressJob {
                sources: vec![dir.join("src")],
                out: dir.join("out.tar"),
            }],
            Format::Tar,
            Conflict::Abort,
            None,
        );
        let cancel = AtomicBool::new(false);
        let mut seen = 0;

        let ended = execute(&work, &cancel, &mut |_| {
            seen += 1;
            if seen == 3 {
                cancel.store(true, Ordering::Relaxed);
            }
        });

        assert_eq!(ended.status, ArchiveStatus::Cancelled);
        assert!(ended.finished.is_empty());
        assert_eq!(seen, 3, "no entry is reached after the stop is asked for");
        assert!(
            !dir.join("out.tar").exists(),
            "a stopped archive is not left behind"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    fn sealed_zip(dir: &Path, name: &str, password: &str) -> PathBuf {
        fs::write(dir.join("secret.txt"), "classified").unwrap();
        let out = dir.join(name);
        let ended = run(&compress(
            vec![CompressJob {
                sources: vec![dir.join("secret.txt")],
                out: out.clone(),
            }],
            Format::Zip,
            Conflict::Abort,
            Some(secret(password)),
        ));
        assert_eq!(ended.status, ArchiveStatus::Completed);
        out
    }

    #[test]
    fn an_encrypted_archive_asks_for_a_password_before_anything_is_written() {
        let dir = scratch("needs");
        let zip = sealed_zip(&dir, "sealed.zip", "hunter2");
        let dest = dir.join("out");

        let ended = run(&extract(
            vec![ExtractJob {
                archive: zip.clone(),
                dest: dest.clone(),
            }],
            Conflict::Abort,
            None,
        ));

        assert_eq!(
            ended,
            Ended::of(Vec::new(), ArchiveStatus::PasswordNeeded { archive: zip })
        );
        assert!(!dest.exists(), "nothing was created");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_wrong_password_is_told_apart_and_writes_nothing_and_the_right_one_extracts() {
        let dir = scratch("password");
        let zip = sealed_zip(&dir, "sealed.zip", "hunter2");
        let dest = dir.join("out");
        let job = vec![ExtractJob {
            archive: zip.clone(),
            dest: dest.clone(),
        }];

        let wrong = run(&extract(job.clone(), Conflict::Abort, Some(secret("nope"))));
        assert_eq!(
            wrong,
            Ended::of(Vec::new(), ArchiveStatus::WrongPassword { archive: zip })
        );
        assert!(!dest.exists());

        let right = run(&extract(job, Conflict::Abort, Some(secret("hunter2"))));
        assert_eq!(right, Ended::of(vec![0], ArchiveStatus::Completed));
        assert_eq!(
            fs::read_to_string(dest.join("secret.txt")).unwrap(),
            "classified"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_plain_archive_never_asks_and_ignores_a_password() {
        let dir = scratch("plain");
        fs::write(dir.join("a.txt"), "a").unwrap();
        let zip = dir.join("plain.zip");
        run(&compress(
            vec![CompressJob {
                sources: vec![dir.join("a.txt")],
                out: zip.clone(),
            }],
            Format::Zip,
            Conflict::Abort,
            None,
        ));
        let job = |name: &str| {
            vec![ExtractJob {
                archive: zip.clone(),
                dest: dir.join(name),
            }]
        };

        assert_eq!(
            run(&extract(job("none"), Conflict::Abort, None)).status,
            ArchiveStatus::Completed
        );
        assert_eq!(
            run(&extract(
                job("some"),
                Conflict::Abort,
                Some(secret("anything"))
            ))
            .status,
            ArchiveStatus::Completed
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_archive_that_cannot_be_opened_is_reported_by_the_extraction_not_as_a_password_question() {
        let dir = scratch("missing");
        let work = extract(
            vec![ExtractJob {
                archive: dir.join("gone.zip"),
                dest: dir.join("out"),
            }],
            Conflict::Abort,
            None,
        );

        let ended = run(&work);

        assert!(
            matches!(ended.status, ArchiveStatus::Failed { .. }),
            "{ended:?}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(16))]

        /// Whatever the jobs are, a request that ends in anything but a failure or a stop lists
        /// exactly the jobs that wrote an archive, in order and without repeats.
        #[test]
        fn finished_jobs_are_listed_in_order_without_repeats(
            skip in proptest::collection::vec(any::<bool>(), 1..6),
        ) {
            let dir = scratch("finished");
            fs::write(dir.join("src.txt"), "x").unwrap();
            let jobs: Vec<CompressJob> = skip
                .iter()
                .enumerate()
                .map(|(n, taken)| {
                    let out = dir.join(format!("{n}.zip"));
                    if *taken {
                        fs::write(&out, "in the way").unwrap();
                    }
                    CompressJob { sources: vec![dir.join("src.txt")], out }
                })
                .collect();

            let ended = run(&compress(jobs, Format::Zip, Conflict::Skip, None));

            let expected: Vec<usize> = skip
                .iter()
                .enumerate()
                .filter_map(|(n, taken)| (!taken).then_some(n))
                .collect();
            prop_assert_eq!(ended, Ended::of(expected, ArchiveStatus::Completed));
            fs::remove_dir_all(&dir).unwrap();
        }
    }
}
