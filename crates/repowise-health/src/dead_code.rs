//! Confidence-tiered dead-code detection: a richer analysis than the
//! plain `PossiblyDeadCode` health marker, layering two extra risk
//! factors on top of the same "zero resolved callers" base signal.
//!
//! This is a deliberately simple, fully-documented approximation of the
//! original repowise's dead-code model (4 finding kinds, 3 confidence
//! tiers driven partly by a runtime-load risk model this port has no
//! equivalent for) — see the module doc comment in `repowise-mcp` for
//! why a full reproduction is out of scope.

use repowise_core::{Language, RepoIndex, SymbolKind};
use repowise_graph::RepoGraph;
use std::collections::HashMap;
use std::path::PathBuf;

/// How much this port's own resolution heuristics can vouch for a
/// dead-code candidate — **not** a claim about runtime safety. Even a
/// `High`-confidence candidate might be loaded reflectively, invoked via
/// dynamic dispatch, or serve as a plugin/entry point this port's static
/// call graph cannot see; "confident it's unreferenced by the analyzed
/// call graph" is not the same claim as "safe to delete".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeadCodeConfidence {
    /// Zero resolved callers, and both risk factors below are present.
    Low,
    /// Zero resolved callers and exactly one risk factor is present.
    Medium,
    /// Zero resolved callers and neither risk factor is present: no
    /// other symbol anywhere in the repo shares this symbol's exact
    /// name, and no unresolved import's last path segment matches this
    /// symbol's file stem.
    High,
}

impl DeadCodeConfidence {
    pub fn label(&self) -> &'static str {
        match self {
            DeadCodeConfidence::Low => "low",
            DeadCodeConfidence::Medium => "medium",
            DeadCodeConfidence::High => "high",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeadCodeCandidate {
    pub file: PathBuf,
    pub symbol: String,
    pub line: usize,
    pub confidence: DeadCodeConfidence,
    /// Human-readable risk factors that lowered this candidate's
    /// confidence below `High` (empty for `High`).
    pub risk_factors: Vec<String>,
}

/// Every function/method with zero resolved in-repo callers
/// (`graph.call_in_degree(...) == 0`), tiered by how much two cheap risk
/// factors undercut that signal:
///
/// 1. **Ambiguous name.** Another symbol elsewhere in the repo shares
///    this exact name. Call resolution (`repowise-graph::build`) prefers
///    a same-file candidate and otherwise fans out to every same-named
///    candidate it finds — so a call meant for *this* symbol could have
///    resolved to the other same-named one instead (shadowed by a
///    same-file match at the caller's end), leaving this symbol looking
///    uncalled when it might just have lost a naming collision.
/// 2. **Same-stem unresolved import elsewhere.** An import anywhere in
///    the repo failed to resolve, and its last path segment matches this
///    symbol's file's stem — plausibly something meant to import this
///    file (making its exports reachable) but this port's
///    directory-layout heuristics couldn't confirm it.
///
/// Zero risk factors -> `High`; one -> `Medium`; both -> `Low`. Shell is
/// exempt entirely (same as the `PossiblyDeadCode` health marker, and
/// for the same reason: shell functions are routinely invoked from
/// contexts — the command line, another script, cron — this port's call
/// graph can't see at all).
pub fn find_dead_code(index: &RepoIndex, graph: &RepoGraph) -> Vec<DeadCodeCandidate> {
    let mut name_counts: HashMap<&str, usize> = HashMap::new();
    for file in &index.files {
        for sym in &file.symbols {
            *name_counts.entry(sym.name.as_str()).or_insert(0) += 1;
        }
    }

    let mut out = Vec::new();

    for file in &index.files {
        if file.language == Language::Shell {
            continue;
        }
        let file_stem = file
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        for sym in &file.symbols {
            if !matches!(sym.kind, SymbolKind::Function | SymbolKind::Method) {
                continue;
            }
            if graph.call_in_degree(&sym.id) > 0 {
                continue;
            }

            let mut risk_factors = Vec::new();
            if name_counts.get(sym.name.as_str()).copied().unwrap_or(0) > 1 {
                risk_factors.push(format!(
                    "another symbol elsewhere in the repo is also named '{}' — a call meant \
                     for this one could have resolved to that one instead",
                    sym.name
                ));
            }
            if !file_stem.is_empty() && graph.unresolved_import_stems.contains(&file_stem) {
                risk_factors.push(format!(
                    "an unresolved import elsewhere in the repo ends in '{file_stem}', \
                     matching this file's name"
                ));
            }

            let confidence = match risk_factors.len() {
                0 => DeadCodeConfidence::High,
                1 => DeadCodeConfidence::Medium,
                _ => DeadCodeConfidence::Low,
            };

            out.push(DeadCodeCandidate {
                file: sym.file.clone(),
                symbol: sym.name.clone(),
                line: sym.start_line,
                confidence,
                risk_factors,
            });
        }
    }

    out.sort_by(|a, b| {
        b.confidence
            .cmp(&a.confidence)
            .then(a.file.cmp(&b.file))
            .then(a.line.cmp(&b.line))
    });
    out
}
