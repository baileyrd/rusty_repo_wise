//! Tests build `RepoIndex`/`FileRecord`/`Symbol` fixtures directly (no
//! parsing needed) so each marker can be exercised in isolation. Files use
//! `Language::Other` to skip repowise-graph's Rust/Python module-path
//! resolution, which isn't relevant here and would otherwise touch disk.

use repowise_core::{
    CallRef, ComplexConditionalRef, FileRecord, IoInLoopRef, JsonParseInLoopRef, Language,
    ListInsertZeroInLoopRef, LockInLoopRef, NestedLoopWithIoRef, RegexCompileInLoopRef, RepoIndex,
    ResourceConstructionInLoopRef, StringConcatInLoopRef, Symbol, SymbolKind,
};
use repowise_graph::RepoGraph;
use repowise_health::{
    analyze, FindingKind, BUMPY_ROAD_MIN_BUMPS, GOD_CLASS_METHODS, HIGH_COMPLEXITY,
    HIGH_NESTING_DEPTH, LONG_FUNCTION_LINES, PRIMITIVE_OBSESSION_MIN_COUNT, TOO_MANY_PARAMS,
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
        max_nesting_depth: 0,
        bumpy_road_bumps: 0,
        complex_conditionals: Vec::new(),
        io_in_loop: Vec::new(),
        string_concat_in_loop: Vec::new(),
        resource_construction_in_loop: Vec::new(),
        lock_in_loop: Vec::new(),
        list_insert_zero_in_loop: Vec::new(),
        json_parse_in_loop: Vec::new(),
        regex_compile_in_loop: Vec::new(),
        nested_loop_with_io: Vec::new(),
        param_count,
        primitive_param_count: 0,
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
fn flags_deeply_nested_functions_but_not_shallow_ones() {
    let mut deep = symbol(
        "deep.rs",
        "deep",
        SymbolKind::Function,
        1,
        20,
        None,
        5,
        1,
        None,
    );
    deep.max_nesting_depth = HIGH_NESTING_DEPTH + 2;
    let mut shallow = symbol(
        "deep.rs",
        "shallow",
        SymbolKind::Function,
        22,
        30,
        None,
        3,
        1,
        None,
    );
    shallow.max_nesting_depth = HIGH_NESTING_DEPTH;

    let idx = index(vec![file_record(
        "deep.rs",
        vec![deep, shallow],
        Vec::new(),
    )]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    assert_eq!(
        findings_for(&report, "deep", FindingKind::NestedComplexity).len(),
        1
    );
    // At the threshold, not above it -- not flagged.
    assert!(findings_for(&report, "shallow", FindingKind::NestedComplexity).is_empty());
}

#[test]
fn flags_bumpy_functions_but_not_ones_below_the_bump_threshold() {
    let mut bumpy = symbol(
        "bumpy.rs",
        "bumpy",
        SymbolKind::Function,
        1,
        20,
        None,
        5,
        1,
        None,
    );
    bumpy.bumpy_road_bumps = BUMPY_ROAD_MIN_BUMPS;
    let mut smooth = symbol(
        "bumpy.rs",
        "smooth",
        SymbolKind::Function,
        22,
        30,
        None,
        3,
        1,
        None,
    );
    smooth.bumpy_road_bumps = BUMPY_ROAD_MIN_BUMPS - 1;

    let idx = index(vec![file_record(
        "bumpy.rs",
        vec![bumpy, smooth],
        Vec::new(),
    )]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    assert_eq!(
        findings_for(&report, "bumpy", FindingKind::BumpyRoad).len(),
        1
    );
    assert!(findings_for(&report, "smooth", FindingKind::BumpyRoad).is_empty());
}

#[test]
fn flags_one_finding_per_complex_conditional_pointing_at_its_own_line() {
    let mut tangled = symbol(
        "tangled.rs",
        "tangled",
        SymbolKind::Function,
        1,
        10,
        None,
        4,
        4,
        None,
    );
    tangled.complex_conditionals = vec![
        ComplexConditionalRef {
            line: 3,
            operator_count: 3,
        },
        ComplexConditionalRef {
            line: 7,
            operator_count: 4,
        },
    ];
    let simple = symbol(
        "tangled.rs",
        "simple",
        SymbolKind::Function,
        12,
        16,
        None,
        1,
        1,
        None,
    );

    let idx = index(vec![file_record(
        "tangled.rs",
        vec![tangled, simple],
        Vec::new(),
    )]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    let findings = findings_for(&report, "tangled", FindingKind::ComplexConditional);
    assert_eq!(findings.len(), 2);
    let lines: Vec<Option<usize>> = findings.iter().map(|f| f.line).collect();
    assert!(lines.contains(&Some(3)));
    assert!(lines.contains(&Some(7)));
    assert!(findings_for(&report, "simple", FindingKind::ComplexConditional).is_empty());
}

#[test]
fn flags_primitive_obsession_at_the_documented_threshold() {
    let mut obsessed = symbol(
        "obsessed.rs",
        "obsessed",
        SymbolKind::Function,
        1,
        5,
        None,
        1,
        4,
        None,
    );
    obsessed.primitive_param_count = PRIMITIVE_OBSESSION_MIN_COUNT;
    let mut domain_typed = symbol(
        "obsessed.rs",
        "domain_typed",
        SymbolKind::Function,
        7,
        11,
        None,
        1,
        2,
        None,
    );
    domain_typed.primitive_param_count = PRIMITIVE_OBSESSION_MIN_COUNT - 1;

    let idx = index(vec![file_record(
        "obsessed.rs",
        vec![obsessed, domain_typed],
        Vec::new(),
    )]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    assert_eq!(
        findings_for(&report, "obsessed", FindingKind::PrimitiveObsession).len(),
        1
    );
    assert!(findings_for(&report, "domain_typed", FindingKind::PrimitiveObsession).is_empty());
}

#[test]
fn flags_one_finding_per_io_in_loop_call_pointing_at_its_own_line() {
    let mut looped = symbol(
        "looped.rs",
        "looped",
        SymbolKind::Function,
        1,
        10,
        None,
        2,
        1,
        None,
    );
    looped.io_in_loop = vec![
        IoInLoopRef {
            line: 4,
            callee_name: "read_to_string".to_string(),
        },
        IoInLoopRef {
            line: 6,
            callee_name: "execute".to_string(),
        },
    ];
    let clean = symbol(
        "looped.rs",
        "clean",
        SymbolKind::Function,
        12,
        16,
        None,
        1,
        1,
        None,
    );

    let idx = index(vec![file_record(
        "looped.rs",
        vec![looped, clean],
        Vec::new(),
    )]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    let findings = findings_for(&report, "looped", FindingKind::IoInLoop);
    assert_eq!(findings.len(), 2);
    let lines: Vec<Option<usize>> = findings.iter().map(|f| f.line).collect();
    assert!(lines.contains(&Some(4)));
    assert!(lines.contains(&Some(6)));
    assert!(findings_for(&report, "clean", FindingKind::IoInLoop).is_empty());
}

#[test]
fn flags_one_finding_per_string_concat_in_loop_pointing_at_its_own_line() {
    let mut looped = symbol(
        "concat.rs",
        "looped",
        SymbolKind::Function,
        1,
        10,
        None,
        2,
        1,
        None,
    );
    looped.string_concat_in_loop = vec![
        StringConcatInLoopRef {
            line: 4,
            variable: "s".to_string(),
        },
        StringConcatInLoopRef {
            line: 6,
            variable: "acc".to_string(),
        },
    ];
    let clean = symbol(
        "concat.rs",
        "clean",
        SymbolKind::Function,
        12,
        16,
        None,
        1,
        1,
        None,
    );

    let idx = index(vec![file_record(
        "concat.rs",
        vec![looped, clean],
        Vec::new(),
    )]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    let findings = findings_for(&report, "looped", FindingKind::StringConcatInLoop);
    assert_eq!(findings.len(), 2);
    let lines: Vec<Option<usize>> = findings.iter().map(|f| f.line).collect();
    assert!(lines.contains(&Some(4)));
    assert!(lines.contains(&Some(6)));
    assert!(findings_for(&report, "clean", FindingKind::StringConcatInLoop).is_empty());
}

#[test]
fn flags_one_finding_per_resource_construction_in_loop_pointing_at_its_own_line() {
    let mut looped = symbol(
        "resource.rs",
        "looped",
        SymbolKind::Function,
        1,
        10,
        None,
        2,
        1,
        None,
    );
    looped.resource_construction_in_loop = vec![
        ResourceConstructionInLoopRef {
            line: 4,
            callee_name: "HttpClient::new".to_string(),
        },
        ResourceConstructionInLoopRef {
            line: 6,
            callee_name: "ThreadPool::new".to_string(),
        },
    ];
    let clean = symbol(
        "resource.rs",
        "clean",
        SymbolKind::Function,
        12,
        16,
        None,
        1,
        1,
        None,
    );

    let idx = index(vec![file_record(
        "resource.rs",
        vec![looped, clean],
        Vec::new(),
    )]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    let findings = findings_for(&report, "looped", FindingKind::ResourceConstructionInLoop);
    assert_eq!(findings.len(), 2);
    let lines: Vec<Option<usize>> = findings.iter().map(|f| f.line).collect();
    assert!(lines.contains(&Some(4)));
    assert!(lines.contains(&Some(6)));
    assert!(findings_for(&report, "clean", FindingKind::ResourceConstructionInLoop).is_empty());
}

#[test]
fn flags_one_finding_per_lock_in_loop_pointing_at_its_own_line() {
    let mut looped = symbol(
        "lock.rs",
        "looped",
        SymbolKind::Function,
        1,
        10,
        None,
        2,
        1,
        None,
    );
    looped.lock_in_loop = vec![
        LockInLoopRef {
            line: 4,
            callee_name: "lock".to_string(),
        },
        LockInLoopRef {
            line: 6,
            callee_name: "try_lock".to_string(),
        },
    ];
    let clean = symbol(
        "lock.rs",
        "clean",
        SymbolKind::Function,
        12,
        16,
        None,
        1,
        1,
        None,
    );

    let idx = index(vec![file_record(
        "lock.rs",
        vec![looped, clean],
        Vec::new(),
    )]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    let findings = findings_for(&report, "looped", FindingKind::LockInLoop);
    assert_eq!(findings.len(), 2);
    let lines: Vec<Option<usize>> = findings.iter().map(|f| f.line).collect();
    assert!(lines.contains(&Some(4)));
    assert!(lines.contains(&Some(6)));
    assert!(findings_for(&report, "clean", FindingKind::LockInLoop).is_empty());
}

#[test]
fn flags_one_finding_per_list_insert_zero_in_loop_pointing_at_its_own_line() {
    let mut looped = symbol(
        "insert.rs",
        "looped",
        SymbolKind::Function,
        1,
        10,
        None,
        2,
        1,
        None,
    );
    looped.list_insert_zero_in_loop = vec![
        ListInsertZeroInLoopRef {
            line: 4,
            variable: "out".to_string(),
        },
        ListInsertZeroInLoopRef {
            line: 6,
            variable: "acc".to_string(),
        },
    ];
    let clean = symbol(
        "insert.rs",
        "clean",
        SymbolKind::Function,
        12,
        16,
        None,
        1,
        1,
        None,
    );

    let idx = index(vec![file_record(
        "insert.rs",
        vec![looped, clean],
        Vec::new(),
    )]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    let findings = findings_for(&report, "looped", FindingKind::ListInsertZeroInLoop);
    assert_eq!(findings.len(), 2);
    let lines: Vec<Option<usize>> = findings.iter().map(|f| f.line).collect();
    assert!(lines.contains(&Some(4)));
    assert!(lines.contains(&Some(6)));
    assert!(findings_for(&report, "clean", FindingKind::ListInsertZeroInLoop).is_empty());
}

#[test]
fn flags_one_finding_per_json_parse_in_loop_pointing_at_its_own_line() {
    let mut looped = symbol(
        "parse.rs",
        "looped",
        SymbolKind::Function,
        1,
        10,
        None,
        2,
        1,
        None,
    );
    looped.json_parse_in_loop = vec![
        JsonParseInLoopRef {
            line: 4,
            callee_name: "serde_json::from_str".to_string(),
        },
        JsonParseInLoopRef {
            line: 6,
            callee_name: "serde_json::from_slice".to_string(),
        },
    ];
    let clean = symbol(
        "parse.rs",
        "clean",
        SymbolKind::Function,
        12,
        16,
        None,
        1,
        1,
        None,
    );

    let idx = index(vec![file_record(
        "parse.rs",
        vec![looped, clean],
        Vec::new(),
    )]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    let findings = findings_for(&report, "looped", FindingKind::JsonParseInLoop);
    assert_eq!(findings.len(), 2);
    let lines: Vec<Option<usize>> = findings.iter().map(|f| f.line).collect();
    assert!(lines.contains(&Some(4)));
    assert!(lines.contains(&Some(6)));
    assert!(findings_for(&report, "clean", FindingKind::JsonParseInLoop).is_empty());
}

#[test]
fn flags_one_finding_per_regex_compile_in_loop_pointing_at_its_own_line() {
    let mut looped = symbol(
        "regex.rs",
        "looped",
        SymbolKind::Function,
        1,
        10,
        None,
        2,
        1,
        None,
    );
    looped.regex_compile_in_loop = vec![
        RegexCompileInLoopRef {
            line: 4,
            callee_name: "Regex::new".to_string(),
        },
        RegexCompileInLoopRef {
            line: 6,
            callee_name: "Regex::new".to_string(),
        },
    ];
    let clean = symbol(
        "regex.rs",
        "clean",
        SymbolKind::Function,
        12,
        16,
        None,
        1,
        1,
        None,
    );

    let idx = index(vec![file_record(
        "regex.rs",
        vec![looped, clean],
        Vec::new(),
    )]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    let findings = findings_for(&report, "looped", FindingKind::RegexCompileInLoop);
    assert_eq!(findings.len(), 2);
    let lines: Vec<Option<usize>> = findings.iter().map(|f| f.line).collect();
    assert!(lines.contains(&Some(4)));
    assert!(lines.contains(&Some(6)));
    assert!(findings_for(&report, "clean", FindingKind::RegexCompileInLoop).is_empty());
}

#[test]
fn flags_one_finding_per_nested_loop_with_io_pointing_at_its_own_line() {
    let mut nested = symbol(
        "nested.rs",
        "nested",
        SymbolKind::Function,
        1,
        10,
        None,
        2,
        1,
        None,
    );
    nested.nested_loop_with_io = vec![
        NestedLoopWithIoRef {
            line: 4,
            callee_name: "read_to_string".to_string(),
        },
        NestedLoopWithIoRef {
            line: 6,
            callee_name: "write_all".to_string(),
        },
    ];
    let clean = symbol(
        "nested.rs",
        "clean",
        SymbolKind::Function,
        12,
        16,
        None,
        1,
        1,
        None,
    );

    let idx = index(vec![file_record(
        "nested.rs",
        vec![nested, clean],
        Vec::new(),
    )]);
    let graph = RepoGraph::build(&idx);
    let report = analyze(&idx, &graph);

    let findings = findings_for(&report, "nested", FindingKind::NestedLoopWithIo);
    assert_eq!(findings.len(), 2);
    let lines: Vec<Option<usize>> = findings.iter().map(|f| f.line).collect();
    assert!(lines.contains(&Some(4)));
    assert!(lines.contains(&Some(6)));
    assert!(findings_for(&report, "clean", FindingKind::NestedLoopWithIo).is_empty());
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
