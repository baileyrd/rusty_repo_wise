//! Multi-repo workspace configuration -- the first slice of issue #64
//! (`get_architecture`/`get_blast_radius`/`list_repos` MCP tools, and
//! the dashboard's workspace views). This crate delivers only the
//! smallest useful piece: naming a set of repo roots and reporting each
//! one's indexed status. `get_architecture`/`get_blast_radius`/the
//! system-map/conformance/contracts/co-changes dashboard views all need
//! real cross-repo dependency resolution (a symbol in one repo
//! resolving as an import/call target in another), which doesn't exist
//! anywhere in this port yet and is deliberately left for a follow-up.
//!
//! A workspace is a small standalone TOML file naming member repos by
//! name and path -- pointed at via a `--workspace <path>` flag on
//! `repowise serve`/`serve-dashboard`/the new `workspace-repos`
//! subcommand, never inferred from or stored inside any member repo's
//! own `.repowise/` directory (a workspace spans repos; no one member
//! repo is a sensible owner of it). Deliberately kept as its own crate
//! rather than folded into `repowise-core`: a future cross-repo slice
//! will need this crate to depend on `repowise-graph`, and
//! `repowise-core` staying dependency-free of every other `repowise-*`
//! crate is a load-bearing invariant the rest of this port relies on.

use repowise_core::RepoIndex;
use serde::Deserialize;
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
