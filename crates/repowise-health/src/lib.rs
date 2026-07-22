//! Deterministic, rule-based code-health scoring — no LLM/ML involved.
//!
//! This implements a focused subset of repowise's "25 deterministic
//! markers": long functions, high cyclomatic complexity, oversized
//! parameter lists, god classes (too many methods), duplicate code
//! (identical function/method bodies), and possibly-dead code (zero
//! resolved callers). Git-history-based markers (churn, hotspots, bug-fix
//! history) aren't implemented yet — that needs the git-analytics layer,
//! which is a separate phase.
//!
//! Every marker here is a plain threshold over data `repowise-parser`/
//! `repowise-graph` already computed; nothing is inferred or guessed.

use repowise_core::{RepoIndex, Symbol, SymbolKind};
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

const PENALTY_LONG_FUNCTION: f64 = 0.5;
const PENALTY_HIGH_COMPLEXITY: f64 = 1.0;
const PENALTY_TOO_MANY_PARAMS: f64 = 0.3;
const PENALTY_GOD_CLASS: f64 = 1.5;
const PENALTY_DUPLICATE: f64 = 0.5;
const PENALTY_DEAD_CODE: f64 = 0.2;

const MAX_SCORE: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    LongFunction,
    HighComplexity,
    TooManyParameters,
    GodClass,
    DuplicateCode,
    PossiblyDeadCode,
}

impl FindingKind {
    pub fn label(&self) -> &'static str {
        match self {
            FindingKind::LongFunction => "long-function",
            FindingKind::HighComplexity => "high-complexity",
            FindingKind::TooManyParameters => "too-many-params",
            FindingKind::GodClass => "god-class",
            FindingKind::DuplicateCode => "duplicate-code",
            FindingKind::PossiblyDeadCode => "possibly-dead-code",
        }
    }

    fn penalty(&self) -> f64 {
        match self {
            FindingKind::LongFunction => PENALTY_LONG_FUNCTION,
            FindingKind::HighComplexity => PENALTY_HIGH_COMPLEXITY,
            FindingKind::TooManyParameters => PENALTY_TOO_MANY_PARAMS,
            FindingKind::GodClass => PENALTY_GOD_CLASS,
            FindingKind::DuplicateCode => PENALTY_DUPLICATE,
            FindingKind::PossiblyDeadCode => PENALTY_DEAD_CODE,
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
        for sym in &file.symbols {
            if !matches!(sym.kind, SymbolKind::Function | SymbolKind::Method) {
                continue;
            }
            check_function_markers(sym, graph, &mut findings);
        }
    }

    check_god_classes(index, &mut findings);
    check_duplicate_code(index, &mut findings);

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

fn check_function_markers(sym: &Symbol, graph: &RepoGraph, findings: &mut Vec<Finding>) {
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
    if sym.param_count > TOO_MANY_PARAMS {
        findings.push(Finding {
            file: sym.file.clone(),
            symbol: Some(sym.name.clone()),
            line: Some(sym.start_line),
            kind: FindingKind::TooManyParameters,
            detail: format!("{} parameters (> {TOO_MANY_PARAMS})", sym.param_count),
        });
    }
    if graph.call_in_degree(&sym.id) == 0 {
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
