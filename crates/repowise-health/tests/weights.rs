//! Tests for the `HealthWeights` abstraction: defaults match this
//! crate's original hand-picked penalties, a partial TOML file only
//! overrides the keys it names, and `analyze_with_weights` actually
//! uses the weights it's given rather than silently falling back to
//! the defaults.

use repowise_core::{FileRecord, Language, RepoIndex, Symbol, SymbolKind};
use repowise_graph::RepoGraph;
use repowise_health::{analyze_with_weights, FindingKind, HealthWeights, HIGH_COMPLEXITY};
use std::path::PathBuf;

fn symbol(name: &str, complexity: usize) -> Symbol {
    let file = PathBuf::from("a.rs");
    Symbol {
        id: Symbol::make_id(&file, name, 1),
        name: name.to_string(),
        kind: SymbolKind::Function,
        file,
        start_line: 1,
        end_line: 5,
        parent: None,
        complexity,
        max_nesting_depth: 0,
        bumpy_road_bumps: 0,
        complex_conditionals: Vec::new(),
        param_count: 0,
        primitive_param_count: 0,
        body_hash: None,
    }
}

fn index_with(symbols: Vec<Symbol>) -> RepoIndex {
    RepoIndex {
        root: PathBuf::from("/fixture"),
        files: vec![FileRecord {
            path: PathBuf::from("a.rs"),
            language: Language::Other,
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
fn default_weights_match_this_crates_original_hardcoded_penalties() {
    let defaults = HealthWeights::default();
    assert_eq!(defaults.long_function, 0.5);
    assert_eq!(defaults.high_complexity, 1.0);
    assert_eq!(defaults.too_many_params, 0.3);
    assert_eq!(defaults.god_class, 1.5);
    assert_eq!(defaults.duplicate_code, 0.5);
    assert_eq!(defaults.near_duplicate_code, 0.3);
    assert_eq!(defaults.possibly_dead_code, 0.2);
    assert_eq!(defaults.low_cohesion, 1.0);
    assert_eq!(defaults.nested_complexity, 1.0);
    assert_eq!(defaults.bumpy_road, 0.5);
    assert_eq!(defaults.complex_conditional, 0.3);
    assert_eq!(defaults.primitive_obsession, 0.3);
}

#[test]
fn from_toml_str_overrides_only_the_keys_it_names() {
    let weights = HealthWeights::from_toml_str("high_complexity = 4.0\n").unwrap();

    assert_eq!(weights.high_complexity, 4.0);
    // Every other key falls back to its documented default.
    assert_eq!(
        weights.long_function,
        HealthWeights::default().long_function
    );
    assert_eq!(weights.god_class, HealthWeights::default().god_class);
}

#[test]
fn from_toml_str_with_no_keys_is_identical_to_default() {
    let weights = HealthWeights::from_toml_str("").unwrap();
    assert_eq!(weights, HealthWeights::default());
}

#[test]
fn from_toml_str_rejects_malformed_toml() {
    assert!(HealthWeights::from_toml_str("not = [valid").is_err());
}

#[test]
fn analyze_with_weights_uses_the_supplied_penalty_not_the_default() {
    let index = index_with(vec![symbol("busy", HIGH_COMPLEXITY + 1)]);
    let graph = RepoGraph::build(&index);

    let custom = HealthWeights {
        high_complexity: 4.0,
        ..HealthWeights::default()
    };
    let report = analyze_with_weights(&index, &graph, &custom);

    let finding = report
        .findings
        .iter()
        .find(|f| f.kind == FindingKind::HighComplexity)
        .expect("expected a HighComplexity finding");
    let file_score = report
        .file_scores
        .iter()
        .find(|f| f.file == finding.file)
        .unwrap();

    // Also triggers PossiblyDeadCode (no callers), so the file score is
    // 10.0 - 4.0 (custom high_complexity) - 0.2 (default dead_code) = 5.8,
    // not the 8.5 a default-weighted run would produce.
    assert_eq!(file_score.score, 5.8);
}
