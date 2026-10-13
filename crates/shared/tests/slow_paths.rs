//! `is_slow` tells this machine's disk from a path that needs a round trip to read.

use std::path::{Path, PathBuf};

use proptest::prelude::*;
use shared::is_slow;

#[test]
fn a_plain_local_path_is_fast() {
    assert!(!is_slow(Path::new("/home/user/notes")));
}

#[test]
fn an_ssh_path_is_slow() {
    assert!(is_slow(Path::new("ssh://host/home/user")));
}

#[test]
fn an_archive_root_and_anything_in_it_is_slow() {
    assert!(is_slow(Path::new("/tmp/a.zip!")));
    assert!(is_slow(Path::new("/tmp/a.tar.gz!/dir/file")));
}

#[test]
fn the_archive_file_itself_is_a_file_on_disk_and_fast() {
    assert!(!is_slow(Path::new("/tmp/a.zip")));
}

proptest! {
    #[test]
    fn anything_below_an_archive_root_is_slow(rest in "[a-z]{1,8}(/[a-z]{1,8}){0,3}") {
        let path = PathBuf::from(format!("/tmp/a.zip!/{rest}"));
        prop_assert!(is_slow(&path));
    }

    #[test]
    fn a_path_with_no_scheme_and_no_mount_mark_is_fast(rest in "/[a-z]{1,8}(/[a-z]{1,8}){0,4}") {
        prop_assert!(!is_slow(Path::new(&rest)));
    }
}
