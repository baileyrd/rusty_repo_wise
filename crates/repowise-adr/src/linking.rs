use repowise_core::{RepoIndex, SymbolKind};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Symbol names shorter than this are skipped when linking — short
/// identifiers (`i`, `db`, `Ok`) match too much prose to be a useful
/// signal.
const MIN_SYMBOL_NAME_LEN: usize = 4;

/// Best-effort link from a decision's body text to the indexed files it
/// mentions: either the file's own relative path appearing verbatim, or
/// one of its (non-module) symbol names appearing as a whole word.
/// Matching text, not meaning — a decision that refers to a file/symbol
/// only descriptively ("the queue module") won't be linked.
pub fn link_to_index(body: &str, index: &RepoIndex) -> Vec<PathBuf> {
    let words: BTreeSet<&str> = body
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();

    let mut linked = BTreeSet::new();
    for file in &index.files {
        let rel = file.path.strip_prefix(&index.root).unwrap_or(&file.path);
        let rel_str = rel.to_string_lossy();
        if rel_str.len() > 3 && body.contains(rel_str.as_ref()) {
            linked.insert(file.path.clone());
            continue;
        }
        let mentioned = file.symbols.iter().any(|sym| {
            !matches!(sym.kind, SymbolKind::Module)
                && sym.name.len() >= MIN_SYMBOL_NAME_LEN
                && words.contains(sym.name.as_str())
        });
        if mentioned {
            linked.insert(file.path.clone());
        }
    }
    linked.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::{FileRecord, Language, Symbol};
    use std::path::PathBuf;

    #[test]
    fn links_by_file_path_and_symbol_name() {
        let root = PathBuf::from("/repo");
        let a_path = root.join("queue.rs");
        let b_path = root.join("cache.rs");

        let symbol = Symbol {
            id: Symbol::make_id(&a_path, "TaskQueue", 1),
            name: "TaskQueue".to_string(),
            kind: SymbolKind::Struct,
            file: a_path.clone(),
            start_line: 1,
            end_line: 1,
            parent: None,
            complexity: 0,
            max_nesting_depth: 0,
            bumpy_road_bumps: 0,
            param_count: 0,
            body_hash: None,
        };
        let index = RepoIndex {
            root: root.clone(),
            files: vec![
                FileRecord {
                    path: a_path.clone(),
                    language: Language::Other,
                    lines: 1,
                    symbols: vec![symbol],
                    imports: vec![],
                    calls: vec![],
                    field_accesses: vec![],
                },
                FileRecord {
                    path: b_path.clone(),
                    language: Language::Other,
                    lines: 1,
                    symbols: vec![],
                    imports: vec![],
                    calls: vec![],
                    field_accesses: vec![],
                },
            ],
            other_files: 0,
        };

        let body = "We decided to introduce TaskQueue instead of a naive Vec.";
        let linked = link_to_index(body, &index);
        assert_eq!(linked, vec![a_path]);

        let body2 = "See cache.rs for the eviction policy.";
        let linked2 = link_to_index(body2, &index);
        assert_eq!(linked2, vec![b_path]);
    }
}
