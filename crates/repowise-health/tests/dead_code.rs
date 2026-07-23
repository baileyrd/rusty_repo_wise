//! Exercises `find_dead_code`'s confidence tiering directly against
//! hand-built `RepoIndex` fixtures (no parsing needed) — mirrors
//! `tests/scoring.rs`'s fixture style.

use repowise_core::{CallRef, FileRecord, ImportRef, Language, RepoIndex, Symbol, SymbolKind};
use repowise_graph::RepoGraph;
use repowise_health::{find_dead_code, DeadCodeConfidence};
use std::path::{Path, PathBuf};

#[allow(clippy::too_many_arguments)]
fn symbol(file: &str, name: &str, kind: SymbolKind, start_line: usize, end_line: usize) -> Symbol {
    let file = PathBuf::from(file);
    Symbol {
        id: Symbol::make_id(&file, name, start_line),
        name: name.to_string(),
        kind,
        file,
        start_line,
        end_line,
        parent: None,
        complexity: 1,
        max_nesting_depth: 0,
        param_count: 0,
        body_hash: None,
    }
}

fn file_record(path: &str, symbols: Vec<Symbol>, calls: Vec<CallRef>) -> FileRecord {
    file_record_full(path, Language::Other, symbols, calls, Vec::new())
}

fn file_record_full(
    path: &str,
    language: Language,
    symbols: Vec<Symbol>,
    calls: Vec<CallRef>,
    imports: Vec<ImportRef>,
) -> FileRecord {
    FileRecord {
        path: PathBuf::from(path),
        language,
        lines: 100,
        symbols,
        imports,
        calls,
        field_accesses: Vec::new(),
    }
}

fn index(files: Vec<FileRecord>) -> RepoIndex {
    RepoIndex {
        root: PathBuf::from("/fixture"),
        files,
        other_files: 0,
    }
}

#[test]
fn uncalled_function_with_no_risk_factors_is_high_confidence() {
    let helper = symbol("lib.rs", "helper", SymbolKind::Function, 1, 3);
    let idx = index(vec![file_record("lib.rs", vec![helper], vec![])]);
    let graph = RepoGraph::build(&idx);

    let candidates = find_dead_code(&idx, &graph);

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].symbol, "helper");
    assert_eq!(candidates[0].confidence, DeadCodeConfidence::High);
    assert!(candidates[0].risk_factors.is_empty());
}

#[test]
fn called_function_is_not_a_candidate_at_all() {
    let helper = symbol("lib.rs", "helper", SymbolKind::Function, 1, 3);
    let caller = symbol("lib.rs", "caller", SymbolKind::Function, 5, 7);
    let call = CallRef {
        caller: Some(caller.id.clone()),
        callee_name: "helper".to_string(),
        line: 6,
    };
    let idx = index(vec![file_record(
        "lib.rs",
        vec![helper, caller],
        vec![call],
    )]);
    let graph = RepoGraph::build(&idx);

    let candidates = find_dead_code(&idx, &graph);

    // `helper` has a caller, `caller` itself has zero callers -> it's the
    // only remaining candidate.
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].symbol, "caller");
}

#[test]
fn ambiguous_name_downgrades_to_medium_confidence() {
    // Two distinct, unrelated `helper` symbols in different files, both
    // uncalled -> each is an "ambiguous name" risk for the other.
    let helper_a = symbol("a.rs", "helper", SymbolKind::Function, 1, 3);
    let helper_b = symbol("b.rs", "helper", SymbolKind::Function, 1, 3);
    let idx = index(vec![
        file_record("a.rs", vec![helper_a], vec![]),
        file_record("b.rs", vec![helper_b], vec![]),
    ]);
    let graph = RepoGraph::build(&idx);

    let candidates = find_dead_code(&idx, &graph);

    assert_eq!(candidates.len(), 2);
    for c in &candidates {
        assert_eq!(c.confidence, DeadCodeConfidence::Medium);
        assert_eq!(c.risk_factors.len(), 1);
    }
}

#[test]
fn unresolved_import_matching_file_stem_downgrades_to_medium_confidence() {
    let helper = symbol("utils.rs", "do_thing", SymbolKind::Function, 1, 3);
    // An import elsewhere that never resolves to any indexed file, but
    // whose last segment ("utils") matches utils.rs's stem.
    let importer = file_record_full(
        "main.rs",
        Language::TypeScript,
        vec![],
        vec![],
        vec![ImportRef {
            path: "some/missing/utils".to_string(),
            line: 1,
            resolved_file: None,
        }],
    );
    let idx = index(vec![
        file_record("utils.rs", vec![helper], vec![]),
        importer,
    ]);
    let graph = RepoGraph::build(&idx);
    assert!(graph.unresolved_import_stems.contains("utils"));

    let candidates = find_dead_code(&idx, &graph);

    let utils_candidate = candidates.iter().find(|c| c.symbol == "do_thing").unwrap();
    assert_eq!(utils_candidate.confidence, DeadCodeConfidence::Medium);
    assert_eq!(utils_candidate.risk_factors.len(), 1);
}

#[test]
fn both_risk_factors_together_downgrade_to_low_confidence() {
    let helper_a = symbol("utils.rs", "helper", SymbolKind::Function, 1, 3);
    let helper_b = symbol("other.rs", "helper", SymbolKind::Function, 1, 3);
    let importer = file_record_full(
        "main.rs",
        Language::TypeScript,
        vec![],
        vec![],
        vec![ImportRef {
            path: "some/missing/utils".to_string(),
            line: 1,
            resolved_file: None,
        }],
    );
    let idx = index(vec![
        file_record("utils.rs", vec![helper_a], vec![]),
        file_record("other.rs", vec![helper_b], vec![]),
        importer,
    ]);
    let graph = RepoGraph::build(&idx);

    let candidates = find_dead_code(&idx, &graph);

    let utils_candidate = candidates
        .iter()
        .find(|c| c.file == Path::new("utils.rs"))
        .unwrap();
    assert_eq!(utils_candidate.confidence, DeadCodeConfidence::Low);
    assert_eq!(utils_candidate.risk_factors.len(), 2);
}

#[test]
fn shell_functions_are_never_reported_as_dead_code_candidates() {
    let func = symbol("script.sh", "run", SymbolKind::Function, 1, 3);
    let idx = index(vec![file_record_full(
        "script.sh",
        Language::Shell,
        vec![func],
        vec![],
        vec![],
    )]);
    let graph = RepoGraph::build(&idx);

    let candidates = find_dead_code(&idx, &graph);

    assert!(candidates.is_empty());
}

#[test]
fn results_are_sorted_highest_confidence_first() {
    let high = symbol("high.rs", "solo", SymbolKind::Function, 1, 3);
    let low_a = symbol("low_a.rs", "dup", SymbolKind::Function, 1, 3);
    let low_b = symbol("low_b.rs", "dup", SymbolKind::Function, 1, 3);
    let idx = index(vec![
        file_record("high.rs", vec![high], vec![]),
        file_record("low_a.rs", vec![low_a], vec![]),
        file_record("low_b.rs", vec![low_b], vec![]),
    ]);
    let graph = RepoGraph::build(&idx);

    let candidates = find_dead_code(&idx, &graph);

    assert_eq!(candidates.len(), 3);
    assert_eq!(candidates[0].confidence, DeadCodeConfidence::High);
    assert_eq!(candidates[1].confidence, DeadCodeConfidence::Medium);
    assert_eq!(candidates[2].confidence, DeadCodeConfidence::Medium);
}
