//! Tests build `RepoIndex`/`FileRecord`/`Symbol` fixtures directly (no
//! parsing needed) so each marker can be exercised in isolation. Files use
//! `Language::Other` to skip repowise-graph's Rust/Python module-path
//! resolution, which isn't relevant here and would otherwise touch disk.

use repowise_core::{CallRef, FileRecord, Language, RepoIndex, Symbol, SymbolKind};
use repowise_graph::RepoGraph;
use repowise_health::{
    analyze, FindingKind, GOD_CLASS_METHODS, HIGH_COMPLEXITY, LONG_FUNCTION_LINES, TOO_MANY_PARAMS,
};
use std::path::{Path, PathBuf};

#[allow(clippy::too_many_arguments)]
fn symbol(
    file: &str,
    name: &str,
    kind: SymbolKind,
    start_line: usize,
    end_line: usize,
    parent: Option<&str>,
    complexity: usize,
    param_count: usize,
    body_hash: Option<u64>,
) -> Symbol {
    let file = PathBuf::from(file);
    Symbol {
        id: Symbol::make_id(&file, name, start_line),
        name: name.to_string(),
        kind,
        file,
        start_line,
        end_line,
        parent: parent.map(str::to_string),
        complexity,
        param_count,
        body_hash,
    }
}

fn file_record(path: &str, symbols: Vec<Symbol>, calls: Vec<CallRef>) -> FileRecord {
    file_record_with_language(path, Language::Other, symbols, calls)
}

fn file_record_with_language(
    path: &str,
    language: Language,
    symbols: Vec<Symbol>,
    calls: Vec<CallRef>,
) -> FileRecord {
    FileRecord {
        path: PathBuf::from(path),
        language,
        lines: 1000,
        symbols,
        imports: Vec::new(),
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

fn findings_for<'a>(
    report: &'a repowise_health::HealthReport,
    name: &str,
    kind: FindingKind,
) -> Vec<&'a repowise_health::Finding> {
    report
        .findings
        .iter()
        .filter(|f| f.symbol.as_deref() == Some(name) && f.kind == kind)
        .collect()
}

#[test]
fn flags_long_high_complexity_and_too_many_params() {
    let big = symbol(
        "big.rs",
        "big",
        SymbolKind::Function,
        1,
        1 + LONG_FUNCTION_LINES + 5,
        None,
        HIGH_COMPLEXITY + 3,
        TOO_MANY_PARAMS + 2,
        None,
    );
    let idx = index(vec![file_record("big.rs", vec![big], vec![])]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    assert_eq!(
        findings_for(&report, "big", FindingKind::LongFunction).len(),
        1
    );
    assert_eq!(
        findings_for(&report, "big", FindingKind::HighComplexity).len(),
        1
    );
    assert_eq!(
        findings_for(&report, "big", FindingKind::TooManyParameters).len(),
        1
    );
    // No callers anywhere in the fixture.
    assert_eq!(
        findings_for(&report, "big", FindingKind::PossiblyDeadCode).len(),
        1
    );
}

#[test]
fn flags_god_classes() {
    let mut symbols = vec![symbol(
        "big.rs",
        "Big",
        SymbolKind::Struct,
        1,
        200,
        None,
        0,
        0,
        None,
    )];
    for i in 0..(GOD_CLASS_METHODS + 1) {
        symbols.push(symbol(
            "big.rs",
            &format!("method_{i}"),
            SymbolKind::Method,
            10 + i,
            10 + i,
            Some("Big"),
            1,
            0,
            None,
        ));
    }
    let idx = index(vec![file_record("big.rs", symbols, vec![])]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    let god_class = findings_for(&report, "Big", FindingKind::GodClass);
    assert_eq!(god_class.len(), 1);
    assert!(god_class[0]
        .detail
        .contains(&(GOD_CLASS_METHODS + 1).to_string()));
}

#[test]
fn flags_duplicate_code_across_files() {
    let a = symbol(
        "a.rs",
        "a_fn",
        SymbolKind::Function,
        1,
        10,
        None,
        1,
        0,
        Some(42),
    );
    let b = symbol(
        "b.rs",
        "b_fn",
        SymbolKind::Function,
        1,
        10,
        None,
        1,
        0,
        Some(42),
    );
    let c = symbol(
        "c.rs",
        "unique_fn",
        SymbolKind::Function,
        1,
        10,
        None,
        1,
        0,
        Some(99),
    );

    let idx = index(vec![
        file_record("a.rs", vec![a], vec![]),
        file_record("b.rs", vec![b], vec![]),
        file_record("c.rs", vec![c], vec![]),
    ]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    let dup_a = findings_for(&report, "a_fn", FindingKind::DuplicateCode);
    assert_eq!(dup_a.len(), 1);
    assert!(dup_a[0].detail.contains("b_fn"));
    assert_eq!(
        findings_for(&report, "b_fn", FindingKind::DuplicateCode).len(),
        1
    );
    assert!(findings_for(&report, "unique_fn", FindingKind::DuplicateCode).is_empty());
}

#[test]
fn dead_code_is_only_flagged_when_uncalled() {
    let used = symbol(
        "f.rs",
        "used",
        SymbolKind::Function,
        20,
        22,
        None,
        1,
        0,
        None,
    );
    let unused = symbol(
        "f.rs",
        "unused",
        SymbolKind::Function,
        30,
        32,
        None,
        1,
        0,
        None,
    );
    let caller = symbol(
        "f.rs",
        "caller",
        SymbolKind::Function,
        1,
        5,
        None,
        1,
        0,
        None,
    );
    let call = CallRef {
        caller: Some(caller.id.clone()),
        callee_name: "used".to_string(),
        line: 3,
    };
    let idx = index(vec![file_record(
        "f.rs",
        vec![used, unused, caller],
        vec![call],
    )]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    assert!(findings_for(&report, "used", FindingKind::PossiblyDeadCode).is_empty());
    assert_eq!(
        findings_for(&report, "unused", FindingKind::PossiblyDeadCode).len(),
        1
    );
}

#[test]
fn shell_functions_are_never_flagged_as_dead_code() {
    // Per repowise's own documented scope for the shell tier, an
    // uncalled shell function must never be flagged -- it's routinely
    // invoked only from the command line, another script, or a cron
    // job, none of which this port's call graph can see.
    let uncalled = symbol(
        "script.sh",
        "uncalled",
        SymbolKind::Function,
        1,
        3,
        None,
        1,
        0,
        None,
    );
    let idx = index(vec![file_record_with_language(
        "script.sh",
        Language::Shell,
        vec![uncalled],
        vec![],
    )]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    assert!(findings_for(&report, "uncalled", FindingKind::PossiblyDeadCode).is_empty());
}

#[test]
fn clean_file_scores_max_and_penalties_reduce_score() {
    let clean_fn = symbol(
        "clean.rs",
        "clean",
        SymbolKind::Function,
        1,
        3,
        None,
        1,
        1,
        None,
    );
    let caller = symbol(
        "clean.rs",
        "caller",
        SymbolKind::Function,
        5,
        7,
        None,
        1,
        0,
        None,
    );
    // Mutual calls so neither symbol looks like uncalled dead code.
    let call = CallRef {
        caller: Some(caller.id.clone()),
        callee_name: "clean".to_string(),
        line: 6,
    };
    let call_back = CallRef {
        caller: Some(clean_fn.id.clone()),
        callee_name: "caller".to_string(),
        line: 2,
    };
    let messy_fn = symbol(
        "messy.rs",
        "messy",
        SymbolKind::Function,
        1,
        1 + LONG_FUNCTION_LINES + 1,
        None,
        HIGH_COMPLEXITY + 1,
        TOO_MANY_PARAMS + 1,
        None,
    );
    let messy_caller = symbol(
        "messy.rs",
        "messy_caller",
        SymbolKind::Function,
        200,
        202,
        None,
        1,
        0,
        None,
    );
    let messy_call = CallRef {
        caller: Some(messy_caller.id.clone()),
        callee_name: "messy".to_string(),
        line: 201,
    };

    let idx = index(vec![
        file_record("clean.rs", vec![clean_fn, caller], vec![call, call_back]),
        file_record("messy.rs", vec![messy_fn, messy_caller], vec![messy_call]),
    ]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    let clean_score = report
        .file_scores
        .iter()
        .find(|f| f.file == Path::new("clean.rs"))
        .unwrap();
    let messy_score = report
        .file_scores
        .iter()
        .find(|f| f.file == Path::new("messy.rs"))
        .unwrap();

    assert_eq!(clean_score.score, 10.0);
    assert_eq!(clean_score.finding_count, 0);
    assert!(messy_score.score < 10.0);
    assert!(messy_score.finding_count >= 3);
    // Worst-first ordering.
    assert_eq!(report.file_scores[0].file, Path::new("messy.rs"));
}
