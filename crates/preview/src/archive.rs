//! Listing the contents of `zip`, `tar` and `tar.gz` archives for the preview pane.
//!
//! An archive is untrusted input that gets opened merely by moving the cursor over it, so every
//! read here is bounded: at most [`Limits::max_entries`] entries are kept, a `.tar.gz` is
//! inflated for at most [`Limits::max_inflated`] bytes, a plain `.tar` is skipped through by
//! seeking instead of reading, and a zip whose central directory is implausibly large is refused
//! before the `zip` crate ever allocates for it. Nothing is extracted, so nothing here can write
//! outside memory. Entry names are attacker-chosen text headed for a terminal, so they are
//! cleaned of control and direction-changing characters ([`clean_name`]) before anyone sees them.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use flate2::read::GzDecoder;

/// The archive formats that can be listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Zip,
    Tar,
    TarGz,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Zip => "zip",
            Kind::Tar => "tar",
            Kind::TarGz => "tar.gz",
        }
    }
}

/// Which format `path` is, going by its name alone (`.zip`, `.jar`, `.tar`, `.tar.gz`, `.tgz`).
/// Like `is_image` and `is_text`, this does not sniff content: a mislabelled file just fails to
/// list and is shown as bytes instead.
pub fn kind_of(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_str()?.to_lowercase();
    if name.ends_with(".zip") || name.ends_with(".jar") {
        Some(Kind::Zip)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Some(Kind::TarGz)
    } else if name.ends_with(".tar") {
        Some(Kind::Tar)
    } else {
        None
    }
}

pub fn is_archive(path: &Path) -> bool {
    kind_of(path).is_some()
}

/// How much work listing one archive may do.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Entries kept; the rest are reported in [`Listing::hidden`].
    pub max_entries: usize,
    /// Bytes a `.tar.gz` is inflated for before the listing stops where it got to.
    pub max_inflated: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 5_000,
            max_inflated: 256 << 20,
        }
    }
}

/// A zip whose central directory is bigger than this is not listed: the `zip` crate reads the
/// whole directory into memory when it opens a file, so this bounds what a hostile zip can make
/// it allocate.
const MAX_ZIP_DIRECTORY: u32 = 8 << 20;

/// The longest name kept, in characters. A tar can carry a name of any length in its extended
/// headers.
const MAX_NAME_CHARS: usize = 1024;

/// One thing in an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Cleaned by [`clean_name`], so safe to draw.
    pub name: String,
    /// Unpacked size in bytes; zero for a directory.
    pub size: u64,
    pub is_dir: bool,
}

/// What was not listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hidden {
    /// Everything was listed.
    Nothing,
    /// This many more entries exist (a zip knows its own count).
    Count(usize),
    /// There are more, but how many is not known (a tar has no index, so counting means reading
    /// it all, or reading was cut short by an error or a limit).
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub kind: Kind,
    pub entries: Vec<Entry>,
    pub hidden: Hidden,
}

impl Listing {
    /// What the listed files add up to unpacked.
    pub fn unpacked_bytes(&self) -> u64 {
        self.entries
            .iter()
            .fold(0u64, |sum, entry| sum.saturating_add(entry.size))
    }
}

/// A name made safe to put on a terminal: control characters (an escape sequence in a file name
/// could otherwise repaint or retitle the screen) and the invisible characters that reorder text
/// (a right-to-left override, U+202E, makes `evil` + U+202E + `txt.exe` display as `evilexe.txt`)
/// become `?`, and an absurdly long name is cut with `…`.
pub fn clean_name(raw: &str) -> String {
    let mut chars = raw.chars();
    let mut name: String = chars
        .by_ref()
        .take(MAX_NAME_CHARS)
        .map(|c| if is_unsafe(c) { '?' } else { c })
        .collect();
    if chars.next().is_some() {
        name.push('…');
    }
    name
}

fn is_unsafe(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}

/// Lists `path` within the default [`Limits`]. `None` if it is not an archive of a known kind,
/// or is damaged from its very first entry.
pub fn list(path: &Path) -> Option<Listing> {
    list_within(path, Limits::default())
}

pub fn list_within(path: &Path, limits: Limits) -> Option<Listing> {
    match kind_of(path)? {
        Kind::Zip => list_zip(path, limits),
        Kind::Tar => {
            let mut archive = tar::Archive::new(BufReader::new(File::open(path).ok()?));
            // Seeking past each entry's data makes listing cost the headers, not the file's size.
            collect_tar(Kind::Tar, archive.entries_with_seek().ok()?, limits)
        }
        Kind::TarGz => {
            let file = BufReader::new(File::open(path).ok()?);
            // A gzip stream cannot seek, so each entry's data is inflated and thrown away; the
            // cap bounds how much of that a small file full of zeros can make us do.
            let inflated = Capped::new(GzDecoder::new(file), limits.max_inflated);
            let mut archive = tar::Archive::new(inflated);
            collect_tar(Kind::TarGz, archive.entries().ok()?, limits)
        }
    }
}

/// A reader that fails, rather than reporting end of file, once `left` bytes have been read from
/// `inner` and more are still coming. A plain `Read::take` would end quietly, and a tar reader
/// that finds the stream ended takes it for the archive's real end, so a cut-off listing would
/// pass for a complete one.
struct Capped<R> {
    inner: R,
    left: u64,
}

impl<R> Capped<R> {
    fn new(inner: R, left: u64) -> Self {
        Self { inner, left }
    }
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            // Exactly at the cap is fine if the stream ends there; only more data is over it.
            return match self.inner.read(&mut [0u8; 1])? {
                0 => Ok(0),
                _ => Err(std::io::Error::other(
                    "archive is bigger than the listing limit",
                )),
            };
        }
        let room = usize::try_from(self.left).map_or(buf.len(), |left| buf.len().min(left));
        let read = self.inner.read(&mut buf[..room])?;
        self.left -= read as u64;
        Ok(read)
    }
}

/// Whether the zip whose last bytes are `tail` claims a central directory small enough to open.
/// Zip64 (marked by all-ones fields, used past 65,535 entries or 4 GiB) is turned away too:
/// reading those means following a second record, and such archives are rare next to how easy it
/// is to make one that lies.
///
/// Every place in `tail` that looks like an end-of-central-directory record is checked, and one
/// claiming too much is enough to refuse the file. A reader may skip a record that turns out to be
/// bogus and use an earlier one, so checking only the record nearest the end would let a file
/// through by putting a small fake there. At least one record must also end exactly at the end of
/// the file, which is what a real one does.
fn directory_is_small(tail: &[u8]) -> bool {
    const SIGNATURE: &[u8] = b"PK\x05\x06";
    const RECORD: usize = 22;
    let mut ends_at_the_end = false;
    for at in 0..tail.len().saturating_sub(RECORD - 1) {
        if !tail[at..].starts_with(SIGNATURE) {
            continue;
        }
        let entries = u16::from_le_bytes([tail[at + 10], tail[at + 11]]);
        let size = u32::from_le_bytes([tail[at + 12], tail[at + 13], tail[at + 14], tail[at + 15]]);
        if entries == u16::MAX || size == u32::MAX || size > MAX_ZIP_DIRECTORY {
            return false;
        }
        let comment = u16::from_le_bytes([tail[at + 20], tail[at + 21]]) as usize;
        ends_at_the_end |= at + RECORD + comment == tail.len();
    }
    ends_at_the_end
}

fn list_zip(path: &Path, limits: Limits) -> Option<Listing> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    // 22 bytes of record plus a comment of at most 65,535.
    let tail_len = len.min(22 + 65_535);
    file.seek(SeekFrom::Start(len - tail_len)).ok()?;
    let mut tail = Vec::new();
    (&mut file).take(tail_len).read_to_end(&mut tail).ok()?;
    if !directory_is_small(&tail) {
        return None;
    }

    let mut archive = zip::ZipArchive::new(BufReader::new(file)).ok()?;
    let total = archive.len();
    let mut entries = Vec::with_capacity(total.min(limits.max_entries));
    for index in 0..total.min(limits.max_entries) {
        // The raw entry: only its header is read, never its data, so nothing is decompressed.
        let Ok(entry) = archive.by_index_raw(index) else {
            return (!entries.is_empty()).then_some(Listing {
                kind: Kind::Zip,
                entries,
                hidden: Hidden::Unknown,
            });
        };
        entries.push(Entry {
            name: clean_name(entry.name()),
            size: if entry.is_dir() { 0 } else { entry.size() },
            is_dir: entry.is_dir(),
        });
    }
    let hidden = match total.saturating_sub(entries.len()) {
        0 => Hidden::Nothing,
        more => Hidden::Count(more),
    };
    Some(Listing {
        kind: Kind::Zip,
        entries,
        hidden,
    })
}

fn collect_tar<'a, R: Read + 'a>(
    kind: Kind,
    iter: impl Iterator<Item = std::io::Result<tar::Entry<'a, R>>>,
    limits: Limits,
) -> Option<Listing> {
    let mut entries = Vec::new();
    let mut hidden = Hidden::Nothing;
    for item in iter {
        if entries.len() >= limits.max_entries {
            hidden = Hidden::Unknown;
            break;
        }
        let Ok(entry) = item else {
            // A damaged first entry means this is not a tar at all; later damage keeps what was
            // read so far.
            if entries.is_empty() {
                return None;
            }
            hidden = Hidden::Unknown;
            break;
        };
        let entry_type = entry.header().entry_type();
        if entry_type.is_pax_global_extensions() {
            continue;
        }
        let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let is_dir = entry_type.is_dir() || name.ends_with('/');
        entries.push(Entry {
            name: clean_name(&name),
            size: if is_dir { 0 } else { entry.size() },
            is_dir,
        });
    }
    Some(Listing {
        kind,
        entries,
        hidden,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::io::{Cursor, Write};

    fn scratch(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("minuteman-archive-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `(name, size)` pairs as a tar's bytes; a name ending in `/` is a directory.
    fn tar_bytes(items: &[(String, usize)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, size) in items {
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o644);
            if name.ends_with('/') {
                header.set_entry_type(tar::EntryType::Directory);
                header.set_size(0);
            } else {
                header.set_size(*size as u64);
            }
            header.set_cksum();
            builder
                .append_data(&mut header, name, vec![b'x'; *size].as_slice())
                .unwrap();
        }
        builder.into_inner().unwrap()
    }

    fn zip_bytes(items: &[(String, usize)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, size) in items {
            if name.ends_with('/') {
                writer.add_directory(name.as_str(), options).unwrap();
            } else {
                writer.start_file(name.as_str(), options).unwrap();
                writer.write_all(&vec![b'x'; *size]).unwrap();
            }
        }
        writer.finish().unwrap().into_inner()
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn names_and_sizes(listing: &Listing) -> Vec<(String, u64)> {
        listing
            .entries
            .iter()
            .map(|e| (e.name.clone(), e.size))
            .collect()
    }

    fn numbered(count: usize) -> Vec<(String, usize)> {
        (0..count).map(|i| (format!("f{i:04}"), 3)).collect()
    }

    #[test]
    fn archives_are_recognised_by_name_case_insensitively() {
        assert_eq!(kind_of(Path::new("a.ZIP")), Some(Kind::Zip));
        assert_eq!(kind_of(Path::new("a.jar")), Some(Kind::Zip));
        assert_eq!(kind_of(Path::new("a.tar")), Some(Kind::Tar));
        assert_eq!(kind_of(Path::new("a.tar.gz")), Some(Kind::TarGz));
        assert_eq!(kind_of(Path::new("a.TGZ")), Some(Kind::TarGz));
        for other in ["a.gz", "a.txt", "zip", "a.tar.xz", "a.7z", "tar"] {
            assert_eq!(kind_of(Path::new(other)), None, "{other}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]

        /// Whatever a zip, a tar or a tar.gz holds comes back in the same order with the same
        /// sizes, and a directory is flagged as one.
        #[test]
        fn every_format_lists_what_was_put_in_it(
            files in proptest::collection::vec(("[a-z]{1,8}(/[a-z]{1,8}){0,2}", 0usize..600), 0..12),
            dir_name in "[a-z]{1,6}",
        ) {
            let mut items = files.clone();
            items.push((format!("{dir_name}/"), 0));
            let expected: Vec<(String, u64)> =
                items.iter().map(|(n, s)| (n.clone(), *s as u64)).collect();
            let dir = scratch("roundtrip");

            let tar = tar_bytes(&items);
            let zip = zip_bytes(&items);
            let cases = [
                ("a.tar", tar.clone(), Kind::Tar),
                ("a.tar.gz", gzip(&tar), Kind::TarGz),
                ("a.zip", zip, Kind::Zip),
            ];
            for (name, bytes, kind) in cases {
                let listing = list(&write(&dir, name, &bytes)).expect(name);
                prop_assert_eq!(listing.kind, kind);
                prop_assert_eq!(listing.hidden, Hidden::Nothing);
                prop_assert_eq!(names_and_sizes(&listing), expected.clone(), "{}", name);
                let last = listing.entries.last().unwrap();
                prop_assert!(last.is_dir, "{} lost the directory flag", name);
            }
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn the_entry_limit_cuts_the_listing_and_says_how_much_was_left_out() {
        let dir = scratch("limit");
        let limits = Limits {
            max_entries: 3,
            ..Limits::default()
        };
        let five = numbered(5);
        let tar = write(&dir, "five.tar", &tar_bytes(&five));
        let zip = write(&dir, "five.zip", &zip_bytes(&five));

        let tar_listing = list_within(&tar, limits).unwrap();
        assert_eq!(tar_listing.entries.len(), 3);
        assert_eq!(
            tar_listing.hidden,
            Hidden::Unknown,
            "a tar cannot count without reading it"
        );
        let zip_listing = list_within(&zip, limits).unwrap();
        assert_eq!(zip_listing.entries.len(), 3);
        assert_eq!(zip_listing.hidden, Hidden::Count(2));

        // Exactly at the limit is not "more".
        let three = numbered(3);
        let exact_tar = write(&dir, "three.tar", &tar_bytes(&three));
        let exact_zip = write(&dir, "three.zip", &zip_bytes(&three));
        assert_eq!(
            list_within(&exact_tar, limits).unwrap().hidden,
            Hidden::Nothing
        );
        assert_eq!(
            list_within(&exact_zip, limits).unwrap().hidden,
            Hidden::Nothing
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_tar_gz_stops_inflating_at_its_cap_and_says_the_listing_is_cut() {
        let dir = scratch("inflate");
        // A megabyte of one byte compresses to almost nothing, which is the point of the cap.
        let items = vec![("big".to_string(), 1 << 20), ("after".to_string(), 10)];
        let path = write(&dir, "bomb.tar.gz", &gzip(&tar_bytes(&items)));
        assert!(std::fs::metadata(&path).unwrap().len() < 10_000);

        let capped = Limits {
            max_inflated: 64 * 1024,
            ..Limits::default()
        };
        let listing = list_within(&path, capped).unwrap();
        assert_eq!(
            names_and_sizes(&listing),
            vec![("big".to_string(), 1 << 20)]
        );
        assert_eq!(
            listing.hidden,
            Hidden::Unknown,
            "a cut listing must not pass for a whole one"
        );

        let whole = list(&path).unwrap();
        assert_eq!(whole.entries.len(), 2);
        assert_eq!(whole.hidden, Hidden::Nothing);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_plain_tar_is_listed_without_reading_its_data() {
        let dir = scratch("seek");
        // A 200 MiB sparse file: reading it would take real time and memory, seeking over it none.
        let path = dir.join("huge.tar");
        let mut builder = tar::Builder::new(File::create(&path).unwrap());
        let mut header = tar::Header::new_gnu();
        header.set_size(200 << 20);
        header.set_mode(0o644);
        header.set_cksum();
        // Writes the header only; the body is a hole the filesystem never stores.
        builder
            .append_data(&mut header, "huge", std::io::empty())
            .ok();
        drop(builder);
        let file = File::options().write(true).open(&path).unwrap();
        file.set_len(512 + (200 << 20) + 1024).unwrap();

        let started = std::time::Instant::now();
        let listing = list(&path).unwrap();
        assert_eq!(
            names_and_sizes(&listing),
            vec![("huge".to_string(), 200 << 20)]
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn files_that_are_not_archives_do_not_list() {
        let dir = scratch("junk");
        for name in [
            "junk.zip",
            "junk.tar",
            "junk.tar.gz",
            "empty.zip",
            "empty.tgz",
        ] {
            let bytes: &[u8] = if name.starts_with("junk") {
                b"this is not an archive"
            } else {
                b""
            };
            assert!(list(&write(&dir, name, bytes)).is_none(), "{name}");
        }
        assert!(list(&dir.join("missing.zip")).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_tar_cut_off_partway_keeps_what_was_readable() {
        let dir = scratch("cut");
        let mut bytes = tar_bytes(&[("one".to_string(), 600), ("two".to_string(), 600)]);
        bytes.truncate(512 + 1024 + 200); // into the second entry's header
        let listing = list(&write(&dir, "cut.tar", &bytes)).unwrap();
        assert_eq!(listing.entries[0].name, "one");
        assert_eq!(listing.hidden, Hidden::Unknown);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn eocd(entries: u16, size: u32, comment: &[u8]) -> Vec<u8> {
        let mut record = b"PK\x05\x06".to_vec();
        record.extend([0, 0, 0, 0]);
        record.extend(entries.to_le_bytes());
        record.extend(entries.to_le_bytes());
        record.extend(size.to_le_bytes());
        record.extend([0, 0, 0, 0]);
        record.extend((comment.len() as u16).to_le_bytes());
        record.extend(comment);
        record
    }

    #[test]
    fn a_zip_that_claims_a_huge_directory_is_refused_before_it_is_opened() {
        assert!(directory_is_small(&eocd(10, 1000, b"")));
        assert!(directory_is_small(&eocd(
            65_534,
            MAX_ZIP_DIRECTORY,
            b"a comment"
        )));
        assert!(!directory_is_small(&eocd(10, MAX_ZIP_DIRECTORY + 1, b"")));
        assert!(
            !directory_is_small(&eocd(u16::MAX, 1000, b"")),
            "zip64 entry count"
        );
        assert!(
            !directory_is_small(&eocd(10, u32::MAX, b"")),
            "zip64 directory size"
        );
        assert!(!directory_is_small(b""));
        assert!(!directory_is_small(b"PK\x05\x06 too short"));
    }

    #[test]
    fn a_small_fake_record_cannot_hide_a_record_that_claims_too_much() {
        // A reader that finds the fake at the very end unusable falls back to the real one before
        // it, so one greedy record anywhere in the tail is enough to refuse the file.
        let fake = eocd(1, 10, b"");
        let real_but_huge = eocd(1, MAX_ZIP_DIRECTORY + 1, &fake);
        assert!(!directory_is_small(&real_but_huge));
        let mut trailing_junk = eocd(1, MAX_ZIP_DIRECTORY + 1, b"");
        trailing_junk.extend(&fake);
        assert!(!directory_is_small(&trailing_junk));
        // A record that does not end where the file does is not a real one on its own.
        assert!(!directory_is_small(
            &[eocd(1, 10, b""), b"trailing".to_vec()].concat()
        ));
    }

    proptest! {
        #[test]
        fn the_directory_check_never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..200)) {
            let _ = directory_is_small(&bytes);
        }

        #[test]
        fn the_directory_check_follows_the_records_own_fields(
            entries in any::<u16>(),
            size in any::<u32>(),
            junk in proptest::collection::vec(any::<u8>(), 0..40),
            comment in proptest::collection::vec(any::<u8>(), 0..30),
        ) {
            let mut tail = junk;
            tail.extend(eocd(entries, size, &comment));
            let plausible = entries != u16::MAX && size != u32::MAX && size <= MAX_ZIP_DIRECTORY;
            // Random bytes that happen to spell the signature are a different test's business.
            let spells_it = |bytes: &[u8]| bytes.windows(4).any(|w| w == b"PK\x05\x06");
            if !spells_it(&comment) && !spells_it(&tail[..tail.len() - 22 - comment.len()]) {
                prop_assert_eq!(directory_is_small(&tail), plausible);
            }
        }

        /// Whatever an archive calls a file, what reaches the screen has no control or
        /// direction-changing character in it and is bounded in length.
        #[test]
        fn cleaned_names_are_safe_and_bounded(raw in any::<String>()) {
            let name = clean_name(&raw);
            prop_assert!(name.chars().all(|c| !is_unsafe(c)));
            prop_assert!(name.chars().count() <= MAX_NAME_CHARS + 1);
        }
    }

    #[test]
    fn escape_sequences_and_direction_overrides_in_names_are_defused() {
        assert_eq!(clean_name("\x1b]0;pwned\x07.txt"), "?]0;pwned?.txt");
        assert_eq!(clean_name("evil\u{202e}txt.exe"), "evil?txt.exe");
        assert_eq!(clean_name("plain-name.rs"), "plain-name.rs");
        assert_eq!(clean_name("naïve/日本語.txt"), "naïve/日本語.txt");
        let long = "a".repeat(MAX_NAME_CHARS + 50);
        assert!(clean_name(&long).ends_with('…'));
    }

    #[test]
    fn a_hostile_name_in_a_real_archive_arrives_cleaned() {
        let dir = scratch("hostile");
        let items = vec![("ok\x1b[2Jname".to_string(), 1)];
        let listing = list(&write(&dir, "h.tar", &tar_bytes(&items))).unwrap();
        assert_eq!(listing.entries[0].name, "ok?[2Jname");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_unpacked_size_adds_up_the_listed_files() {
        let listing = Listing {
            kind: Kind::Tar,
            entries: vec![
                Entry {
                    name: "a".into(),
                    size: 5,
                    is_dir: false,
                },
                Entry {
                    name: "d/".into(),
                    size: 0,
                    is_dir: true,
                },
                Entry {
                    name: "b".into(),
                    size: u64::MAX,
                    is_dir: false,
                },
            ],
            hidden: Hidden::Nothing,
        };
        assert_eq!(
            listing.unpacked_bytes(),
            u64::MAX,
            "saturates instead of wrapping"
        );
    }
}
