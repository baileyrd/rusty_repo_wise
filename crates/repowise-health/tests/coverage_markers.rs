//! Tests for the coverage-driven health markers (issue #243).
//!
//! The behaviour worth pinning hardest is the *absence* of these markers
//! when there's no coverage data: a repo that never ingested coverage
//! must never be scored as though it were untested.

use repowise_core::coverage::{parse_lcov, CoverageData};
use repowise_core::{FileRecord, ImportRef, Language, RepoIndex};
use repowise_graph::RepoGraph;
use repowise_health::{
    analyze, analyze_with_context, analyze_with_hotspots, FindingKind, HealthWeights,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn file(path: &str, imports: Vec<&str>) -> FileRecord {
    FileRecord {
        path: PathBuf::from(path),
        // Not `Language::Other`: the graph skips import-edge building for
        // it entirely (see repowise-graph's build loop), so dependents
        // would always be zero and `untested_hotspot` could never fire.
        // `resolved_file` is pre-set, so no module-path resolution runs.
        language: Language::Rust,
        lines: 100,
        symbols: Vec::new(),
        imports: imports
            .into_iter()
            .map(|target| ImportRef {
                path: target.to_string(),
                resolved_file: Some(PathBuf::from(target)),
                line: 1,
            })
            .collect(),
        calls: Vec::new(),
        field_accesses: Vec::new(),
    }
}

/// `core.rs` is imported by four other files, making it "centrally
/// depended upon" by `untested_hotspot`'s gate.
fn index_with_four_dependents() -> RepoIndex {
    RepoIndex {
        root: PathBuf::from("/repo"),
        files: vec![
            file("core.rs", vec![]),
            file("a.rs", vec!["core.rs"]),
            file("b.rs", vec!["core.rs"]),
            file("c.rs", vec!["core.rs"]),
            file("d.rs", vec!["core.rs"]),
        ],
        other_files: 0,
        indexed_commit: None,
    }
}

fn hot(paths: &[&str]) -> HashSet<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

/// 1 line hit out of 5 = 20% coverage on core.rs.
fn barely_covered() -> CoverageData {
    parse_lcov("SF:core.rs\nDA:1,1\nDA:2,0\nDA:3,0\nDA:4,0\nDA:5,0\nend_of_record\n").unwrap()
}

fn kinds(report: &repowise_health::HealthReport, kind: FindingKind) -> Vec<&Path> {
    report
        .findings
        .iter()
        .filter(|f| f.kind == kind)
        .map(|f| f.file.as_path())
        .collect()
}

#[test]
fn no_coverage_data_reports_neither_marker() {
    // The single most important property here. Without it, every repo
    // that never ran `coverage add` would be scored as untested.
    let index = index_with_four_dependents();
    let graph = RepoGraph::build(&index);
    let weights = HealthWeights::default();

    for report in [
        analyze(&index, &graph),
        analyze_with_hotspots(&index, &graph, &weights, &hot(&["core.rs"])),
        analyze_with_context(&index, &graph, &weights, &hot(&["core.rs"]), None, None),
    ] {
        assert!(kinds(&report, FindingKind::CoverageGap).is_empty());
        assert!(kinds(&report, FindingKind::UntestedHotspot).is_empty());
    }
}

#[test]
fn untested_hotspot_needs_all_three_signals() {
    let index = index_with_four_dependents();
    let graph = RepoGraph::build(&index);
    let weights = HealthWeights::default();
    let cov = barely_covered();

    // All three present -> fires.
    let all = analyze_with_context(
        &index,
        &graph,
        &weights,
        &hot(&["core.rs"]),
        Some(&cov),
        None,
    );
    assert_eq!(
        kinds(&all, FindingKind::UntestedHotspot),
        vec![Path::new("core.rs")]
    );

    // Not a hotspot -> does not fire (falls back to the milder gap marker).
    let not_hot = analyze_with_context(&index, &graph, &weights, &HashSet::new(), Some(&cov), None);
    assert!(kinds(&not_hot, FindingKind::UntestedHotspot).is_empty());
    assert_eq!(
        kinds(&not_hot, FindingKind::CoverageGap),
        vec![Path::new("core.rs")]
    );

    // Hot and untested, but nothing depends on it -> does not fire.
    let lonely = RepoIndex {
        root: PathBuf::from("/repo"),
        files: vec![file("core.rs", vec![])],
        other_files: 0,
        indexed_commit: None,
    };
    let lonely_graph = RepoGraph::build(&lonely);
    let few_deps = analyze_with_context(
        &lonely,
        &lonely_graph,
        &weights,
        &hot(&["core.rs"]),
        Some(&cov),
        None,
    );
    assert!(kinds(&few_deps, FindingKind::UntestedHotspot).is_empty());

    // Hot and depended-on, but well covered -> does not fire.
    let covered =
        parse_lcov("SF:core.rs\nDA:1,1\nDA:2,1\nDA:3,1\nDA:4,1\nDA:5,1\nend_of_record\n").unwrap();
    let well_tested = analyze_with_context(
        &index,
        &graph,
        &weights,
        &hot(&["core.rs"]),
        Some(&covered),
        None,
    );
    assert!(kinds(&well_tested, FindingKind::UntestedHotspot).is_empty());
    assert!(kinds(&well_tested, FindingKind::CoverageGap).is_empty());
}

#[test]
fn a_file_never_measured_is_skipped_even_when_others_are_covered() {
    // core.rs is measured at 20%; a.rs appears in no report at all and
    // must not be treated as 0%.
    let index = index_with_four_dependents();
    let graph = RepoGraph::build(&index);
    let report = analyze_with_context(
        &index,
        &graph,
        &HealthWeights::default(),
        &HashSet::new(),
        Some(&barely_covered()),
        None,
    );
    let gaps = kinds(&report, FindingKind::CoverageGap);
    assert_eq!(gaps, vec![Path::new("core.rs")]);
    assert!(!gaps.contains(&Path::new("a.rs")));
}

#[test]
fn the_two_markers_do_not_both_charge_the_same_file() {
    // untested_hotspot subsumes coverage_gap; stacking them would
    // double-penalize one underlying fact.
    let index = index_with_four_dependents();
    let graph = RepoGraph::build(&index);
    let report = analyze_with_context(
        &index,
        &graph,
        &HealthWeights::default(),
        &hot(&["core.rs"]),
        Some(&barely_covered()),
        None,
    );
    assert_eq!(
        kinds(&report, FindingKind::UntestedHotspot),
        vec![Path::new("core.rs")]
    );
    assert!(kinds(&report, FindingKind::CoverageGap).is_empty());
}

#[test]
fn a_measured_file_with_zero_percent_still_counts_as_measured() {
    // Some(0.0) is a real finding; None is not. This is the distinction
    // CoverageData::line_coverage_of exists to preserve.
    let index = index_with_four_dependents();
    let graph = RepoGraph::build(&index);
    let zero = parse_lcov("SF:core.rs\nDA:1,0\nDA:2,0\nend_of_record\n").unwrap();
    let report = analyze_with_context(
        &index,
        &graph,
        &HealthWeights::default(),
        &HashSet::new(),
        Some(&zero),
        None,
    );
    assert_eq!(
        kinds(&report, FindingKind::CoverageGap),
        vec![Path::new("core.rs")]
    );
}

#[test]
fn coverage_penalties_are_configurable_like_every_other_marker() {
    let toml = "coverage_gap = 2.5\nuntested_hotspot = 4.0\n";
    let weights = HealthWeights::from_toml_str(toml).unwrap();
    assert_eq!(weights.coverage_gap, 2.5);
    assert_eq!(weights.untested_hotspot, 4.0);
    // An omitted key keeps its documented default.
    let partial = HealthWeights::from_toml_str("coverage_gap = 1.0\n").unwrap();
    assert_eq!(
        partial.untested_hotspot,
        HealthWeights::default().untested_hotspot
    );
}
