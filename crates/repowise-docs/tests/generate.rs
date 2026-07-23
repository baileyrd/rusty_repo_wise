//! Exercises `generate()`'s freshness detection (New/Changed/Unchanged)
//! against a real directory on disk, since it reads each file's source
//! from disk to compute its content hash — nothing here is mocked.

use repowise_core::{FileRecord, Language, RepoIndex, Symbol, SymbolKind};
use repowise_docs::{generate, PageStatus};
use repowise_graph::RepoGraph;
use repowise_health::analyze;
use std::fs;

fn build_index(root: &std::path::Path) -> RepoIndex {
    let file_path = root.join("lib.rs");
    let symbol = Symbol {
        id: Symbol::make_id(&file_path, "helper", 1),
        name: "helper".to_string(),
        kind: SymbolKind::Function,
        file: file_path.clone(),
        start_line: 1,
        end_line: 1,
        parent: None,
        complexity: 1,
        param_count: 0,
        body_hash: None,
    };
    RepoIndex {
        root: root.to_path_buf(),
        files: vec![FileRecord {
            path: file_path,
            language: Language::Rust,
            lines: 1,
            symbols: vec![symbol],
            imports: Vec::new(),
            calls: Vec::new(),
            field_accesses: Vec::new(),
        }],
        other_files: 0,
    }
}

#[test]
fn tracks_new_unchanged_and_changed_pages() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("lib.rs"), "fn helper() {}\n").unwrap();

    let index = build_index(&root);
    let graph = RepoGraph::build(&index);
    let health = analyze(&index, &graph);

    // First run: no prior page on disk.
    let first = generate(&index, &graph, &health).unwrap();
    assert_eq!(first.pages.len(), 1);
    assert_eq!(first.pages[0].status, PageStatus::New);
    assert!(first.pages[0].wiki_path.is_file());
    assert_eq!(first.counts(), (1, 0, 0));

    // Second run: source unchanged.
    let second = generate(&index, &graph, &health).unwrap();
    assert_eq!(second.pages[0].status, PageStatus::Unchanged);
    assert_eq!(second.counts(), (0, 0, 1));

    // Third run: source file's content changed on disk.
    fs::write(root.join("lib.rs"), "fn helper() { 1 }\n").unwrap();
    let third = generate(&index, &graph, &health).unwrap();
    assert_eq!(third.pages[0].status, PageStatus::Changed);
    assert_eq!(third.counts(), (0, 1, 0));

    let content = fs::read_to_string(&third.pages[0].wiki_path).unwrap();
    assert!(content.contains("helper"));
    assert!(content.contains("content-hash:"));
}
