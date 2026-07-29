//! Tests for the organizational-signal health markers (issue #313,
//! split from #62). `OrgSignals` is built by hand here, the same way
//! `coverage_markers.rs` builds `CoverageData` directly -- this crate
//! only scores externally-supplied data, so no real git repo is needed
//! to test the thresholds.

use repowise_core::org_signals::OrgSignals;
use repowise_core::{FileRecord, ImportRef, Language, RepoIndex};
use repowise_graph::RepoGraph;
use repowise_health::{
    analyze_with_context, FindingKind, HealthWeights, CHURN_RISK_MIN_CHURN,
    CO_CHANGE_SCATTER_MIN_PARTNERS, DEVELOPER_CONGESTION_MIN_AUTHORS, KNOWLEDGE_LOSS_MIN_CHURN,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn file(path: &str, imports: Vec<&str>) -> FileRecord {
    FileRecord {
        path: PathBuf::from(path),
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

/// A non-code file (like `README.md`) -- never a candidate for a graph
/// edge, unlike `file()`'s `Language::Rust`.
fn doc_file(path: &str) -> FileRecord {
    FileRecord {
        path: PathBuf::from(path),
        language: Language::Other,
        lines: 10,
        symbols: Vec::new(),
        imports: Vec::new(),
        calls: Vec::new(),
        field_accesses: Vec::new(),
    }
}

fn index(files: Vec<FileRecord>) -> RepoIndex {
    RepoIndex {
        root: PathBuf::from("/repo"),
        files,
        other_files: 0,
        indexed_commit: None,
    }
}

fn kinds(report: &repowise_health::HealthReport, kind: FindingKind) -> Vec<&Path> {
    report
        .findings
        .iter()
        .filter(|f| f.kind == kind)
        .map(|f| f.file.as_path())
        .collect()
}

fn analyze(
    idx: &RepoIndex,
    graph: &RepoGraph,
    org: Option<&OrgSignals>,
) -> repowise_health::HealthReport {
    analyze_with_context(
        idx,
        graph,
        &HealthWeights::default(),
        &HashSet::new(),
        None,
        org,
    )
}

#[test]
fn no_org_signals_means_no_markers_at_all() {
    let idx = index(vec![file("a.rs", vec![])]);
    let graph = RepoGraph::build(&idx);

    let report = analyze(&idx, &graph, None);
    for kind in [
        FindingKind::PriorDefect,
        FindingKind::ChurnRisk,
        FindingKind::KnowledgeLoss,
        FindingKind::CoChangeScatter,
        FindingKind::HiddenCoupling,
        FindingKind::DeveloperCongestion,
    ] {
        assert!(
            kinds(&report, kind).is_empty(),
            "{kind:?} must not fire without OrgSignals"
        );
    }
}

#[test]
fn prior_defect_fires_on_any_bugfix_commit() {
    let idx = index(vec![file("a.rs", vec![]), file("b.rs", vec![])]);
    let graph = RepoGraph::build(&idx);
    let mut org = OrgSignals::default();
    org.bugfix_commits.insert(PathBuf::from("a.rs"), 1);

    let report = analyze(&idx, &graph, Some(&org));
    assert_eq!(
        kinds(&report, FindingKind::PriorDefect),
        vec![Path::new("a.rs")]
    );
}

#[test]
fn churn_risk_needs_the_threshold_met() {
    let idx = index(vec![file("a.rs", vec![]), file("b.rs", vec![])]);
    let graph = RepoGraph::build(&idx);
    let mut org = OrgSignals::default();
    org.churn
        .insert(PathBuf::from("a.rs"), CHURN_RISK_MIN_CHURN);
    org.churn
        .insert(PathBuf::from("b.rs"), CHURN_RISK_MIN_CHURN - 1);

    let report = analyze(&idx, &graph, Some(&org));
    assert_eq!(
        kinds(&report, FindingKind::ChurnRisk),
        vec![Path::new("a.rs")]
    );
}

#[test]
fn knowledge_loss_needs_both_enough_churn_and_a_low_bus_factor() {
    let idx = index(vec![
        file("low_churn.rs", vec![]),
        file("high_bus_factor.rs", vec![]),
        file("real_risk.rs", vec![]),
    ]);
    let graph = RepoGraph::build(&idx);
    let mut org = OrgSignals::default();
    // Low churn: bus factor 1 here is meaningless (barely touched).
    org.churn
        .insert(PathBuf::from("low_churn.rs"), KNOWLEDGE_LOSS_MIN_CHURN - 1);
    org.bus_factor.insert(PathBuf::from("low_churn.rs"), 1);
    // Plenty of churn, but ownership is spread out.
    org.churn.insert(
        PathBuf::from("high_bus_factor.rs"),
        KNOWLEDGE_LOSS_MIN_CHURN,
    );
    org.bus_factor
        .insert(PathBuf::from("high_bus_factor.rs"), 3);
    // Both conditions met.
    org.churn
        .insert(PathBuf::from("real_risk.rs"), KNOWLEDGE_LOSS_MIN_CHURN);
    org.bus_factor.insert(PathBuf::from("real_risk.rs"), 1);

    let report = analyze(&idx, &graph, Some(&org));
    assert_eq!(
        kinds(&report, FindingKind::KnowledgeLoss),
        vec![Path::new("real_risk.rs")]
    );
}

/// Bus factor `0` means "no blameable history" (see
/// `OrgSignals::bus_factor_of`'s own doc), not a bus factor of zero
/// authors -- it must not be treated as maximally concentrated
/// ownership.
#[test]
fn a_bus_factor_of_zero_is_no_data_not_a_risk() {
    let idx = index(vec![file("a.rs", vec![])]);
    let graph = RepoGraph::build(&idx);
    let mut org = OrgSignals::default();
    org.churn
        .insert(PathBuf::from("a.rs"), KNOWLEDGE_LOSS_MIN_CHURN);
    // bus_factor left unset -> bus_factor_of returns 0.

    let report = analyze(&idx, &graph, Some(&org));
    assert!(kinds(&report, FindingKind::KnowledgeLoss).is_empty());
}

#[test]
fn co_change_scatter_needs_the_partner_count_threshold() {
    let idx = index(vec![file("a.rs", vec![]), file("b.rs", vec![])]);
    let graph = RepoGraph::build(&idx);
    let mut org = OrgSignals::default();
    org.co_change_partner_count
        .insert(PathBuf::from("a.rs"), CO_CHANGE_SCATTER_MIN_PARTNERS);
    org.co_change_partner_count
        .insert(PathBuf::from("b.rs"), CO_CHANGE_SCATTER_MIN_PARTNERS - 1);

    let report = analyze(&idx, &graph, Some(&org));
    assert_eq!(
        kinds(&report, FindingKind::CoChangeScatter),
        vec![Path::new("a.rs")]
    );
}

#[test]
fn developer_congestion_needs_more_than_the_threshold() {
    let idx = index(vec![file("a.rs", vec![]), file("b.rs", vec![])]);
    let graph = RepoGraph::build(&idx);
    let mut org = OrgSignals::default();
    org.recent_author_count
        .insert(PathBuf::from("a.rs"), DEVELOPER_CONGESTION_MIN_AUTHORS + 1);
    org.recent_author_count
        .insert(PathBuf::from("b.rs"), DEVELOPER_CONGESTION_MIN_AUTHORS);

    let report = analyze(&idx, &graph, Some(&org));
    assert_eq!(
        kinds(&report, FindingKind::DeveloperCongestion),
        vec![Path::new("a.rs")]
    );
}

/// The core of `hidden_coupling`: a pair that co-changes often in git
/// history but has no import/call edge in the dependency graph.
#[test]
fn hidden_coupling_fires_only_without_a_graph_edge() {
    let idx = index(vec![
        // a.rs/b.rs co-change but never import each other.
        file("a.rs", vec![]),
        file("b.rs", vec![]),
        // c.rs/d.rs also co-change, but c.rs actually imports d.rs --
        // the graph already explains this coupling, so it must not be
        // "hidden".
        file("c.rs", vec!["d.rs"]),
        file("d.rs", vec![]),
    ]);
    let graph = RepoGraph::build(&idx);
    let org = OrgSignals {
        co_changed_pairs: vec![
            (PathBuf::from("a.rs"), PathBuf::from("b.rs"), 5),
            (PathBuf::from("c.rs"), PathBuf::from("d.rs"), 5),
        ],
        ..Default::default()
    };

    let report = analyze(&idx, &graph, Some(&org));
    let mut hidden = kinds(&report, FindingKind::HiddenCoupling);
    hidden.sort();
    // Reported for both sides of the a/b pair, so either file's score
    // reflects it.
    assert_eq!(hidden, vec![Path::new("a.rs"), Path::new("b.rs")]);
}

/// Found by running `repowise health` against this port's own workspace:
/// `README.md` co-changed with a source file 31 times and fired
/// `hidden_coupling`, which is the wrong read entirely -- a
/// documentation file was never a candidate for a graph edge, so the
/// graph's silence about it means nothing. Every org-signal marker must
/// skip `Language::Other` files, not just `hidden_coupling`.
#[test]
fn a_documentation_file_never_triggers_any_org_signal_marker() {
    let idx = index(vec![doc_file("README.md"), file("a.rs", vec![])]);
    let graph = RepoGraph::build(&idx);
    let org = OrgSignals {
        churn: [(PathBuf::from("README.md"), 100)].into_iter().collect(),
        bugfix_commits: [(PathBuf::from("README.md"), 5)].into_iter().collect(),
        bus_factor: [(PathBuf::from("README.md"), 1)].into_iter().collect(),
        co_change_partner_count: [(PathBuf::from("README.md"), 50)].into_iter().collect(),
        recent_author_count: [(PathBuf::from("README.md"), 10)].into_iter().collect(),
        co_changed_pairs: vec![(PathBuf::from("README.md"), PathBuf::from("a.rs"), 31)],
    };

    let report = analyze(&idx, &graph, Some(&org));
    assert!(
        report
            .findings
            .iter()
            .all(|f| f.file != Path::new("README.md")),
        "a documentation file must never carry an org-signal finding: {:?}",
        report.findings
    );
    // The a.rs side of the pair must not fire either -- the pair as a
    // whole is out of this marker's domain, not just README.md's half.
    assert!(kinds(&report, FindingKind::HiddenCoupling).is_empty());
}

#[test]
fn org_signal_weights_are_configurable_like_every_other_marker() {
    let toml = "prior_defect = 1.2\nhidden_coupling = 2.0\n";
    let weights = HealthWeights::from_toml_str(toml).unwrap();
    assert_eq!(weights.prior_defect, 1.2);
    assert_eq!(weights.hidden_coupling, 2.0);
    // Omitted keys keep their documented defaults.
    assert_eq!(weights.churn_risk, HealthWeights::default().churn_risk);
}
