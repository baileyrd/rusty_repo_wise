//! Multi-repo workspace configuration -- the ongoing implementation of
//! issue #64. Earlier slices delivered: naming a set of repo roots and
//! reporting each one's indexed status (`repo_status`/`list_repos`);
//! and per-repo git-history co-change coupling shown side by side
//! (`workspace_co_changes`) -- notably NOT cross-repo dependency
//! resolution, since separate repos have separate git histories and
//! can't literally co-change together.
//!
//! This slice adds the real thing: `workspace_architecture` resolves
//! imports across repo boundaries (via
//! `repowise_graph::cross_repo_import_edges`), powering the
//! `get_architecture` MCP tool and the dashboard's system-map view.
//! `workspace_blast_radius` answers "which other repos would be
//! affected if this file changed" (direct cross-repo importers only --
//! matching `RepoGraph::dependents_of`'s existing single-repo
//! precedent, which is also one-hop, not transitive), powering
//! `get_blast_radius`. `detect_workspace_cycles` flags circular
//! cross-repo dependencies over the same edge data, for the
//! conformance view. Covers every language this port resolves
//! single-repo via a name -> file module map: Rust, Python,
//! Java/Kotlin/Scala, Go, C#, and PHP's `use Namespace\Class;` form --
//! see `repowise_graph::cross_repo`'s own doc comment
//! (`MODULE_MAP_LANGUAGES`) for exactly which and why. Every other
//! language resolves imports directly against the filesystem instead of
//! through a module map (a different resolution mechanism entirely) and
//! has no cross-repo equivalent here, deliberately, for a future slice.
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

mod breaking_changes;
mod contracts;
mod metrics;

pub use metrics::{workspace_metrics, Confidence, WorkspaceMetrics};

pub use contracts::{
    workspace_contracts, workspace_diagnostics, ConsumerCall, ContractDiagnostics, ContractMatch,
    ContractsReport, OrphanProducer, ProducerRoute, RepoEndpointCounts, UnmatchedConsumer,
    UnmatchedReason,
};

pub use breaking_changes::{workspace_contract_changes, BrokenContract, ContractKey};

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
    /// Optional path to a committed portable index
    /// (`repowise export --format index`), issue #384. When set, this
    /// repo's index is read from that artifact instead of from a local
    /// `.repowise/index.json` under `path`.
    ///
    /// Resolved relative to the workspace file, the same rule `path`
    /// already follows — a workspace file is meant to be checked in and
    /// shared, so nothing in it may depend on the invoking process's
    /// current directory.
    ///
    /// `path` stays **required** even with `index` set: it is the repo's
    /// anchor root (re-anchoring a portable index needs somewhere to
    /// anchor to) and the working directory for the commands that shell
    /// out to git. It does **not** have to exist for the index-only
    /// commands — `into_anchored` never touches the filesystem — which
    /// is what lets `workspace-architecture`, `workspace-blast-radius`,
    /// `workspace-conformance`, and `workspace-metrics` run against
    /// repos that were never cloned.
    #[serde(default)]
    pub index: Option<PathBuf>,
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
    /// Absolute path to this repo's committed portable index, when the
    /// workspace file names one. See [`load_repo_index`].
    pub index: Option<PathBuf>,
}

/// Where a repo's index came from — reported so a workspace answer built
/// from committed artifacts can't be mistaken for one built from live
/// local indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexSource {
    /// A local `.repowise/index.json` under the repo's own path.
    Local,
    /// A committed portable artifact named by the workspace file.
    Portable,
}

impl IndexSource {
    pub fn label(&self) -> &'static str {
        match self {
            IndexSource::Local => "local",
            IndexSource::Portable => "portable",
        }
    }
}

/// A repo's index plus where it came from and whether it still matches
/// that repo's checkout.
pub struct LoadedRepoIndex {
    pub index: RepoIndex,
    pub source: IndexSource,
    /// Module paths recorded in the artifact for the languages that
    /// can't recompute one without a checkout (issue #388). Empty for a
    /// local index, and for artifacts exported before #388 -- in both
    /// cases resolution falls back to recomputing from disk.
    pub module_paths: HashMap<PathBuf, String>,
    /// `None` when there is nothing to compare against — no git in the
    /// repo path, or no commit recorded in the artifact. Reported as
    /// unknown, never as "up to date".
    pub stale: Option<bool>,
}

/// Load one repo's index, from its committed portable artifact when the
/// workspace file names one and from its local `.repowise/index.json`
/// otherwise (issue #384).
///
/// Mixed sources across a workspace are expected, not exceptional: repos
/// release on their own schedules, so some members will publish an
/// artifact while others are checked out locally.
pub fn load_repo_index(repo: &ResolvedWorkspaceRepo) -> anyhow::Result<LoadedRepoIndex> {
    let (index, source, module_paths) = match &repo.index {
        Some(artifact) => {
            let portable = repowise_core::portable::PortableIndex::load(artifact)?;
            let module_paths = portable.anchored_module_paths(&repo.path);
            (
                portable.into_anchored(&repo.path)?,
                IndexSource::Portable,
                module_paths,
            )
        }
        None => (
            RepoIndex::load(&repo.path)?,
            IndexSource::Local,
            HashMap::new(),
        ),
    };
    let stale = staleness_of(&index, &repo.path);
    Ok(LoadedRepoIndex {
        index,
        source,
        stale,
        module_paths,
    })
}

/// Languages whose cross-repo module map is derived by **reading the
/// filesystem**, not from the index alone.
///
/// `repowise_graph::modpath::rust_module_path` walks up to a
/// `Cargo.toml` and reads the package name out of it; `go_module_path`
/// does the same with `go.mod`. Python, Java/Kotlin/Scala, C#, and PHP
/// derive their module paths from `(file, root)` by string manipulation
/// and need nothing on disk.
///
/// That difference decides whether a workspace member backed only by a
/// committed artifact can participate in cross-repo resolution at all.
const DISK_DERIVED_MODULE_MAP_LANGUAGES: [repowise_core::Language; 2] =
    [repowise_core::Language::Rust, repowise_core::Language::Go];

/// Repos whose cross-repo imports **cannot** resolve, because their
/// index came from an artifact, their path isn't on disk, and their
/// language needs a manifest file to derive module names (issue #384).
///
/// This exists because the failure is otherwise invisible: resolution
/// simply finds nothing, and "no cross-repo dependencies" is a perfectly
/// plausible-looking answer. `workspace-conformance` gates CI on exactly
/// that shape of result, so a silent blind spot there reads as a pass.
///
/// Returns `(repo name, language label)` pairs.
pub fn resolution_blind_spots(repos: &[ResolvedWorkspaceRepo]) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for repo in repos {
        if repo.index.is_none() || repo.path.exists() {
            continue;
        }
        let Ok(loaded) = load_repo_index(repo) else {
            continue;
        };
        for lang in DISK_DERIVED_MODULE_MAP_LANGUAGES {
            // A file with a recorded module path (issue #388) resolves
            // fine without a checkout -- that is the whole point of
            // recording it -- so only files still relying on
            // recomputation count as blind.
            if loaded
                .index
                .files
                .iter()
                .any(|f| f.language == lang && !loaded.module_paths.contains_key(&f.path))
            {
                out.push((repo.name.clone(), lang.label()));
                break;
            }
        }
    }
    out
}

/// Whether `index` describes something other than what `root` currently
/// has checked out. `None` means unanswerable, which is a third answer
/// and not a synonym for "fresh".
fn staleness_of(index: &RepoIndex, root: &Path) -> Option<bool> {
    let indexed = index.indexed_commit.as_deref()?;
    let head = repo_head_sha(root)?;
    Some(indexed != head)
}

/// `HEAD`'s short SHA for `root`, or `None` if it isn't a git repo.
///
/// Shelled out here rather than taken from `repowise-git` on purpose:
/// this crate deliberately depends on `repowise-git` only for
/// co-change reporting, and a workspace made entirely of committed
/// artifacts should not need that dependency to answer "is this
/// current".
fn repo_head_sha(root: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!sha.is_empty()).then_some(sha)
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
            let resolve = |p: PathBuf| {
                let joined = if p.is_absolute() {
                    p
                } else {
                    config_dir.join(p)
                };
                joined.canonicalize().unwrap_or(joined)
            };
            ResolvedWorkspaceRepo {
                name: r.name,
                path: resolve(r.path),
                index: r.index.map(resolve),
            }
        })
        .collect())
}

/// The directory a workspace's own state lives in -- sibling to the
/// workspace TOML file itself, matching upstream repowise's own
/// `.repowise-workspace/` naming for the same concept. Never inside any
/// one member repo's own `.repowise/`: a workspace spans repos, so no
/// one member repo is a sensible owner of workspace-level state, the
/// same reasoning `load_resolved` already applies to relative repo
/// paths. Currently the only consumer is contract breaking-change
/// snapshots (`breaking_changes.rs`); a future slice could move other
/// workspace-level caches here too.
pub fn workspace_state_dir(config_path: &Path) -> PathBuf {
    let canonical = config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.to_path_buf());
    let dir = canonical.parent().unwrap_or_else(|| Path::new("."));
    dir.join(".repowise-workspace")
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
    /// Where the index came from, when one loaded (issue #384).
    pub source: Option<IndexSource>,
    /// Whether that index has drifted from this repo's checkout.
    /// `None` = unanswerable (no git, or no commit recorded), which is
    /// a distinct answer from "fresh".
    pub stale: Option<bool>,
}

/// Reports whether `repo` has a prior `repowise init`/`update` (a
/// loadable `.repowise/index.json`), and its file counts if so. Reused
/// as-is by the CLI/MCP/dashboard frontends -- none of them re-derive
/// this signal themselves.
pub fn repo_status(repo: &ResolvedWorkspaceRepo) -> RepoStatus {
    match load_repo_index(repo) {
        Ok(loaded) => RepoStatus {
            name: repo.name.clone(),
            path: repo.path.clone(),
            indexed: true,
            file_count: Some(loaded.index.files.len()),
            other_file_count: Some(loaded.index.other_files),
            source: Some(loaded.source),
            stale: loaded.stale,
        },
        Err(_) => RepoStatus {
            name: repo.name.clone(),
            path: repo.path.clone(),
            indexed: false,
            file_count: None,
            other_file_count: None,
            source: None,
            stale: None,
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
    let mut module_overrides: Vec<HashMap<PathBuf, String>> = Vec::new();
    for repo in repos {
        match load_repo_index(repo) {
            Ok(loaded) => {
                statuses.push(RepoStatus {
                    name: repo.name.clone(),
                    path: repo.path.clone(),
                    indexed: true,
                    file_count: Some(loaded.index.files.len()),
                    other_file_count: Some(loaded.index.other_files),
                    source: Some(loaded.source),
                    stale: loaded.stale,
                });
                module_overrides.push(loaded.module_paths);
                indices.push((repo.name.clone(), loaded.index));
            }
            Err(_) => statuses.push(RepoStatus {
                name: repo.name.clone(),
                path: repo.path.clone(),
                indexed: false,
                file_count: None,
                other_file_count: None,
                source: None,
                stale: None,
            }),
        }
    }

    let with_modules: Vec<(String, RepoIndex, &HashMap<PathBuf, String>)> = indices
        .iter()
        .zip(&module_overrides)
        .map(|((name, index), overrides)| (name.clone(), index.clone(), overrides))
        .collect();
    let edges = repowise_graph::cross_repo_import_edges_with_modules(&with_modules);

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
    let loaded: Vec<(String, RepoIndex, HashMap<PathBuf, String>)> = repos
        .iter()
        .filter_map(|r| {
            load_repo_index(r)
                .ok()
                .map(|l| (r.name.clone(), l.index, l.module_paths))
        })
        .collect();
    let indices: Vec<(String, RepoIndex, &HashMap<PathBuf, String>)> = loaded
        .iter()
        .map(|(name, index, overrides)| (name.clone(), index.clone(), overrides))
        .collect();
    repowise_graph::cross_repo_import_edges_with_modules(&indices)
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

    use std::fs;

    /// Two Rust crates where `consumer` genuinely imports across the
    /// repo boundary.
    ///
    /// Rust rather than Python on purpose: this port's Python resolver
    /// walks progressively shorter module prefixes, so a `pkg/` package
    /// present in both repos makes `pkg.core` resolve against the
    /// importer's *own* `pkg/__init__.py` and never become a cross-repo
    /// candidate at all. A fixture like that yields zero edges from both
    /// sources, and an "identical results" assertion over two empty
    /// lists proves nothing. Crate-name module maps produce a real edge.
    fn two_repo_fixture(dir: &Path) {
        for (name, rel, body) in [
            ("provider", "src/thing.rs", "pub fn thing() -> i32 { 1 }\n"),
            (
                "consumer",
                "src/lib.rs",
                "use provider::thing::thing;\n\npub fn use_it() -> i32 { thing() }\n",
            ),
        ] {
            let repo = dir.join(name);
            fs::create_dir_all(repo.join("src")).unwrap();
            fs::write(
                repo.join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
            )
            .unwrap();
            let path = repo.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }
    }

    fn index_of(root: &Path) -> RepoIndex {
        let discovered = repowise_core::discover_files(root).unwrap();
        let mut files = Vec::new();
        let mut other_files = 0;
        for entry in discovered {
            if matches!(entry.language, repowise_core::Language::Other) {
                other_files += 1;
                continue;
            }
            let source = fs::read_to_string(&entry.path).unwrap();
            match repowise_parser::parse_file(&entry.path, entry.language, &source).unwrap() {
                Some(record) => files.push(record),
                None => other_files += 1,
            }
        }
        RepoIndex {
            root: root.to_path_buf(),
            files,
            other_files,
            indexed_commit: None,
        }
    }

    /// The correctness question #384 flagged as needing verification
    /// rather than assumption: cross-repo import resolution spans repos,
    /// but anchoring is per-repo, so a workspace built from portable
    /// artifacts must resolve exactly what local indexes would.
    #[test]
    fn cross_repo_resolution_is_identical_from_portable_indexes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        two_repo_fixture(&root);

        let repos: Vec<ResolvedWorkspaceRepo> = ["provider", "consumer"]
            .iter()
            .map(|name| ResolvedWorkspaceRepo {
                name: name.to_string(),
                path: root.join(name),
                index: None,
            })
            .collect();

        // Baseline: resolve from indexes built in place.
        let local: Vec<(String, RepoIndex)> = repos
            .iter()
            .map(|r| (r.name.clone(), index_of(&r.path)))
            .collect();
        let from_local = repowise_graph::cross_repo_import_edges(&local);

        // Same repos, but each index round-tripped through the portable
        // form and re-anchored.
        let portable: Vec<(String, RepoIndex)> = repos
            .iter()
            .map(|r| {
                let idx = index_of(&r.path);
                let restored = repowise_core::portable::PortableIndex::from_index(&idx)
                    .into_anchored(&r.path)
                    .unwrap();
                (r.name.clone(), restored)
            })
            .collect();
        let from_portable = repowise_graph::cross_repo_import_edges(&portable);

        // Guard against a vacuous pass: two empty lists are trivially
        // "identical" and would prove nothing about anchoring.
        assert!(
            !from_local.is_empty(),
            "fixture must actually resolve a cross-repo edge, or this test proves nothing"
        );
        assert_eq!(
            from_local.len(),
            from_portable.len(),
            "portable indexes resolved a different number of cross-repo edges"
        );
        for (a, b) in from_local.iter().zip(&from_portable) {
            assert_eq!((&a.from_repo, &a.to_repo), (&b.from_repo, &b.to_repo));
            assert_eq!((&a.from_file, &a.to_file), (&b.from_file, &b.to_file));
        }
    }

    /// Mixed sources in one workspace are the expected case, not an
    /// edge case: repos publish artifacts on their own schedules.
    #[test]
    fn a_workspace_can_mix_local_and_portable_members() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        two_repo_fixture(&root);

        // `provider` gets a committed artifact; `consumer` stays local.
        let artifact = root.join("provider.portable.json");
        repowise_core::portable::PortableIndex::from_index(&index_of(&root.join("provider")))
            .save(&artifact)
            .unwrap();
        index_of(&root.join("consumer"))
            .save(&root.join("consumer"))
            .unwrap();

        let repos = [
            ResolvedWorkspaceRepo {
                name: "provider".to_string(),
                path: root.join("provider"),
                index: Some(artifact),
            },
            ResolvedWorkspaceRepo {
                name: "consumer".to_string(),
                path: root.join("consumer"),
                index: None,
            },
        ];

        let provider = load_repo_index(&repos[0]).unwrap();
        let consumer = load_repo_index(&repos[1]).unwrap();
        assert_eq!(provider.source, IndexSource::Portable);
        assert_eq!(consumer.source, IndexSource::Local);
        assert_eq!(provider.index.files.len(), consumer.index.files.len());

        // And both report through the shared status path.
        assert_eq!(
            repo_status(&repos[0]).source,
            Some(IndexSource::Portable),
            "the source must be visible to callers, not just internal"
        );
    }

    /// An index-only workspace member never has to be cloned. This is
    /// the whole point of #384 -- `into_anchored` doesn't touch the
    /// filesystem, so a path that doesn't exist still anchors.
    #[test]
    fn a_member_backed_by_an_artifact_needs_no_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        two_repo_fixture(&root);

        let artifact = root.join("provider.portable.json");
        repowise_core::portable::PortableIndex::from_index(&index_of(&root.join("provider")))
            .save(&artifact)
            .unwrap();

        let never_cloned = ResolvedWorkspaceRepo {
            name: "provider".to_string(),
            path: root.join("not-checked-out-anywhere"),
            index: Some(artifact),
        };
        let loaded = load_repo_index(&never_cloned).expect("no checkout required");
        assert_eq!(loaded.source, IndexSource::Portable);
        assert!(!loaded.index.files.is_empty());
        assert_eq!(
            loaded.stale, None,
            "no git to compare against must read as unknown, never as fresh"
        );
    }

    /// The regression #388 closes: a Rust member with no checkout used
    /// to contribute zero cross-repo edges, because its crate name lives
    /// in a `Cargo.toml` that isn't there. With the module path recorded
    /// at export time, it resolves.
    #[test]
    fn a_never_cloned_rust_member_still_resolves_cross_repo_edges() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        two_repo_fixture(&root);

        let provider_path = root.join("provider");
        let provider_index = index_of(&provider_path);
        let module_paths: Vec<(PathBuf, String)> =
            repowise_graph::module_map(&provider_index, repowise_core::Language::Rust)
                .into_iter()
                .map(|(mp, file)| (file, mp))
                .collect();
        assert!(!module_paths.is_empty(), "fixture must have Rust modules");

        let artifact = root.join("provider.portable.json");
        repowise_core::portable::PortableIndex::from_index(&provider_index)
            .with_module_paths(&provider_path, module_paths)
            .unwrap()
            .save(&artifact)
            .unwrap();
        index_of(&root.join("consumer"))
            .save(&root.join("consumer"))
            .unwrap();

        let repos = [
            ResolvedWorkspaceRepo {
                name: "provider".to_string(),
                // Deliberately somewhere that does not exist.
                path: root.join("provider-never-cloned"),
                index: Some(artifact),
            },
            ResolvedWorkspaceRepo {
                name: "consumer".to_string(),
                path: root.join("consumer"),
                index: None,
            },
        ];

        let report = workspace_architecture(&repos);
        assert!(
            !report.edges.is_empty(),
            "a never-cloned Rust member must still resolve: {:?}",
            report.edges
        );
        assert_eq!(report.edges[0].from_repo, "consumer");
        assert_eq!(report.edges[0].to_repo, "provider");

        // And it is no longer reported as a blind spot.
        assert!(
            resolution_blind_spots(&repos).is_empty(),
            "recorded module paths mean this is no longer blind"
        );
    }

    /// The warning must still fire for an artifact exported before #388,
    /// which carries no module paths -- those repos are still blind, and
    /// silently so.
    #[test]
    fn a_pre_388_artifact_without_module_paths_is_still_a_blind_spot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        two_repo_fixture(&root);

        let artifact = root.join("provider.portable.json");
        // No `with_module_paths` -- exactly what a pre-#388 export wrote.
        repowise_core::portable::PortableIndex::from_index(&index_of(&root.join("provider")))
            .save(&artifact)
            .unwrap();

        let repos = [ResolvedWorkspaceRepo {
            name: "provider".to_string(),
            path: root.join("never-cloned"),
            index: Some(artifact),
        }];
        let blind = resolution_blind_spots(&repos);
        assert_eq!(
            blind.len(),
            1,
            "an artifact with no module paths is still blind"
        );
        assert_eq!(blind[0].1, "Rust");
    }

    /// The limitation that makes `resolution_blind_spots` necessary,
    /// found by running the feature rather than reading it: Rust's
    /// module map comes from a `Cargo.toml` on disk, so a Rust member
    /// with no checkout contributes no edges -- and an empty edge list
    /// is indistinguishable from "these repos are independent".
    #[test]
    fn a_rust_member_without_a_checkout_is_flagged_as_a_blind_spot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        two_repo_fixture(&root);

        let artifact = root.join("provider.portable.json");
        repowise_core::portable::PortableIndex::from_index(&index_of(&root.join("provider")))
            .save(&artifact)
            .unwrap();

        let repos = vec![ResolvedWorkspaceRepo {
            name: "provider".to_string(),
            path: root.join("never-cloned"),
            index: Some(artifact),
        }];

        let blind = resolution_blind_spots(&repos);
        assert_eq!(
            blind.len(),
            1,
            "a Rust member with no checkout must be flagged"
        );
        assert_eq!(blind[0].0, "provider");
        assert_eq!(blind[0].1, "Rust");
    }

    /// The flag is about the *combination*, not about portability: a
    /// member with a real checkout resolves fine however its index was
    /// loaded, so flagging it would be noise.
    #[test]
    fn a_portable_member_that_is_checked_out_is_not_a_blind_spot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        two_repo_fixture(&root);

        let artifact = root.join("provider.portable.json");
        repowise_core::portable::PortableIndex::from_index(&index_of(&root.join("provider")))
            .save(&artifact)
            .unwrap();

        let repos = vec![ResolvedWorkspaceRepo {
            name: "provider".to_string(),
            path: root.join("provider"), // really there
            index: Some(artifact),
        }];
        assert!(resolution_blind_spots(&repos).is_empty());
    }

    #[test]
    fn from_toml_str_parses_an_optional_index_path() {
        let toml = r#"
            [[repo]]
            name = "a"
            path = "/a"
            index = "artifacts/a.portable.json"

            [[repo]]
            name = "b"
            path = "/b"
        "#;
        let config = WorkspaceConfig::from_toml_str(toml).unwrap();
        assert_eq!(
            config.repos[0].index,
            Some(PathBuf::from("artifacts/a.portable.json"))
        );
        assert_eq!(config.repos[1].index, None, "index stays optional");
    }

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
