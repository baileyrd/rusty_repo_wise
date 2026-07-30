//! Architecture-complexity metrics over a workspace's repo-level
//! dependency graph: propagation cost, the cyclic core, and a single
//! deterministic score.
//!
//! Everything here is derived from the edges
//! [`crate::workspace_architecture`] already computes -- no new
//! resolution, no new scanning. What's new is the aggregate: today you
//! can ask "is there a cycle?" and get yes/no, but not "how coupled is
//! this system?".
//!
//! # Structural edges only
//!
//! Co-change is deliberately excluded. It's a *behavioral* signal that
//! moves with how a team happened to work that quarter; this score is
//! meant to describe *structure*. Folding them together would make the
//! number drift for reasons that aren't architectural, which is exactly
//! what makes a metric untrustworthy over time.
//!
//! # The honesty problem this module has to solve
//!
//! Cross-repo import resolution in this port covers every language
//! resolved single-repo via a name -> file module map (Rust, Python,
//! Java/Kotlin/Scala, Go, C#, PHP -- see
//! `repowise_graph::cross_repo::MODULE_MAP_LANGUAGES`). Every other
//! language (TypeScript/JavaScript/C/C++/Ruby/Swift/Dart/Shell, and the
//! Structural/Lightweight tiers) resolves relative imports directly
//! against the filesystem instead, which has no cross-repo equivalent
//! here, so those imports are left unresolved.
//!
//! That makes the naive version of this feature actively dangerous: a
//! workspace of six TypeScript services would resolve zero edges,
//! compute a propagation cost of zero, find no cycles, and report the
//! *best possible* architecture score. A perfect score is precisely the
//! wrong answer for a system nobody measured.
//!
//! So [`WorkspaceMetrics`] carries a [`Confidence`], and the score is an
//! `Option` that is `None` whenever the graph couldn't be resolved
//! rather than genuinely being empty. Refusing to score is the honest
//! output; a flattering one isn't.

use crate::{workspace_architecture, ResolvedWorkspaceRepo};
use repowise_core::RepoIndex;
use std::collections::{BTreeSet, HashMap};

/// How much the numbers below can be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// Cross-repo edges were resolved. The metrics describe a real
    /// graph.
    Resolved,
    /// No edges resolved, and no files in any resolvable language exist
    /// to resolve them from. The zeros below mean "nothing was
    /// measured", **not** "nothing is coupled". No score is reported.
    NoResolvableLanguage,
    /// No edges resolved, but files in a resolvable language are
    /// present -- so resolution ran and genuinely found nothing.
    /// Plausibly a real finding (truly independent repos), reported
    /// with a score but flagged, because an unrecognized layout can
    /// produce the same zero.
    NoEdgesFound,
}

impl Confidence {
    pub fn label(&self) -> &'static str {
        match self {
            Confidence::Resolved => "resolved",
            Confidence::NoResolvableLanguage => "no-resolvable-language",
            Confidence::NoEdgesFound => "no-edges-found",
        }
    }

    pub fn explanation(&self) -> &'static str {
        match self {
            Confidence::Resolved => "cross-repo edges were resolved; metrics describe a real graph",
            Confidence::NoResolvableLanguage => {
                "no cross-repo edges could be resolved and no files in a resolvable \
                 language were found -- cross-repo resolution in this port covers Rust, \
                 Python, Java, Kotlin, Scala, Go, C#, and PHP; every other language is \
                 left unresolved, so nothing here was measured. These zeros are not a \
                 finding of low coupling."
            }
            Confidence::NoEdgesFound => {
                "resolution ran over a resolvable language and found no cross-repo \
                 imports -- either genuinely independent repos, or a layout this port's \
                 resolver doesn't recognize"
            }
        }
    }
}

/// Architecture-complexity metrics for one workspace.
#[derive(Debug, Clone)]
pub struct WorkspaceMetrics {
    pub repo_count: usize,
    /// Distinct directed repo-to-repo dependency edges.
    pub edge_count: usize,
    /// Share of all ordered repo pairs where the second is reachable
    /// from the first, counting each repo as reaching itself. `0.0`
    /// when there is nothing to measure -- read [`Self::confidence`]
    /// before reading this.
    pub propagation_cost: f64,
    /// Repo groups that form circular dependencies (strongly connected
    /// components of more than one repo, plus self-cycles).
    pub cyclic_core: Vec<Vec<String>>,
    /// How many repos sit inside any cycle.
    pub repos_in_cyclic_core: usize,
    /// 1 (loosely coupled) to 10 (highly coupled). `None` when nothing
    /// was measurable -- see the module doc.
    pub complexity_score: Option<f64>,
    /// Repos whose index couldn't be loaded, so they contributed no
    /// edges in either direction.
    pub unindexed_repos: Vec<String>,
    pub confidence: Confidence,
}

impl WorkspaceMetrics {
    /// Which direction the score runs, stated in the output so `3/10`
    /// is never read as a grade.
    pub const SCALE: &'static str = "1 = loosely coupled, 10 = highly coupled (lower is better)";
}

/// Weight of propagation cost in the score.
///
/// Propagation cost is the broader signal -- it measures how far a
/// change can reach through the whole system -- so it carries most of
/// the score. The remainder goes to cycles, which are rarer but a
/// harder structural problem when present.
const PROPAGATION_WEIGHT: f64 = 0.7;
const CYCLE_WEIGHT: f64 = 0.3;

/// Reachability including self, as an adjacency-set closure.
///
/// Repo-level graphs are tiny (a workspace is tens of repos, not
/// thousands), so a straightforward fixpoint is both fast enough and
/// far easier to verify than anything cleverer.
fn transitive_closure(
    repos: &[String],
    edges: &[(String, String)],
) -> HashMap<String, BTreeSet<String>> {
    let mut reach: HashMap<String, BTreeSet<String>> = repos
        .iter()
        .map(|r| {
            // A repo always reaches itself: the standard visibility
            // matrix has a 1 diagonal, and dropping it would make a
            // single-repo workspace report 0% instead of 100%.
            let mut set = BTreeSet::new();
            set.insert(r.clone());
            (r.clone(), set)
        })
        .collect();

    for (from, to) in edges {
        if let Some(set) = reach.get_mut(from) {
            set.insert(to.clone());
        }
    }

    loop {
        let mut changed = false;
        for repo in repos {
            let current: Vec<String> = reach
                .get(repo)
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default();
            let mut additions = BTreeSet::new();
            for target in &current {
                if let Some(onward) = reach.get(target) {
                    for r in onward {
                        if !current.contains(r) {
                            additions.insert(r.clone());
                        }
                    }
                }
            }
            if !additions.is_empty() {
                if let Some(set) = reach.get_mut(repo) {
                    for a in additions {
                        changed |= set.insert(a);
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    reach
}

/// Does any repo in the workspace contain files in a language cross-repo
/// resolution can actually resolve (`repowise_graph::cross_repo::MODULE_MAP_LANGUAGES`)?
///
/// The discriminator between "resolution found nothing" and "resolution
/// never had anything to work with" -- see [`Confidence`].
fn has_resolvable_language_files(repos: &[ResolvedWorkspaceRepo]) -> bool {
    repos.iter().any(|repo| {
        RepoIndex::load(&repo.path)
            .map(|index| {
                index
                    .files
                    .iter()
                    .any(|f| repowise_graph::MODULE_MAP_LANGUAGES.contains(&f.language))
            })
            .unwrap_or(false)
    })
}

/// Compute architecture-complexity metrics for `repos`.
pub fn workspace_metrics(repos: &[ResolvedWorkspaceRepo]) -> WorkspaceMetrics {
    let report = workspace_architecture(repos);
    let names: Vec<String> = repos.iter().map(|r| r.name.clone()).collect();
    let unindexed_repos: Vec<String> = report
        .repos
        .iter()
        .filter(|r| !r.indexed)
        .map(|r| r.name.clone())
        .collect();

    // Deduplicated and self-edges dropped: a repo importing itself is
    // not a cross-repo dependency, and counting it would inflate both
    // the edge count and the propagation cost.
    let edges: Vec<(String, String)> = report
        .repo_edges
        .iter()
        .filter(|e| e.from_repo != e.to_repo)
        .map(|e| (e.from_repo.clone(), e.to_repo.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let cyclic_core = repowise_graph::detect_repo_cycles(&edges);
    let repos_in_cyclic_core = cyclic_core.iter().flatten().collect::<BTreeSet<_>>().len();

    let n = names.len();
    let propagation_cost = if n == 0 {
        0.0
    } else {
        let reach = transitive_closure(&names, &edges);
        let reachable: usize = names
            .iter()
            .map(|r| reach.get(r).map(|s| s.len()).unwrap_or(0))
            .sum();
        reachable as f64 / (n * n) as f64
    };

    let confidence = if !edges.is_empty() {
        Confidence::Resolved
    } else if has_resolvable_language_files(repos) {
        Confidence::NoEdgesFound
    } else {
        Confidence::NoResolvableLanguage
    };

    // Withheld rather than flattering: see the module doc. A workspace
    // whose languages this port can't resolve would otherwise score the
    // best possible number for having been unmeasurable.
    let complexity_score = if confidence == Confidence::NoResolvableLanguage {
        None
    } else {
        let cyclic_share = if n == 0 {
            0.0
        } else {
            repos_in_cyclic_core as f64 / n as f64
        };
        // Propagation cost already floors at 1/n (self-reachability),
        // so subtract that floor before scaling -- otherwise a fully
        // decoupled workspace scores above 1 purely for existing.
        let floor = if n == 0 { 0.0 } else { 1.0 / n as f64 };
        let coupling =
            ((propagation_cost - floor) / (1.0 - floor).max(f64::EPSILON)).clamp(0.0, 1.0);
        let raw = PROPAGATION_WEIGHT * coupling + CYCLE_WEIGHT * cyclic_share;
        Some(1.0 + raw * 9.0)
    };

    WorkspaceMetrics {
        repo_count: n,
        edge_count: edges.len(),
        propagation_cost,
        cyclic_core,
        repos_in_cyclic_core,
        complexity_score,
        unindexed_repos,
        confidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn edges(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn closure_counts_self_reachability() {
        let reach = transitive_closure(&names(&["a", "b"]), &[]);
        assert_eq!(reach["a"].len(), 1, "a reaches only itself");
        assert_eq!(reach["b"].len(), 1);
    }

    #[test]
    fn closure_follows_a_chain_transitively() {
        let reach = transitive_closure(&names(&["a", "b", "c"]), &edges(&[("a", "b"), ("b", "c")]));
        assert!(
            reach["a"].contains("c"),
            "a -> b -> c must make c reachable from a: {:?}",
            reach["a"]
        );
        assert!(!reach["c"].contains("a"), "edges are directed");
    }

    #[test]
    fn closure_terminates_on_a_cycle() {
        let reach = transitive_closure(&names(&["a", "b"]), &edges(&[("a", "b"), ("b", "a")]));
        assert_eq!(reach["a"].len(), 2);
        assert_eq!(reach["b"].len(), 2);
    }
}
