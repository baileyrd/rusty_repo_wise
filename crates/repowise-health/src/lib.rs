//! Deterministic, rule-based code-health scoring — no LLM/ML involved.
//!
//! This implements a focused subset of repowise's "25 deterministic
//! markers": long functions, high cyclomatic complexity, oversized
//! parameter lists, god classes (too many methods), duplicate code
//! (identical function/method bodies), near-duplicate code (Rabin-Karp
//! rolling-hash overlap — see the `near_duplicate` module doc comment),
//! possibly-dead code (zero resolved callers), low cohesion (LCOM4 —
//! see the `lcom4` module doc comment for its Rust/Python/TS+JS-only
//! scope), nested complexity (max control-flow nesting depth — see
//! `repowise_core::Symbol::max_nesting_depth`), a "bumpy road" (count of
//! distinct nested-block regions — see
//! `repowise_core::Symbol::bumpy_road_bumps`), and complex conditionals
//! (per-condition boolean-operator chains, Rust/Python/TS+JS-only — see
//! `repowise_core::Symbol::complex_conditionals`), and primitive obsession
//! (parameter lists leaning on bare primitives instead of domain types,
//! Rust/TypeScript-only since it needs declared parameter types — see
//! `repowise_core::Symbol::primitive_param_count`). Git-history-based
//! markers (churn, hotspots, bug-fix history) aren't implemented yet —
//! that needs the git-analytics layer, which is a separate phase.
//!
//! Every marker here is a plain threshold over data `repowise-parser`/
//! `repowise-graph` already computed; nothing is inferred or guessed.
//! The one exception is `near_duplicate`, which re-reads source text
//! fresh from disk since `Symbol` doesn't carry raw body text — see its
//! own module doc comment for why that's still consistent with this
//! crate's usual "no I/O" shape rather than a quiet exception to it.

mod dead_code;
mod lcom4;
mod near_duplicate;

pub use dead_code::{find_dead_code, DeadCodeCandidate, DeadCodeConfidence};
pub use lcom4::{find_low_cohesion, LowCohesionCandidate, LOW_COHESION_MIN_COMPONENTS};
pub use near_duplicate::{find_near_duplicates, NearDuplicateCandidate};

use repowise_core::{Language, RepoIndex, Symbol, SymbolKind};
use repowise_graph::RepoGraph;
use std::collections::HashMap;
use std::path::PathBuf;

/// A function/method longer than this (in lines) is flagged.
pub const LONG_FUNCTION_LINES: usize = 50;
/// A function/method with cyclomatic complexity above this is flagged.
pub const HIGH_COMPLEXITY: usize = 10;
/// A function/method with more than this many parameters is flagged.
pub const TOO_MANY_PARAMS: usize = 6;
/// A struct/class with more than this many methods is flagged ("god class").
pub const GOD_CLASS_METHODS: usize = 15;
/// A function/method with control-flow nested deeper than this is flagged.
/// E.g. an `if` inside a `for` inside an `if` is depth 3.
pub const HIGH_NESTING_DEPTH: usize = 4;
/// A function/method with at least this many "bumpy road" nested-block
/// regions (see `repowise_core::Symbol::bumpy_road_bumps`) is flagged.
pub const BUMPY_ROAD_MIN_BUMPS: usize = 3;
/// A function/method with at least this many bare-primitive-typed
/// parameters (see `repowise_core::Symbol::primitive_param_count`) is
/// flagged.
pub const PRIMITIVE_OBSESSION_MIN_COUNT: usize = 3;

const PENALTY_LONG_FUNCTION: f64 = 0.5;
const PENALTY_HIGH_COMPLEXITY: f64 = 1.0;
const PENALTY_TOO_MANY_PARAMS: f64 = 0.3;
const PENALTY_GOD_CLASS: f64 = 1.5;
const PENALTY_DUPLICATE: f64 = 0.5;
const PENALTY_DEAD_CODE: f64 = 0.2;
const PENALTY_LOW_COHESION: f64 = 1.0;
// Weaker signal than an exact-hash `DuplicateCode` match (a heuristic
// overlap ratio, not a byte-for-byte match), so it's penalized less.
const PENALTY_NEAR_DUPLICATE: f64 = 0.3;
// Same weight as `HighComplexity`: both are cheap AST-derived structural
// signals of the same rough severity, just measuring different things
// (branch count vs. nesting depth).
const PENALTY_NESTED_COMPLEXITY: f64 = 1.0;
// Lighter than `NestedComplexity`: a complementary signal on the same
// underlying data (scattered nesting vs. a single deep point), not an
// independent problem worth double-weighting.
const PENALTY_BUMPY_ROAD: f64 = 0.5;
// A function can rack up multiple flagged conditions at once; a
// per-occurrence weight lighter than the whole-function markers above
// avoids one messy function alone tanking its score.
const PENALTY_COMPLEX_CONDITIONAL: f64 = 0.3;
// Same weight as `TooManyParameters`/`ComplexConditional`: another
// parameter-list-shaped structural-complexity signal, not a central-logic
// problem worth a heavier penalty.
const PENALTY_PRIMITIVE_OBSESSION: f64 = 0.3;

const MAX_SCORE: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    LongFunction,
    HighComplexity,
    TooManyParameters,
    GodClass,
    DuplicateCode,
    NearDuplicateCode,
    PossiblyDeadCode,
    LowCohesion,
    NestedComplexity,
    BumpyRoad,
    ComplexConditional,
    PrimitiveObsession,
}

impl FindingKind {
    pub fn label(&self) -> &'static str {
        match self {
            FindingKind::LongFunction => "long-function",
            FindingKind::HighComplexity => "high-complexity",
            FindingKind::TooManyParameters => "too-many-params",
            FindingKind::GodClass => "god-class",
            FindingKind::DuplicateCode => "duplicate-code",
            FindingKind::NearDuplicateCode => "near-duplicate-code",
            FindingKind::PossiblyDeadCode => "possibly-dead-code",
            FindingKind::LowCohesion => "low-cohesion",
            FindingKind::NestedComplexity => "nested-complexity",
            FindingKind::BumpyRoad => "bumpy-road",
            FindingKind::ComplexConditional => "complex-conditional",
            FindingKind::PrimitiveObsession => "primitive-obsession",
        }
    }

    fn penalty(&self) -> f64 {
        match self {
            FindingKind::LongFunction => PENALTY_LONG_FUNCTION,
            FindingKind::HighComplexity => PENALTY_HIGH_COMPLEXITY,
            FindingKind::TooManyParameters => PENALTY_TOO_MANY_PARAMS,
            FindingKind::GodClass => PENALTY_GOD_CLASS,
            FindingKind::DuplicateCode => PENALTY_DUPLICATE,
            FindingKind::NearDuplicateCode => PENALTY_NEAR_DUPLICATE,
            FindingKind::PossiblyDeadCode => PENALTY_DEAD_CODE,
            FindingKind::LowCohesion => PENALTY_LOW_COHESION,
            FindingKind::NestedComplexity => PENALTY_NESTED_COMPLEXITY,
            FindingKind::BumpyRoad => PENALTY_BUMPY_ROAD,
            FindingKind::ComplexConditional => PENALTY_COMPLEX_CONDITIONAL,
            FindingKind::PrimitiveObsession => PENALTY_PRIMITIVE_OBSESSION,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub file: PathBuf,
    pub symbol: Option<String>,
    pub line: Option<usize>,
    pub kind: FindingKind,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct FileHealth {
    pub file: PathBuf,
    /// 0.0 (unhealthy) to 10.0 (no markers triggered).
    pub score: f64,
    pub finding_count: usize,
}

pub struct HealthReport {
    /// One entry per indexed file, sorted worst-score-first.
    pub file_scores: Vec<FileHealth>,
    pub findings: Vec<Finding>,
    pub average_score: f64,
}

impl HealthReport {
    pub fn findings_by_kind(&self) -> Vec<(FindingKind, usize)> {
        let mut counts: HashMap<&'static str, (FindingKind, usize)> = HashMap::new();
        for f in &self.findings {
            counts.entry(f.kind.label()).or_insert((f.kind, 0)).1 += 1;
        }
        let mut out: Vec<(FindingKind, usize)> = counts.into_values().collect();
        out.sort_by_key(|b| std::cmp::Reverse(b.1));
        out
    }
}

pub fn analyze(index: &RepoIndex, graph: &RepoGraph) -> HealthReport {
    let mut findings = Vec::new();

    for file in &index.files {
        // Per repowise's own documented scope, shell scripts get a
        // narrower marker set than Full/Good-tier languages: no
        // dead-code detection (a shell function is routinely invoked
        // only from the command line, another script, or a cron job —
        // none of which this port's call graph can see, making the
        // signal too unreliable to report).
        let skip_dead_code = file.language == Language::Shell;
        for sym in &file.symbols {
            if !matches!(sym.kind, SymbolKind::Function | SymbolKind::Method) {
                continue;
            }
            check_function_markers(sym, graph, skip_dead_code, &mut findings);
        }
    }

    check_god_classes(index, &mut findings);
    check_duplicate_code(index, &mut findings);
    check_near_duplicate_code(index, &mut findings);
    check_low_cohesion(index, &mut findings);

    let file_scores = score_files(index, &findings);
    let average_score = if file_scores.is_empty() {
        MAX_SCORE
    } else {
        file_scores.iter().map(|f| f.score).sum::<f64>() / file_scores.len() as f64
    };

    HealthReport {
        file_scores,
        findings,
        average_score,
    }
}

fn check_function_markers(
    sym: &Symbol,
    graph: &RepoGraph,
    skip_dead_code: bool,
    findings: &mut Vec<Finding>,
) {
    let length = sym.end_line.saturating_sub(sym.start_line) + 1;
    if length > LONG_FUNCTION_LINES {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::LongFunction,
            detail: format!("{length} lines (> {LONG_FUNCTION_LINES})"),
        });
    }
    if sym.complexity > HIGH_COMPLEXITY {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::HighComplexity,
            detail: format!(
                "cyclomatic complexity {} (> {HIGH_COMPLEXITY})",
                sym.complexity
            ),
        });
    }
    if sym.max_nesting_depth > HIGH_NESTING_DEPTH {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::NestedComplexity,
            detail: format!(
                "control flow nested {} levels deep (> {HIGH_NESTING_DEPTH})",
                sym.max_nesting_depth
            ),
        });
    }
    if sym.bumpy_road_bumps >= BUMPY_ROAD_MIN_BUMPS {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::BumpyRoad,
            detail: format!(
                "{} separate nested-block regions (>= {BUMPY_ROAD_MIN_BUMPS})",
                sym.bumpy_road_bumps
            ),
        });
    }
    // Threshold is already applied at extraction time (see
    // `repowise_parser::metrics::complex_conditionals`); every entry
    // here is already flagged, so no further filtering is needed.
    for cc in &sym.complex_conditionals {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(cc.line),
            kind: FindingKind::ComplexConditional,
            detail: format!(
                "condition chains {} boolean operators (>= 3)",
                cc.operator_count
            ),
        });
    }
    if sym.param_count > TOO_MANY_PARAMS {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::TooManyParameters,
            detail: format!("{} parameters (> {TOO_MANY_PARAMS})", sym.param_count),
        });
    }
    if sym.primitive_param_count >= PRIMITIVE_OBSESSION_MIN_COUNT {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::PrimitiveObsession,
            detail: format!(
                "{} bare-primitive-typed parameters (>= {PRIMITIVE_OBSESSION_MIN_COUNT})",
                sym.primitive_param_count
            ),
        });
    }
    if !skip_dead_code && graph.call_in_degree(&sym.id) == 0 {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::PossiblyDeadCode,
            detail: "no in-repo callers found (best-effort; may be a public API, \
                     trait impl, entry point, or a call this heuristic couldn't resolve)"
                .to_string(),
        });
    }
}

fn check_god_classes(index: &RepoIndex, findings: &mut Vec<Finding>) {
    let mut method_counts: HashMap<(PathBuf, String), usize> = HashMap::new();
    for file in &index.files {
        for sym in &file.symbols {
            if sym.kind == SymbolKind::Method {
                if let Some(parent) = &sym.parent {
                    *method_counts
                        .entry((file.path.clone(), parent.clone()))
                        .or_insert(0) += 1;
                }
            }
        }
    }
    for ((file, parent), count) in &method_counts {
        if *count <= GOD_CLASS_METHODS {
            continue;
        }
        let line = index
            .files
            .iter()
            .find(|f| &f.path == file)
            .and_then(|f| {
                f.symbols.iter().find(|s| {
                    &s.name == parent && matches!(s.kind, SymbolKind::Struct | SymbolKind::Class)
                })
            })
            .map(|s| s.start_line);
        findings.push(Finding {
            file: file.clone(),
            symbol: Some(parent.clone()),
            line,
            kind: FindingKind::GodClass,
            detail: format!("{count} methods (> {GOD_CLASS_METHODS})"),
        });
    }
}

fn check_duplicate_code(index: &RepoIndex, findings: &mut Vec<Finding>) {
    let mut groups: HashMap<u64, Vec<&Symbol>> = HashMap::new();
    for file in &index.files {
        for sym in &file.symbols {
            if let Some(hash) = sym.body_hash {
                groups.entry(hash).or_default().push(sym);
            }
        }
    }
    for group in groups.values() {
        if group.len() < 2 {
            continue;
        }
        for sym in group {
            let others: Vec<&str> = group
                .iter()
                .filter(|s| s.id != sym.id)
                .map(|s| s.name.as_str())
                .collect();
            findings.push(Finding {
                file: sym.file.clone(),
                symbol: Some(sym.name.clone()),
                line: Some(sym.start_line),
                kind: FindingKind::DuplicateCode,
                detail: format!("body identical to: {}", others.join(", ")),
            });
        }
    }
}

fn check_near_duplicate_code(index: &RepoIndex, findings: &mut Vec<Finding>) {
    for candidate in near_duplicate::find_near_duplicates(index) {
        findings.push(Finding {
            file: candidate.file,
            symbol: Some(candidate.symbol),
            line: Some(candidate.line),
            kind: FindingKind::NearDuplicateCode,
            detail: format!(
                "~{:.0}% textually similar to `{}` in {} (not identical -- \
                 see 'duplicate code' for exact matches)",
                candidate.overlap_ratio * 100.0,
                candidate.other_symbol,
                candidate.other_file.display()
            ),
        });
    }
}

fn check_low_cohesion(index: &RepoIndex, findings: &mut Vec<Finding>) {
    for candidate in lcom4::find_low_cohesion(index) {
        findings.push(Finding {
            file: candidate.file,
            symbol: Some(candidate.class),
            line: candidate.line,
            kind: FindingKind::LowCohesion,
            detail: format!(
                "{} disjoint field-access groups across {} tracked methods (>= {LOW_COHESION_MIN_COMPONENTS})",
                candidate.components, candidate.tracked_methods
            ),
        });
    }
}

fn score_files(index: &RepoIndex, findings: &[Finding]) -> Vec<FileHealth> {
    let mut scores: HashMap<PathBuf, f64> = index
        .files
        .iter()
        .map(|f| (f.path.clone(), MAX_SCORE))
        .collect();
    let mut counts: HashMap<PathBuf, usize> =
        index.files.iter().map(|f| (f.path.clone(), 0)).collect();

    for finding in findings {
        if let Some(s) = scores.get_mut(&finding.file) {
            *s -= finding.kind.penalty();
        }
        if let Some(c) = counts.get_mut(&finding.file) {
            *c += 1;
        }
    }

    let mut file_scores: Vec<FileHealth> = scores
        .into_iter()
        .map(|(file, score)| FileHealth {
            score: score.clamp(0.0, MAX_SCORE),
            finding_count: counts.get(&file).copied().unwrap_or(0),
            file,
        })
        .collect();
    file_scores.sort_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap()
            .then(b.finding_count.cmp(&a.finding_count))
            .then(a.file.cmp(&b.file))
    });
    file_scores
}
