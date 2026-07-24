//! Multi-repo workspace configuration -- the ongoing implementation of
//! issue #64. Earlier slices delivered: naming a set of repo roots and
//! reporting each one's indexed status (`repo_status`/`list_repos`);
//! and per-repo git-history co-change coupling shown side by side
//! (`workspace_co_changes`) -- notably NOT cross-repo dependency
//! resolution, since separate repos have separate git histories and
//! can't literally co-change together.
//!
//! This slice adds the real thing: `workspace_architecture` resolves
//! Rust `use` imports across repo boundaries (via
//! `repowise_graph::cross_repo_import_edges`), powering the
//! `get_architecture` MCP tool and the dashboard's system-map view.
//! `workspace_blast_radius` answers "which other repos would be
//! affected if this file changed" (direct cross-repo importers only --
//! matching `RepoGraph::dependents_of`'s existing single-repo
//! precedent, which is also one-hop, not transitive), powering
//! `get_blast_radius`. `detect_workspace_cycles` flags circular
//! cross-repo dependencies over the same edge data, for the
//! conformance view. Rust-only for now -- the only language this port
//! anchors to a `Cargo.toml`-derived crate name; every other language's
//! cross-repo imports are left unresolved, deliberately, for a future
//! slice.
//!
//! `workspace_contracts` (in `contracts.rs`) is the last of #64's five
//! bundled items, and fully independent of the rest of this crate: a
//! regex-based scan of each indexed file's raw text for a small, fixed
//! table of HTTP route-registration/HTTP-call patterns, matched
//! producer-to-consumer across repos. No cross-repo symbol resolution
//! involved -- see that module's own doc comment for why this is
//! coarse and heuristic by design.
//!
//! A workspace is a small standalone TOML file naming member repos by
//! name and path -- pointed at via a `--workspace <path>` flag on
//! `repowise serve`/`serve-dashboard`/the `workspace-repos`/
//! `workspace-co-changes`/`workspace-architecture`/
//! `workspace-blast-radius` subcommands, never inferred from or stored
//! inside any member repo's own `.repowise/` directory (a workspace
//! spans repos; no one member repo is a sensible owner of it).
//! Deliberately kept as its own crate rather than folded into
//! `repowise-core`: this crate now depends on `repowise-graph` (as its
//! own doc comment always anticipated), and `repowise-core` staying
//! dependency-free of every other `repowise-*` crate is a load-bearing
//! invariant the rest of this port relies on.

mod contracts;

pub use contracts::{
    workspace_contracts, ConsumerCall, ContractMatch, ContractsReport, ProducerRoute,
};

use repowise_core::RepoIndex;
use repowise_graph::CrossRepoImportEdge;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The raw shape of a workspace TOML file:
/// ```toml
/// [[repo]]
/// name = "rusty_repo_wise"
/// path = "/home/user/rusty_repo_wise"
///
/// [[repo]]
/// name = "some_other_repo"
/// path = "../some_other_repo"
/// ```
/// `path` may be relative -- see [`load_resolved`] for how it's
/// resolved.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(rename = "repo", default)]
    pub repos: Vec<WorkspaceRepoConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceRepoConfig {
    pub name: String,
    pub path: PathBuf,
}

impl WorkspaceConfig {
    pub fn from_toml_str(s: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(s)?)
    }
}

/// One configured repo with `path` already resolved to an absolute
/// path -- relative to the config file's own parent directory, not the
/// process's current directory, since a workspace file is meant to be
/// checked in or shared independent of wherever it's invoked from. The
/// only form callers (CLI/MCP/dashboard) should hold onto.
#[derive(Debug, Clone)]
pub struct ResolvedWorkspaceRepo {
    pub name: String,
    pub path: PathBuf,
}

/// Load and parse the workspace file at `config_path`, resolving every
/// configured repo's `path` relative to `config_path`'s own parent
/// directory.
pub fn load_resolved(config_path: &Path) -> anyhow::Result<Vec<ResolvedWorkspaceRepo>> {
    let config_path = config_path.canonicalize()?;
    let contents = std::fs::read_to_string(&config_path)?;
    let config = WorkspaceConfig::from_toml_str(&contents)?;
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    Ok(config
        .repos
        .into_iter()
        .map(|r| {
            let joined = if r.path.is_absolute() {
                r.path
            } else {
                config_dir.join(&r.path)
            };
            let path = joined.canonicalize().unwrap_or(joined);
            ResolvedWorkspaceRepo { name: r.name, path }
        })
        .collect())
}

/// A configured repo's indexed status -- never errors; an unindexed or
/// unreadable repo just reports `indexed: false`, the same
/// degrade-rather-than-fail shape as this port's other optional-data
/// views (e.g. `/api/hotspots`'s `available` flag).
#[derive(Debug, Clone)]
pub struct RepoStatus {
    pub name: String,
    pub path: PathBuf,
    pub indexed: bool,
    pub file_count: Option<usize>,
    pub other_file_count: Option<usize>,
}

/// Reports whether `repo` has a prior `repowise init`/`update` (a
/// loadable `.repowise/index.json`), and its file counts if so. Reused
/// as-is by the CLI/MCP/dashboard frontends -- none of them re-derive
/// this signal themselves.
pub fn repo_status(repo: &ResolvedWorkspaceRepo) -> RepoStatus {
    match RepoIndex::load(&repo.path) {
        Ok(index) => RepoStatus {
            name: repo.name.clone(),
            path: repo.path.clone(),
            indexed: true,
            file_count: Some(index.files.len()),
            other_file_count: Some(index.other_files),
        },
        Err(_) => RepoStatus {
            name: repo.name.clone(),
            path: repo.path.clone(),
            indexed: false,
            file_count: None,
            other_file_count: None,
        },
    }
}

/// One file pair that co-changes within a single repo, and how often.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoChangePair {
    pub file_a: PathBuf,
    pub file_b: PathBuf,
    pub count: usize,
}

/// One repo's most-coupled file pairs. `available` is `false` when the
/// repo has no readable git history (e.g. not a git repo, or `git` isn't
/// on `PATH`) -- the same degrade-rather-than-fail shape as `RepoStatus`.
#[derive(Debug, Clone)]
pub struct RepoCoChanges {
    pub name: String,
    pub path: PathBuf,
    pub available: bool,
    pub pairs: Vec<CoChangePair>,
}

/// Per-repo file co-change coupling for every repo in the workspace --
/// "cross-repo" in the sense that one view covers N repos side by side,
/// not that files literally co-change across repo boundaries (separate
/// git histories can't share a commit). Each repo's pairs come straight
/// from its own `repowise_git::GitAnalytics`, independent of any
/// cross-repo symbol resolution.
pub fn workspace_co_changes(repos: &[ResolvedWorkspaceRepo], top_n: usize) -> Vec<RepoCoChanges> {
    repos
        .iter()
        .map(
            |repo| match repowise_git::GitAnalytics::collect(&repo.path) {
                Ok(analytics) => RepoCoChanges {
                    name: repo.name.clone(),
                    path: repo.path.clone(),
                    available: true,
                    pairs: analytics
                        .top_co_changed_pairs(top_n)
                        .into_iter()
                        .map(|(file_a, file_b, count)| CoChangePair {
                            file_a,
                            file_b,
                            count,
                        })
                        .collect(),
                },
                Err(_) => RepoCoChanges {
                    name: repo.name.clone(),
                    path: repo.path.clone(),
                    available: false,
                    pairs: Vec::new(),
                },
            },
        )
        .collect()
}

/// One repo-pair's cross-repo import count -- the compact "system map"
/// summary. Small by construction (bounded by repo_count^2), never
/// capped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoEdgeSummary {
    pub from_repo: String,
    pub to_repo: String,
    pub edge_count: usize,
}

/// Cross-repo Rust import resolution across an entire workspace: each
/// configured repo's own indexed status (reusing `RepoStatus`, same
/// shape `list_repos`/`workspace_co_changes` already report), the raw
/// resolved edges, and a repo-pair summary. Repos with no prior
/// `repowise init`/`update` are reported as `indexed: false` in `repos`
/// and simply contribute no edges -- same degrade-rather-than-fail
/// shape as every other workspace-wide view.
pub struct ArchitectureReport {
    pub repos: Vec<RepoStatus>,
    pub repo_edges: Vec<RepoEdgeSummary>,
    pub edges: Vec<CrossRepoImportEdge>,
}

pub fn workspace_architecture(repos: &[ResolvedWorkspaceRepo]) -> ArchitectureReport {
    let mut statuses = Vec::with_capacity(repos.len());
    let mut indices: Vec<(String, RepoIndex)> = Vec::new();
    for repo in repos {
        match RepoIndex::load(&repo.path) {
            Ok(index) => {
                statuses.push(RepoStatus {
                    name: repo.name.clone(),
                    path: repo.path.clone(),
                    indexed: true,
                    file_count: Some(index.files.len()),
                    other_file_count: Some(index.other_files),
                });
                indices.push((repo.name.clone(), index));
            }
            Err(_) => statuses.push(RepoStatus {
                name: repo.name.clone(),
                path: repo.path.clone(),
                indexed: false,
                file_count: None,
                other_file_count: None,
            }),
        }
    }

    let edges = repowise_graph::cross_repo_import_edges(&indices);

    let mut counts: HashMap<(String, String), usize> = HashMap::new();
    for e in &edges {
        *counts
            .entry((e.from_repo.clone(), e.to_repo.clone()))
            .or_default() += 1;
    }
    let mut repo_edges: Vec<RepoEdgeSummary> = counts
        .into_iter()
        .map(|((from_repo, to_repo), edge_count)| RepoEdgeSummary {
            from_repo,
            to_repo,
            edge_count,
        })
        .collect();
    repo_edges.sort_by(|a, b| {
        a.from_repo
            .cmp(&b.from_repo)
            .then(a.to_repo.cmp(&b.to_repo))
    });

    ArchitectureReport {
        repos: statuses,
        repo_edges,
        edges,
    }
}

/// Direct-only cross-repo importers of `file` (within `repo_name`) --
/// matches `RepoGraph::dependents_of`'s existing single-repo precedent
/// (one hop, not transitive), just resolved across repo boundaries
/// instead of within one. `file` must already be an absolute, canonical
/// path within `repo_name`'s own root.
pub fn workspace_blast_radius(
    repos: &[ResolvedWorkspaceRepo],
    repo_name: &str,
    file: &Path,
) -> Vec<CrossRepoImportEdge> {
    let indices: Vec<(String, RepoIndex)> = repos
        .iter()
        .filter_map(|r| {
            RepoIndex::load(&r.path)
                .ok()
                .map(|idx| (r.name.clone(), idx))
        })
        .collect();
    repowise_graph::cross_repo_import_edges(&indices)
        .into_iter()
        .filter(|e| e.to_repo == repo_name && e.to_file == file)
        .collect()
}

/// Circular cross-repo dependencies (e.g. repo A imports repo B imports
/// repo A) -- a workspace's repo-level dependency graph should form a
/// DAG; a cycle is a concrete, deterministic "pattern divergence"
/// finding that needs no further human-specified rule set to detect,
/// reusing exactly the same edges `workspace_architecture` already
/// computes. Thin wrapper over `repowise_graph::detect_repo_cycles` so
/// `repowise-graph` stays decoupled from `ResolvedWorkspaceRepo`.
pub fn detect_workspace_cycles(repos: &[ResolvedWorkspaceRepo]) -> Vec<Vec<String>> {
    let report = workspace_architecture(repos);
    let pairs: Vec<(String, String)> = report
        .repo_edges
        .into_iter()
        .map(|e| (e.from_repo, e.to_repo))
        .collect();
    repowise_graph::detect_repo_cycles(&pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_toml_str_parses_repo_entries() {
        let toml = r#"
            [[repo]]
            name = "a"
            path = "/a"

            [[repo]]
            name = "b"
            path = "../b"
        "#;

        let config = WorkspaceConfig::from_toml_str(toml).unwrap();

        assert_eq!(config.repos.len(), 2);
        assert_eq!(config.repos[0].name, "a");
        assert_eq!(config.repos[0].path, PathBuf::from("/a"));
        assert_eq!(config.repos[1].name, "b");
        assert_eq!(config.repos[1].path, PathBuf::from("../b"));
    }

    #[test]
    fn from_toml_str_rejects_malformed_toml() {
        assert!(WorkspaceConfig::from_toml_str("not valid toml [[[").is_err());
    }

    #[test]
    fn from_toml_str_defaults_to_an_empty_repo_list() {
        let config = WorkspaceConfig::from_toml_str("").unwrap();
        assert!(config.repos.is_empty());
    }
}
