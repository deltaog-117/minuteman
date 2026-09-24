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

//! Classifies source text into a small, curated set of syntax-highlighting categories
//! (`TokenKind`) using `syntect`'s bundled Sublime-syntax definitions. Pure data, no color: `tui`
//! maps each `TokenKind` onto whichever theme is active, the same way `tui::style::FileKind`
//! classifies a file and leaves picking its actual color to `tui`.

use std::path::Path;
use std::sync::OnceLock;

use syntect::easy::ScopeRegionIterator;
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    String,
    Comment,
    Number,
    Function,
    Type,
    Plain,
}

/// A source line's tokens in order, already classified — concatenating every piece's text
/// reproduces the original line with its trailing newline removed.
pub type HighlightedLine = Vec<(TokenKind, String)>;

fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// Whether `file_name` (just the name — this never touches the filesystem, unlike syntect's own
/// `find_syntax_for_file`) maps to a real syntax definition rather than falling back to
/// undecorated "Plain Text". Worth checking before bothering to call [`highlight`], which still
/// works either way but has nothing to color for a file with no known syntax.
pub fn has_syntax(file_name: &str) -> bool {
    syntax_for(file_name).is_some()
}

/// Tries `file_name` whole first (this is how a bundled syntax matches an exact, extensionless
/// name like `Makefile` or `Dockerfile`), then its extension — mirroring
/// `SyntaxSet::find_syntax_for_file` minus the first-line shebang fallback, which needs to open
/// the file and so doesn't fit a function that only ever sees already-loaded text.
fn syntax_for(file_name: &str) -> Option<&'static SyntaxReference> {
    let ss = syntax_set();
    let extension = Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    ss.find_syntax_by_extension(file_name)
        .or_else(|| ss.find_syntax_by_extension(extension))
}

/// Tokenizes `text` line by line for `file_name`'s syntax, falling back to one `Plain` span per
/// line when none is known. Runs the whole file through `syntect`'s line-oriented parser, so call
/// it off the render thread the same way `preview::load_text` is read off it.
pub fn highlight(text: &str, file_name: &str) -> Vec<HighlightedLine> {
    let ss = syntax_set();
    let syntax = syntax_for(file_name).unwrap_or_else(|| ss.find_syntax_plain_text());
    let mut parse_state = ParseState::new(syntax);
    let mut scope_stack = ScopeStack::new();
    let mut lines = Vec::new();

    for line in LinesWithEndings::from(text) {
        let Ok(ops) = parse_state.parse_line(line, ss) else {
            lines.push(vec![(TokenKind::Plain, strip_newline(line).to_string())]);
            continue;
        };

        let mut spans: HighlightedLine = Vec::new();
        for (piece, op) in ScopeRegionIterator::new(&ops, line) {
            let _ = scope_stack.apply(op);
            if piece.is_empty() {
                continue;
            }
            let kind = classify(&scope_stack);
            match spans.last_mut() {
                Some((last_kind, last_text)) if *last_kind == kind => last_text.push_str(piece),
                _ => spans.push((kind, piece.to_string())),
            }
        }
        if let Some((_, last_text)) = spans.last_mut() {
            let trimmed = strip_newline(last_text).to_string();
            if trimmed.is_empty() {
                spans.pop();
            } else {
                *last_text = trimmed;
            }
        }
        lines.push(spans);
    }
    lines
}

fn strip_newline(line: &str) -> &str {
    line.strip_suffix("\r\n")
        .or_else(|| line.strip_suffix('\n'))
        .unwrap_or(line)
}

/// The most specific (topmost) scope in `stack` this project draws a distinct color for, or
/// `Plain` if none of them are. Deliberately a small, curated set rather than every scope
/// syntect's bundled grammars can produce, so it stays mappable onto a handful of theme colors
/// instead of needing one theme field per possible scope.
fn classify(stack: &ScopeStack) -> TokenKind {
    stack
        .scopes
        .iter()
        .rev()
        .find_map(|scope| classify_scope(&scope.to_string()))
        .unwrap_or(TokenKind::Plain)
}

fn classify_scope(scope: &str) -> Option<TokenKind> {
    if scope.starts_with("comment") {
        Some(TokenKind::Comment)
    } else if scope.starts_with("string") {
        Some(TokenKind::String)
    } else if scope.starts_with("constant.numeric") || scope.starts_with("constant.character") {
        Some(TokenKind::Number)
    } else if scope.starts_with("constant.language") {
        // `true`/`false`/`nil`/`self`-style built-ins read like keywords in most editor themes.
        Some(TokenKind::Keyword)
    } else if scope.starts_with("constant") {
        Some(TokenKind::Number)
    } else if scope.starts_with("keyword") || scope.starts_with("storage") {
        Some(TokenKind::Keyword)
    } else if scope.starts_with("entity.name.function") || scope.starts_with("support.function") {
        Some(TokenKind::Function)
    } else if scope.starts_with("entity.name.type")
        || scope.starts_with("entity.name.class")
        || scope.starts_with("entity.name.struct")
        || scope.starts_with("support.type")
        || scope.starts_with("support.class")
    {
        Some(TokenKind::Type)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lines: &[HighlightedLine]) -> Vec<Vec<TokenKind>> {
        lines
            .iter()
            .map(|line| line.iter().map(|(kind, _)| *kind).collect())
            .collect()
    }

    fn text(lines: &[HighlightedLine]) -> String {
        lines
            .iter()
            .map(|line| line.iter().map(|(_, text)| text.as_str()).collect())
            .collect::<Vec<String>>()
            .join("\n")
    }

    #[test]
    fn an_unknown_extension_is_one_plain_span_per_line() {
        let lines = highlight("hello\nworld", "notes.mystery");
        assert_eq!(kinds(&lines), vec![vec![TokenKind::Plain]; 2]);
        assert_eq!(text(&lines), "hello\nworld");
    }

    #[test]
    fn rust_keywords_strings_comments_and_numbers_are_told_apart() {
        let source = "// a comment\nfn main() {\n    let n = 42;\n    let s = \"hi\";\n}\n";
        let lines = highlight(source, "main.rs");

        assert!(
            lines[0]
                .iter()
                .any(|(k, t)| *k == TokenKind::Comment && t.contains("a comment"))
        );
        assert!(
            lines[1]
                .iter()
                .any(|(k, t)| *k == TokenKind::Keyword && t == "fn")
        );
        assert!(
            lines[1]
                .iter()
                .any(|(k, t)| *k == TokenKind::Function && t == "main")
        );
        assert!(
            lines[2]
                .iter()
                .any(|(k, t)| *k == TokenKind::Number && t == "42")
        );
        assert!(
            lines[3]
                .iter()
                .any(|(k, t)| *k == TokenKind::String && t.contains("hi"))
        );
    }

    #[test]
    fn an_exact_extensionless_file_name_is_still_recognised() {
        assert!(has_syntax("Makefile"));
        assert!(!has_syntax("notes.mystery"));
    }

    #[test]
    fn concatenating_every_span_reproduces_the_original_lines() {
        let source = "fn main() {\n    println!(\"hi\");\n}";
        let lines = highlight(source, "main.rs");
        assert_eq!(text(&lines), source);
    }

    #[test]
    fn adjacent_same_kind_pieces_are_merged() {
        // Several punctuation/plain tokens in a row inside a plain-text file should not fragment
        // into one span per character.
        let lines = highlight("just plain words, nothing special", "notes.mystery");
        assert_eq!(lines[0].len(), 1);
    }
}
