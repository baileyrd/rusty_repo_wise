//! Code-comment decision mining: scans the comment/docstring block sitting
//! directly above each indexed symbol for the same decision-keyword
//! heuristic `commits.rs`/`pull_requests.rs` already use. Pure
//! filesystem/parsing work, unlike the PR-body source — no new
//! dependency, no network call.
//!
//! Deliberately scoped to comments *immediately above* a symbol's
//! declaration line (the common doc-comment convention across most
//! languages this port parses: `///`/`/** */` above a Rust/Java/C-family
//! declaration, `#`-prefixed lines above a Python/Ruby function). Python
//! and JavaScript's alternative convention — a docstring as the first
//! statement *inside* the function body — isn't handled; a documented
//! gap, not a silent one.

use crate::commits::is_decision_message;
use crate::{DecisionRecord, DecisionSource};
use repowise_core::RepoIndex;
use std::collections::HashSet;

/// Mine decision-like comments sitting directly above an indexed
/// function/method/class/etc. Re-reads each file's source fresh from
/// disk (comment text isn't kept anywhere in `RepoIndex`) — same
/// tradeoff `get_symbol`/`repowise-docs` already make elsewhere in this
/// port.
pub fn mine_code_comment_decisions(index: &RepoIndex) -> Vec<DecisionRecord> {
    let mut records = Vec::new();

    for file in &index.files {
        let Ok(source) = std::fs::read_to_string(&file.path) else {
            continue;
        };
        let lines: Vec<&str> = source.lines().collect();

        // A comment block can sit above several symbols only if they
        // share the very same start_line (never happens in practice),
        // so this just guards against mining the exact same block twice
        // for pathological input.
        let mut seen_starts: HashSet<usize> = HashSet::new();

        for sym in &file.symbols {
            let Some((comment_line, text)) = comment_block_above(&lines, sym.start_line) else {
                continue;
            };
            if !is_decision_message(&text) {
                continue;
            }
            if !seen_starts.insert(comment_line) {
                continue;
            }

            let rel = file.path.strip_prefix(&index.root).unwrap_or(&file.path);
            records.push(DecisionRecord {
                id: format!("comment:{}:{comment_line}", rel.display()),
                title: summarize(&text),
                source: DecisionSource::CodeComment {
                    file: file.path.clone(),
                    line: comment_line,
                },
                status: None,
                superseded_by: None,
                date: None,
                body: text,
                linked_files: vec![file.path.clone()],
            });
        }
    }

    records
}

/// The contiguous comment block ending on the line directly above
/// `start_line` (1-indexed, matching `Symbol::start_line`), if any:
/// either a run of `//`/`#`-prefixed lines, or a `/* ... */` block
/// (walked upward from its closing `*/` to the matching opening `/*`).
/// Returns the block's own first line number and full text.
fn comment_block_above(lines: &[&str], start_line: usize) -> Option<(usize, String)> {
    if start_line < 2 {
        return None;
    }
    let above_idx = start_line - 2;
    let above = lines.get(above_idx)?.trim();
    if above.is_empty() {
        return None;
    }

    if above.ends_with("*/") {
        let mut i = above_idx;
        loop {
            if lines[i].contains("/*") {
                break;
            }
            if i == 0 {
                return None;
            }
            i -= 1;
        }
        let text = lines[i..=above_idx].join("\n");
        return Some((i + 1, text));
    }

    let is_line_comment: fn(&str) -> bool = if above.starts_with("//") {
        |l: &str| l.trim_start().starts_with("//")
    } else if above.starts_with('#') {
        |l: &str| l.trim_start().starts_with('#')
    } else {
        return None;
    };

    let mut i = above_idx;
    while i > 0 && is_line_comment(lines[i - 1]) {
        i -= 1;
    }
    let text = lines[i..=above_idx].join("\n");
    Some((i + 1, text))
}

/// First non-blank line of a comment block, with leading comment
/// markers (`/`, `*`, `#`) and whitespace trimmed, as a short title.
fn summarize(text: &str) -> String {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| {
            l.trim_start_matches(['/', '*', '#', ' '])
                .trim()
                .to_string()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::{FileRecord, Language, Symbol, SymbolKind};
    use std::path::PathBuf;

    fn write_file(dir: &std::path::Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn symbol_at(file: &std::path::Path, name: &str, start_line: usize) -> Symbol {
        Symbol {
            id: Symbol::make_id(file, name, start_line),
            name: name.to_string(),
            kind: SymbolKind::Function,
            file: file.to_path_buf(),
            start_line,
            end_line: start_line + 1,
            parent: None,
            complexity: 1,
            max_nesting_depth: 0,
            param_count: 0,
            body_hash: None,
        }
    }

    fn index_with(root: &std::path::Path, path: PathBuf, symbols: Vec<Symbol>) -> RepoIndex {
        RepoIndex {
            root: root.to_path_buf(),
            files: vec![FileRecord {
                path,
                language: Language::Rust,
                lines: 10,
                symbols,
                imports: Vec::new(),
                calls: Vec::new(),
                field_accesses: Vec::new(),
            }],
            other_files: 0,
        }
    }

    #[test]
    fn mines_a_decision_like_line_comment_above_a_function() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = write_file(
            &root,
            "lib.rs",
            "// We decided to adopt sled over rocksdb for the index store.\nfn open_store() {}\n",
        );
        let index = index_with(&root, path.clone(), vec![symbol_at(&path, "open_store", 2)]);

        let records = mine_code_comment_decisions(&index);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].linked_files, vec![path.clone()]);
        assert!(matches!(
            &records[0].source,
            DecisionSource::CodeComment { line: 1, .. }
        ));
        assert_eq!(
            records[0].title,
            "We decided to adopt sled over rocksdb for the index store."
        );
    }

    #[test]
    fn mines_a_decision_like_block_comment_above_a_function() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = write_file(
            &root,
            "lib.rs",
            "/*\n * We chose sled instead of rocksdb here.\n */\nfn open_store() {}\n",
        );
        let index = index_with(&root, path.clone(), vec![symbol_at(&path, "open_store", 4)]);

        let records = mine_code_comment_decisions(&index);

        assert_eq!(records.len(), 1);
        assert!(matches!(
            &records[0].source,
            DecisionSource::CodeComment { line: 1, .. }
        ));
    }

    #[test]
    fn ignores_a_non_decision_comment() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = write_file(
            &root,
            "lib.rs",
            "// Opens the on-disk store.\nfn open_store() {}\n",
        );
        let index = index_with(&root, path.clone(), vec![symbol_at(&path, "open_store", 2)]);

        assert!(mine_code_comment_decisions(&index).is_empty());
    }

    #[test]
    fn ignores_a_symbol_with_no_comment_directly_above_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = write_file(
            &root,
            "lib.rs",
            "// We decided to adopt sled.\n\nfn open_store() {}\n",
        );
        // A blank line separates the comment from the function -> not
        // "directly above" by this port's deliberately simple rule.
        let index = index_with(&root, path.clone(), vec![symbol_at(&path, "open_store", 3)]);

        assert!(mine_code_comment_decisions(&index).is_empty());
    }
}
