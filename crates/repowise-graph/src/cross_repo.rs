//! Cross-repo Rust import resolution: a `use` import in one workspace
//! repo, resolved against another workspace repo's Rust module-path
//! map. Rust-only -- the only language this port anchors to a
//! `Cargo.toml`-derived crate name (see `modpath::rust_module_path`);
//! every other language's cross-repo imports are left unresolved,
//! deliberately, for a future slice.
//!
//! This is a separate pass from `RepoGraph::build`, which only ever
//! sees one repo's `RepoIndex` at a time. Callers (`repowise-workspace`)
//! supply every configured repo's `RepoIndex` up front.

use crate::modpath;
use petgraph::algo::kosaraju_scc;
use petgraph::graph::DiGraph;
use repowise_core::{Language, RepoIndex};
use std::collections::HashMap;
use std::path::PathBuf;

/// Rust module path -> defining file, for one repo's `RepoIndex`. A
/// freestanding duplicate of the map `RepoGraph::build` computes
/// internally for its own single-repo resolution -- kept as a pure
/// function so it's usable per-repo before any single-repo `RepoGraph`
/// exists, without building a full `RepoGraph` per other repo just to
/// read one field back out.
pub fn rust_module_map(index: &RepoIndex) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();
    for file in &index.files {
        if file.language == Language::Rust {
            if let Some(mp) = modpath::rust_module_path(&file.path) {
                map.insert(mp, file.path.clone());
            }
        }
    }
    map
}

/// One `use` import in `from_repo`/`from_file` resolved to a specific
/// file in a *different* workspace repo. First match across other repos
/// wins (in the order given to `cross_repo_import_edges`) -- cross-repo
/// name collisions are rare and out of scope to disambiguate further.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossRepoImportEdge {
    pub from_repo: String,
    pub from_file: PathBuf,
    pub line: usize,
    pub to_repo: String,
    pub to_file: PathBuf,
    pub import_path: String,
}

/// Cross-repo Rust `use` resolution over every repo in `repos`.
///
/// An import counts as a cross-repo candidate only if it's unresolved
/// BOTH at parse time (`resolved_file: None`) AND against its own
/// repo's module map -- the same two-part "did this end up unresolved"
/// test `RepoGraph::build` applies for single-repo resolution, re-
/// derived here so an import already fully explained by a sibling crate
/// within the SAME repo (this port's own multi-crate layout, for
/// example) is never mistaken for a cross-repo edge, even if another
/// repo happens to define a module at the same dotted path.
pub fn cross_repo_import_edges(repos: &[(String, RepoIndex)]) -> Vec<CrossRepoImportEdge> {
    let maps: Vec<(&str, HashMap<String, PathBuf>)> = repos
        .iter()
        .map(|(name, index)| (name.as_str(), rust_module_map(index)))
        .collect();

    let mut edges = Vec::new();
    for (i, (from_repo, index)) in repos.iter().enumerate() {
        let own_map = &maps[i].1;
        for file in &index.files {
            if file.language != Language::Rust {
                continue;
            }
            for imp in &file.imports {
                if imp.resolved_file.is_some() {
                    continue;
                }
                if modpath::resolve_import(&imp.path, "::", own_map).is_some() {
                    continue; // resolves within its own repo -- not a cross-repo candidate
                }
                for (other_repo, other_map) in &maps {
                    if *other_repo == from_repo.as_str() {
                        continue;
                    }
                    if let Some(target) = modpath::resolve_import(&imp.path, "::", other_map) {
                        edges.push(CrossRepoImportEdge {
                            from_repo: from_repo.clone(),
                            from_file: file.path.clone(),
                            line: imp.line,
                            to_repo: other_repo.to_string(),
                            to_file: target.clone(),
                            import_path: imp.path.clone(),
                        });
                        break;
                    }
                }
            }
        }
    }
    edges
}

/// Detects cycles among repo-level edges (e.g. repo A imports repo B
/// imports repo A). Generic over repo-name string pairs rather than
/// `CrossRepoImportEdge`s -- operates on an aggregated repo-to-repo
/// edge list, not individual import sites. Each returned group is the
/// set of repo names involved in one cycle (order not meaningful); a
/// repo with no cycle partner is omitted, not returned as a singleton.
pub fn detect_repo_cycles(repo_edges: &[(String, String)]) -> Vec<Vec<String>> {
    let mut graph = DiGraph::<String, ()>::new();
    let mut index_of = HashMap::new();
    let mut self_loops = std::collections::HashSet::new();
    for (from, to) in repo_edges {
        if from == to {
            self_loops.insert(from.clone());
        }
        let a = *index_of
            .entry(from.clone())
            .or_insert_with(|| graph.add_node(from.clone()));
        let b = *index_of
            .entry(to.clone())
            .or_insert_with(|| graph.add_node(to.clone()));
        graph.add_edge(a, b, ());
    }

    kosaraju_scc(&graph)
        .into_iter()
        .filter_map(|scc| {
            if scc.len() > 1 {
                Some(scc.into_iter().map(|idx| graph[idx].clone()).collect())
            } else {
                let name = &graph[scc[0]];
                self_loops.contains(name).then(|| vec![name.clone()])
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_repo_cycles_finds_a_two_repo_cycle() {
        let edges = vec![
            ("a".to_string(), "b".to_string()),
            ("b".to_string(), "a".to_string()),
        ];
        let cycles = detect_repo_cycles(&edges);
        assert_eq!(cycles.len(), 1);
        let mut cycle = cycles[0].clone();
        cycle.sort();
        assert_eq!(cycle, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn detect_repo_cycles_reports_none_for_a_dag() {
        let edges = vec![
            ("a".to_string(), "b".to_string()),
            ("b".to_string(), "c".to_string()),
        ];
        assert!(detect_repo_cycles(&edges).is_empty());
    }

    #[test]
    fn detect_repo_cycles_ignores_isolated_repos_with_no_edges() {
        assert!(detect_repo_cycles(&[]).is_empty());
    }
}
