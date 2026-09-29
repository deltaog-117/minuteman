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

//! What kind of file a path is, read from the freedesktop shared MIME database the way every
//! desktop program reads it: `globs2` maps file names to types, `subclasses` says a Python script
//! is also plain text, and `aliases` says two names mean one type.
//!
//! Detection is by file name, with one content check for names that match nothing (text or not).
//! A machine without the database still gets a small built-in table, so "open this `.md`" never
//! depends on `shared-mime-info` being installed.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// How many leading bytes decide whether an unnamed file is text.
const SNIFF_BYTES: usize = 512;

/// Where the freedesktop databases live, in the order they override each other.
#[derive(Debug, Clone, Default)]
pub struct XdgDirs {
    pub data_home: PathBuf,
    pub data_dirs: Vec<PathBuf>,
    pub config_home: PathBuf,
    pub config_dirs: Vec<PathBuf>,
    /// `XDG_CURRENT_DESKTOP`, lower-cased, for desktop-specific `mimeapps.list` files.
    pub desktops: Vec<String>,
    /// Locale names to look up `Name[..]` under, most specific first (`pt_BR`, then `pt`).
    pub locales: Vec<String>,
}

impl XdgDirs {
    /// The directories the environment names, with the specification's defaults.
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let home = var("HOME").map(PathBuf::from).unwrap_or_default();
        let list = |name: &str, default: &str| -> Vec<PathBuf> {
            std::env::split_paths(&var(name).unwrap_or_else(|| default.into()))
                .filter(|p| p.is_absolute())
                .collect()
        };
        let lang = var("LC_ALL")
            .or_else(|| var("LC_MESSAGES"))
            .or_else(|| var("LANG"))
            .unwrap_or_default();
        Self {
            data_home: var("XDG_DATA_HOME")
                .map_or_else(|| home.join(".local/share"), PathBuf::from),
            data_dirs: list("XDG_DATA_DIRS", "/usr/local/share:/usr/share"),
            config_home: var("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from),
            config_dirs: list("XDG_CONFIG_DIRS", "/etc/xdg"),
            desktops: var("XDG_CURRENT_DESKTOP")
                .map(|d| d.split(':').map(str::to_lowercase).collect())
                .unwrap_or_default(),
            locales: locale_candidates(&lang),
        }
    }

    /// Every data directory, highest priority first.
    pub fn all_data_dirs(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.data_home.as_path()).chain(self.data_dirs.iter().map(PathBuf::as_path))
    }
}

/// `pt_BR.UTF-8@euro` → `["pt_BR", "pt"]`: the names a `Name[..]` key may carry, best first.
pub fn locale_candidates(lang: &str) -> Vec<String> {
    let base = lang.split(['.', '@']).next().unwrap_or("");
    if base.is_empty() || base == "C" || base == "POSIX" {
        return Vec::new();
    }
    let mut out = vec![base.to_string()];
    if let Some((language, _)) = base.split_once('_') {
        out.push(language.to_string());
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pattern {
    /// `*.ext`: the name ends with this (dot included).
    Suffix(String),
    /// No wildcard: the whole name.
    Exact(String),
    /// Anything else, with `*` and `?`.
    Wild(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Glob {
    mime: String,
    pattern: Pattern,
    /// Longer patterns are more specific: `*.tar.gz` beats `*.gz` at equal weight.
    length: usize,
    weight: u32,
    case_sensitive: bool,
}

impl Glob {
    fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let mut fields = line.splitn(4, ':');
        let weight = fields.next()?.parse().ok()?;
        let mime = fields.next()?.to_string();
        let text = fields.next()?;
        let case_sensitive = fields
            .next()
            .is_some_and(|f| f.split(',').any(|f| f == "cs"));
        if mime.is_empty() || text.is_empty() {
            return None;
        }
        let text = if case_sensitive {
            text.to_string()
        } else {
            text.to_lowercase()
        };
        let pattern = match text.strip_prefix('*') {
            Some(rest) if !rest.contains(['*', '?']) => Pattern::Suffix(rest.to_string()),
            _ if !text.contains(['*', '?']) => Pattern::Exact(text.clone()),
            _ => Pattern::Wild(text.clone()),
        };
        Some(Self {
            mime,
            pattern,
            length: text.chars().count(),
            weight,
            case_sensitive,
        })
    }

    fn matches(&self, name: &str, lower: &str) -> bool {
        let subject = if self.case_sensitive { name } else { lower };
        match &self.pattern {
            Pattern::Suffix(suffix) => subject.len() > suffix.len() && subject.ends_with(suffix),
            Pattern::Exact(exact) => subject == exact,
            Pattern::Wild(pattern) => wild_match(pattern, subject),
        }
    }
}

/// `*` (any run, possibly empty) and `?` (one character) against `text`; nothing else is special.
fn wild_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut backtrack: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) && p[pi] != '*' {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            backtrack = Some((pi, ti));
            pi += 1;
        } else if let Some((star, resume)) = backtrack {
            pi = star + 1;
            ti = resume + 1;
            backtrack = Some((star, resume + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// Extensions the built-in table knows, for a machine without `globs2`.
const BUILTIN: &[(&str, &str)] = &[
    ("txt", "text/plain"),
    ("md", "text/markdown"),
    ("markdown", "text/markdown"),
    ("rs", "text/rust"),
    ("py", "text/x-python"),
    ("sh", "application/x-shellscript"),
    ("toml", "application/toml"),
    ("json", "application/json"),
    ("yaml", "application/yaml"),
    ("yml", "application/yaml"),
    ("xml", "application/xml"),
    ("html", "text/html"),
    ("htm", "text/html"),
    ("css", "text/css"),
    ("js", "text/javascript"),
    ("c", "text/x-csrc"),
    ("h", "text/x-chdr"),
    ("cpp", "text/x-c++src"),
    ("csv", "text/csv"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("bmp", "image/bmp"),
    ("svg", "image/svg+xml"),
    ("pdf", "application/pdf"),
    ("zip", "application/zip"),
    ("tar", "application/x-tar"),
    ("gz", "application/gzip"),
    ("mp3", "audio/mpeg"),
    ("flac", "audio/flac"),
    ("ogg", "audio/ogg"),
    ("wav", "audio/x-wav"),
    ("mp4", "video/mp4"),
    ("mkv", "video/x-matroska"),
    ("webm", "video/webm"),
];

#[derive(Debug, Clone, Default)]
pub struct MimeDb {
    globs: Vec<Glob>,
    /// `child → parents`, from `subclasses`.
    parents: HashMap<String, Vec<String>>,
    /// `canonical → its aliases`, from `aliases`.
    aliases: HashMap<String, Vec<String>>,
}

impl MimeDb {
    /// Reads `mime/globs2`, `mime/subclasses` and `mime/aliases` from every data directory. A
    /// missing or unreadable file just contributes nothing.
    pub fn load(xdg: &XdgDirs) -> Self {
        let mut db = Self::default();
        for dir in xdg.all_data_dirs() {
            let read = |name: &str| std::fs::read_to_string(dir.join("mime").join(name)).ok();
            if let Some(text) = read("globs2") {
                db.add_globs(&text);
            }
            if let Some(text) = read("subclasses") {
                db.add_subclasses(&text);
            }
            if let Some(text) = read("aliases") {
                db.add_aliases(&text);
            }
        }
        db
    }

    pub fn add_globs(&mut self, text: &str) {
        self.globs.extend(text.lines().filter_map(Glob::parse));
    }

    pub fn add_subclasses(&mut self, text: &str) {
        for (child, parent) in pairs(text) {
            self.parents.entry(child).or_default().push(parent);
        }
    }

    pub fn add_aliases(&mut self, text: &str) {
        for (alias, canonical) in pairs(text) {
            self.aliases.entry(canonical).or_default().push(alias);
        }
    }

    /// The type a file *name* says it is: the highest-weight matching glob, the longest pattern
    /// breaking a tie; the built-in extension table when the database has nothing.
    pub fn mime_of_name(&self, name: &str) -> Option<String> {
        let lower = name.to_lowercase();
        self.globs
            .iter()
            .filter(|glob| glob.matches(name, &lower))
            .max_by_key(|glob| (glob.weight, glob.length))
            .map(|glob| glob.mime.clone())
            .or_else(|| builtin(&lower))
    }

    /// The type of the file at `path`: by name, and for a name that says nothing, by whether its
    /// first bytes look like text. Unreadable files and directories are not asked.
    pub fn mime_of_file(&self, path: &Path) -> String {
        if path.is_dir() {
            return "inode/directory".into();
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(mime) = self.mime_of_name(&name) {
            return mime;
        }
        let mut head = vec![0; SNIFF_BYTES];
        let read = std::fs::File::open(path)
            .and_then(|mut file| file.read(&mut head))
            .unwrap_or(0);
        head.truncate(read);
        if looks_like_text(&head) {
            "text/plain".into()
        } else {
            "application/octet-stream".into()
        }
    }

    /// `mime` and every type an application registered for might have been registered under: its
    /// aliases, then its parents (with their aliases), nearest first. Anything `text/*` is also
    /// plain text.
    pub fn lineage(&self, mime: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut queue = std::collections::VecDeque::from([mime.to_string()]);
        while let Some(next) = queue.pop_front() {
            if out.contains(&next) {
                continue;
            }
            out.push(next.clone());
            if let Some(aliases) = self.aliases.get(&next) {
                let fresh: Vec<String> = aliases
                    .iter()
                    .filter(|a| !out.contains(a))
                    .cloned()
                    .collect();
                out.extend(fresh);
            }
            if let Some(parents) = self.parents.get(&next) {
                queue.extend(parents.iter().cloned());
            }
        }
        if mime.starts_with("text/") && !out.iter().any(|t| t == "text/plain") {
            out.push("text/plain".into());
        }
        out
    }
}

fn pairs(text: &str) -> impl Iterator<Item = (String, String)> + '_ {
    text.lines().filter_map(|line| {
        let line = line.trim();
        if line.starts_with('#') {
            return None;
        }
        let (a, b) = line.split_once(char::is_whitespace)?;
        Some((a.to_string(), b.trim().to_string()))
    })
}

fn builtin(lower_name: &str) -> Option<String> {
    let (_, ext) = lower_name
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())?;
    BUILTIN
        .iter()
        .find(|(known, _)| *known == ext)
        .map(|(_, mime)| (*mime).to_string())
}

/// Whether `head` is (the start of) text: no NUL byte and valid UTF-8, allowing the cut to fall
/// in the middle of a character. An empty file counts, as the specification has it.
pub fn looks_like_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    }
}

/// Whether `pattern` (`image/*`, `text/x-python`) covers `mime`. Only a trailing `/*` is a
/// wildcard; anything else must match exactly.
pub fn pattern_covers(pattern: &str, mime: &str) -> bool {
    match pattern.strip_suffix("/*") {
        Some(major) => mime.split_once('/').is_some_and(|(m, _)| m == major),
        None => pattern == mime,
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn db() -> MimeDb {
        let mut db = MimeDb::default();
        db.add_globs(
            "# comment\n\
             50:application/gzip:*.gz\n\
             50:application/x-compressed-tar:*.tar.gz\n\
             50:text/x-makefile:Makefile\n\
             50:text/markdown:*.md\n\
             80:text/x-special:*.md:cs\n\
             50:image/png:*.png\n\
             50:text/x-readme:README*\n\
             50:text/plain:*.txt\n",
        );
        db.add_subclasses(
            "text/markdown text/plain\n\
             application/x-compressed-tar application/gzip\n\
             application/x-pdf application/pdf\n",
        );
        db.add_aliases("text/x-markdown text/markdown\n");
        db
    }

    #[test]
    fn a_longer_pattern_wins_a_tie_and_a_higher_weight_wins_outright() {
        let db = db();
        assert_eq!(
            db.mime_of_name("a.tar.gz").as_deref(),
            Some("application/x-compressed-tar")
        );
        assert_eq!(db.mime_of_name("a.gz").as_deref(), Some("application/gzip"));
        // The weight-80 rule is case sensitive, so only a lower-case `.md` reaches it.
        assert_eq!(db.mime_of_name("a.md").as_deref(), Some("text/x-special"));
        assert_eq!(db.mime_of_name("a.MD").as_deref(), Some("text/markdown"));
    }

    #[test]
    fn exact_names_wildcards_and_case_folding() {
        let db = db();
        assert_eq!(
            db.mime_of_name("Makefile").as_deref(),
            Some("text/x-makefile")
        );
        assert_eq!(
            db.mime_of_name("makefile").as_deref(),
            Some("text/x-makefile")
        );
        assert_eq!(
            db.mime_of_name("README.rst").as_deref(),
            Some("text/x-readme")
        );
        assert_eq!(db.mime_of_name("PHOTO.PNG").as_deref(), Some("image/png"));
        assert_eq!(db.mime_of_name("nothing"), None);
        // A bare suffix is not a name: `.png` alone is a hidden file called png.
        assert_eq!(MimeDb::default().mime_of_name(".png"), None);
    }

    #[test]
    fn the_builtin_table_answers_when_the_database_is_empty() {
        let empty = MimeDb::default();
        assert_eq!(
            empty.mime_of_name("notes.MD").as_deref(),
            Some("text/markdown")
        );
        assert_eq!(
            empty.mime_of_name("a.tar.gz").as_deref(),
            Some("application/gzip")
        );
        assert_eq!(empty.mime_of_name("noext"), None);
    }

    #[test]
    fn lineage_lists_aliases_parents_and_the_implicit_plain_text() {
        let db = db();
        assert_eq!(
            db.lineage("text/markdown"),
            ["text/markdown", "text/x-markdown", "text/plain"]
        );
        assert_eq!(
            db.lineage("application/x-compressed-tar"),
            ["application/x-compressed-tar", "application/gzip"]
        );
        // Anything text is plain text, even without a subclass entry.
        assert_eq!(
            db.lineage("text/x-unknown"),
            ["text/x-unknown", "text/plain"]
        );
        assert_eq!(db.lineage("image/png"), ["image/png"]);
    }

    #[test]
    fn a_cycle_in_subclasses_ends() {
        let mut db = MimeDb::default();
        db.add_subclasses("a/x b/y\nb/y a/x\n");
        assert_eq!(db.lineage("a/x"), ["a/x", "b/y"]);
    }

    #[test]
    fn text_is_told_from_binary_by_content() {
        assert!(looks_like_text(b""));
        assert!(looks_like_text("héllo".as_bytes()));
        // A cut through the middle of `é` is still text.
        assert!(looks_like_text(&"héllo".as_bytes()[..2]));
        assert!(!looks_like_text(b"ab\0cd"));
        assert!(!looks_like_text(&[0xff, 0xfe, 0x41]));
    }

    #[test]
    fn a_nameless_file_is_sniffed() {
        let dir = std::env::temp_dir().join(format!("mm-mime-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("LICENSE"), "plain words\n").unwrap();
        std::fs::write(dir.join("blob"), [0u8, 1, 2, 3]).unwrap();
        let db = MimeDb::default();
        assert_eq!(db.mime_of_file(&dir.join("LICENSE")), "text/plain");
        assert_eq!(
            db.mime_of_file(&dir.join("blob")),
            "application/octet-stream"
        );
        assert_eq!(db.mime_of_file(&dir.join("missing")), "text/plain");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_directory_is_a_directory_not_an_empty_text_file() {
        let dir = std::env::temp_dir();
        assert_eq!(MimeDb::default().mime_of_file(&dir), "inode/directory");
    }

    #[test]
    fn a_mime_pattern_only_wildcards_its_subtype() {
        assert!(pattern_covers("image/*", "image/png"));
        assert!(!pattern_covers("image/*", "video/png"));
        assert!(!pattern_covers("image/*", "imagery/png"));
        assert!(pattern_covers("text/x-python", "text/x-python"));
        assert!(!pattern_covers("text/x-python", "text/x-python3"));
        assert!(!pattern_covers("image", "image/png"));
    }

    #[test]
    fn locales_lose_the_encoding_and_keep_the_language() {
        assert_eq!(locale_candidates("pt_BR.UTF-8"), ["pt_BR", "pt"]);
        assert_eq!(locale_candidates("de@euro"), ["de"]);
        assert!(locale_candidates("C").is_empty());
        assert!(locale_candidates("").is_empty());
    }

    proptest! {
        /// `*` and `?` match like the shell's, checked against a straightforward regex-free
        /// oracle: a pattern made of the text itself always matches, and one that adds a
        /// character the text lacks never does.
        #[test]
        fn a_wildcard_pattern_matches_what_it_was_made_from(text in "[a-c.]{0,12}", cut in 0usize..13) {
            prop_assert!(wild_match("*", &text));
            prop_assert!(wild_match(&text, &text));
            let cut = cut.min(text.len());
            let pattern = format!("{}*{}", &text[..cut], &text[cut..]);
            prop_assert!(wild_match(&pattern, &text));
            let longer = format!("{text}z");
            prop_assert!(!wild_match(&longer, &text));
        }

        /// No file name, however odd, makes a lookup panic, and whatever it finds is a
        /// well-formed `major/minor` type.
        #[test]
        fn any_name_gets_a_wellformed_type_or_none(name in ".{0,40}") {
            if let Some(mime) = db().mime_of_name(&name) {
                prop_assert!(mime.split_once('/').is_some_and(|(a, b)| !a.is_empty() && !b.is_empty()));
            }
        }

        /// Every extension the built-in table lists resolves, in any letter case.
        #[test]
        fn every_builtin_extension_resolves_in_any_case(index in 0..BUILTIN.len(), upper in any::<bool>()) {
            let (ext, mime) = BUILTIN[index];
            let name = if upper { format!("f.{}", ext.to_uppercase()) } else { format!("f.{ext}") };
            let found = MimeDb::default().mime_of_name(&name);
            prop_assert_eq!(found.as_deref(), Some(mime));
        }

        /// A lineage starts at the type itself and never repeats one.
        #[test]
        fn a_lineage_starts_at_the_type_and_has_no_repeats(pick in 0usize..5) {
            let types = ["text/markdown", "application/x-compressed-tar", "image/png", "text/x-q", "a/b"];
            let lineage = db().lineage(types[pick]);
            prop_assert_eq!(lineage[0].as_str(), types[pick]);
            let mut seen = std::collections::HashSet::new();
            prop_assert!(lineage.iter().all(|t| seen.insert(t.clone())));
        }
    }
}
