//! Inline decision marker mining: a small, explicit tag vocabulary
//! (`WHY:`, `DECISION:`, `TRADEOFF:`, `ADR:`, `RATIONALE:`, `REJECTED:`)
//! recognized as a prefix inside any comment syntax. Much lower
//! false-positive risk than the freeform code-comment source
//! (`code_comments.rs`) — this is an explicit opt-in convention, not a
//! keyword guess, so every match is deliberate.
//!
//! A plain text scan, not language-specific parsing (per this issue's
//! own acceptance criteria) — `comment_lines` tracks `/* ... */` state
//! line-by-line rather than reusing `code_comments::comment_block_above`.
//! That helper answers a different question (the comment block sitting
//! *directly above* a specific symbol's declaration line) than this
//! source needs (every comment line in the file, wherever it sits,
//! above or within any symbol) — reusing it here would have meant
//! calling it once per symbol and still needing a separate whole-file
//! scan for markers that aren't adjacent to a declaration at all, which
//! is more complexity than just scanning once.

use crate::{DecisionRecord, DecisionSource};
use repowise_core::RepoIndex;

const MARKERS: &[&str] = &[
    "WHY",
    "DECISION",
    "TRADEOFF",
    "ADR",
    "RATIONALE",
    "REJECTED",
];

/// Mine every comment line in every indexed file for a leading marker
/// tag, linking each hit to the file it's in (the same file-level grain
/// `DecisionRecord::linked_files` uses everywhere else in this crate).
pub fn mine_inline_marker_decisions(index: &RepoIndex) -> Vec<DecisionRecord> {
    let mut records = Vec::new();

    for file in &index.files {
        let Ok(source) = std::fs::read_to_string(&file.path) else {
            continue;
        };
        let rel = file.path.strip_prefix(&index.root).unwrap_or(&file.path);

        for (line, text) in comment_lines(&source) {
            let Some((marker, detail)) = match_marker(&text) else {
                continue;
            };
            records.push(DecisionRecord {
                id: format!("marker:{}:{line}", rel.display()),
                title: detail.clone(),
                source: DecisionSource::InlineMarker {
                    file: file.path.clone(),
                    line,
                    marker: marker.to_string(),
                },
                status: None,
                superseded_by: None,
                date: None,
                body: detail,
                linked_files: vec![file.path.clone()],
            });
        }
    }

    records
}

/// Every line that's (at least partly) a comment, 1-indexed, with its
/// comment-marker-stripped text. Tracks `/* ... */` state across lines;
/// `//`/`#` line comments are recognized only when they start the line
/// (ignoring leading whitespace) — a trailing `code(); // WHY: ...` on
/// the same line as real code is out of scope for this simple scan.
fn comment_lines(source: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut in_block = false;

    for (idx, raw_line) in source.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw_line.trim_start();

        if in_block {
            out.push((line_no, trimmed.to_string()));
            if raw_line.contains("*/") {
                in_block = false;
            }
            continue;
        }

        if trimmed.starts_with("//") || trimmed.starts_with('#') {
            out.push((line_no, trimmed.to_string()));
        } else if trimmed.starts_with("/*") {
            out.push((line_no, trimmed.to_string()));
            if !trimmed.contains("*/") {
                in_block = true;
            }
        }
    }

    out
}

/// Strip a leading comment marker (`///`, `//!`, `//`, `#!`, `#`, `/**`,
/// `/*`, or a JavaDoc/rustdoc-style continuation `*`) from one already-
/// identified comment line.
fn strip_comment_marker(line: &str) -> &str {
    let mut s = line.trim_start();
    for prefix in ["/**", "///", "//!", "//", "#!", "#", "/*"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim_start();
            return s;
        }
    }
    if let Some(rest) = s.strip_prefix('*') {
        if !rest.starts_with('/') {
            s = rest.trim_start();
        }
    }
    s
}

/// If `line` (already known to be a comment) starts, after its comment
/// marker, with one of `MARKERS` followed by `:`, the marker tag and the
/// rest of the line (trailing `*/` and whitespace trimmed) as the
/// decision's detail text.
fn match_marker(line: &str) -> Option<(&'static str, String)> {
    let content = strip_comment_marker(line);
    for &marker in MARKERS {
        if let Some(rest) = content.strip_prefix(marker) {
            if let Some(rest) = rest.strip_prefix(':') {
                let detail = rest.trim().trim_end_matches("*/").trim().to_string();
                return Some((marker, detail));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::{FileRecord, Language, RepoIndex};
    use std::path::PathBuf;

    fn index_for(root: &std::path::Path, path: PathBuf) -> RepoIndex {
        RepoIndex {
            root: root.to_path_buf(),
            files: vec![FileRecord {
                path,
                language: Language::Other,
                lines: 10,
                symbols: Vec::new(),
                imports: Vec::new(),
                calls: Vec::new(),
                field_accesses: Vec::new(),
            }],
            other_files: 0,
        }
    }

    fn mine_single_file(contents: &str) -> Vec<DecisionRecord> {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("lib.rs");
        std::fs::write(&path, contents).unwrap();
        mine_inline_marker_decisions(&index_for(&root, path))
    }

    #[test]
    fn recognizes_every_marker_tag_in_hash_comment_syntax() {
        for marker in MARKERS {
            let contents = format!("# {marker}: some detail here\nx = 1\n");
            let records = mine_single_file(&contents);
            assert_eq!(records.len(), 1, "expected a hit for {marker}");
            assert!(matches!(
                &records[0].source,
                DecisionSource::InlineMarker { line: 1, marker: m, .. } if m == marker
            ));
            assert_eq!(records[0].title, "some detail here");
        }
    }

    #[test]
    fn recognizes_a_marker_in_double_slash_comment_syntax() {
        let records = mine_single_file("// DECISION: adopt sled over rocksdb\nfn f() {}\n");
        assert_eq!(records.len(), 1);
        assert!(matches!(
            &records[0].source,
            DecisionSource::InlineMarker { marker, .. } if marker == "DECISION"
        ));
    }

    #[test]
    fn recognizes_a_marker_in_a_block_comment() {
        let records = mine_single_file("/*\n * WHY: sled has simpler ops\n */\nfn f() {}\n");
        assert_eq!(records.len(), 1);
        assert!(matches!(
            &records[0].source,
            DecisionSource::InlineMarker { line: 2, marker, .. } if marker == "WHY"
        ));
        assert_eq!(records[0].title, "sled has simpler ops");
    }

    #[test]
    fn links_to_the_file_the_marker_sits_in() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("lib.rs");
        std::fs::write(&path, "# TRADEOFF: simplicity over raw throughput\n").unwrap();

        let records = mine_inline_marker_decisions(&index_for(&root, path.clone()));

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].linked_files, vec![path]);
    }

    #[test]
    fn ignores_a_plain_comment_with_no_marker_tag() {
        let records = mine_single_file("# just a regular comment\nfn f() {}\n");
        assert!(records.is_empty());
    }

    #[test]
    fn ignores_a_word_that_only_resembles_a_marker() {
        // "ADRENALINE" starts with "ADR" but isn't followed by ':'.
        let records = mine_single_file("# ADRENALINE: not a real marker\n");
        assert!(records.is_empty());
    }
}
