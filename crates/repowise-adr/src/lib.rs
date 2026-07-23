//! Best-effort architectural-decision mining: extracts decisions from
//! four sources — `docs/adr/*.md` files, commit messages, merged PR
//! bodies, and decision-like code comments — each matching a
//! decision-like keyword heuristic — links each to the indexed
//! files/symbols its body mentions (or, for PRs, the files the GitHub
//! API reports it actually touched; or, for code comments, the file the
//! comment sits in), and tracks supersession via an ADR's
//! `Status: ... Superseded by ADR-XXXX` line.
//!
//! Only 4 of the original repowise's 8 decision sources are implemented
//! here — a focused subset, not a shallow stub of all 8 (see the README
//! for which are deferred and why). The PR-body source is the one place
//! this crate makes a network call at all, and only when a
//! `REPOWISE_GITHUB_TOKEN` env var is set — see the `pull_requests`
//! module doc comment for why that's an explicit opt-in rather than an
//! unauthenticated fallback.

mod adr_files;
mod code_comments;
mod commits;
mod linking;
mod pull_requests;

pub use pull_requests::{parse_github_owner_repo, GITHUB_API_BASE};

use repowise_core::RepoIndex;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionSource {
    Adr { file: PathBuf },
    CommitMessage { hash: String, author: String },
    PullRequest { number: u64, author: String },
    CodeComment { file: PathBuf, line: usize },
}

#[derive(Debug, Clone)]
pub struct DecisionRecord {
    pub id: String,
    pub title: String,
    pub source: DecisionSource,
    /// Raw `Status:` line value (ADR source only).
    pub status: Option<String>,
    /// Normalized `ADR-XXXX` this decision is superseded by, if its
    /// status line says so.
    pub superseded_by: Option<String>,
    /// Raw `Date:` line value (ADR source only).
    pub date: Option<String>,
    /// Full text used for linking to graph nodes (the whole ADR file,
    /// or the commit message/subject).
    pub body: String,
    pub linked_files: Vec<PathBuf>,
}

impl DecisionRecord {
    pub fn is_superseded(&self) -> bool {
        self.superseded_by.is_some()
    }
}

/// Mine decisions from `docs/adr/*.md`, decision-like commit messages,
/// decision-like merged PR bodies, and decision-like code comments under
/// `index.root`, linking each to the files/symbols its body mentions.
/// Missing `docs/adr/`, an unreadable git history, or an
/// unavailable/unauthenticated GitHub API each degrade to an empty
/// result for that source rather than failing the whole call — all four
/// sources are independent.
pub fn mine(index: &RepoIndex) -> anyhow::Result<Vec<DecisionRecord>> {
    let mut records = adr_files::mine_adr_files(&index.root)?;

    let commits = repowise_git::collect_commits(&index.root).unwrap_or_default();
    records.extend(commits::mine_commit_decisions(&commits));

    let token = std::env::var("REPOWISE_GITHUB_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());
    records.extend(mine_pull_requests(&index.root, token.as_deref()).unwrap_or_default());

    records.extend(code_comments::mine_code_comment_decisions(index));

    for record in &mut records {
        // PR and code-comment decisions are already linked to their
        // real file (the PR's GitHub-reported file list, or the file
        // the comment sits in, set above) — text-matching would only
        // throw that away, so only the text-linked sources (ADR files,
        // commit messages) get run through the linker here.
        if matches!(
            record.source,
            DecisionSource::PullRequest { .. } | DecisionSource::CodeComment { .. }
        ) {
            continue;
        }
        record.linked_files = linking::link_to_index(&record.body, index);
    }

    Ok(records)
}

/// Mine merged PR bodies via the GitHub API, if (and only if) `token` is
/// `Some`, `root` is a git repo with a GitHub-hosted `origin` remote, and
/// the API call succeeds — any one of those failing degrades to an
/// empty result. `token` comes from a `REPOWISE_GITHUB_TOKEN` env var at
/// the `mine` call site; kept as a plain parameter here (rather than
/// reading the env var directly) so this function stays a pure,
/// deterministic unit to test. See the `pull_requests` module doc
/// comment for why a token is required at all rather than falling back
/// to GitHub's unauthenticated API.
fn mine_pull_requests(root: &Path, token: Option<&str>) -> anyhow::Result<Vec<DecisionRecord>> {
    let Some(token) = token else {
        return Ok(Vec::new());
    };
    let Some(remote_url) = git_remote_url(root) else {
        return Ok(Vec::new());
    };
    let Some((owner, repo)) = pull_requests::parse_github_owner_repo(&remote_url) else {
        return Ok(Vec::new());
    };

    let prs = pull_requests::fetch_merged_pull_requests(
        pull_requests::GITHUB_API_BASE,
        &owner,
        &repo,
        Some(token),
    )?;
    Ok(pull_requests::mine_pull_request_decisions(&prs, root))
}

/// The `origin` remote's configured URL, read via `git config --get`
/// rather than `git remote get-url` — the latter applies any configured
/// `url.<base>.insteadOf` rewrites (e.g. a corporate proxy substitution),
/// which is the wrong thing here: this needs the actual GitHub host to
/// know which repo to query, not wherever `insteadOf` happens to
/// redirect fetches/pushes to.
fn git_remote_url(root: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("failed to run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn mine_pull_requests_is_empty_with_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q"]);
        git(
            &root,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/owner/repo.git",
            ],
        );

        // No token given -> no network call is even attempted.
        let records = mine_pull_requests(&root, None).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn mine_pull_requests_is_empty_with_no_git_remote() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q"]);

        // A token is present, but there's no "origin" remote to resolve
        // an owner/repo from.
        let records = mine_pull_requests(&root, Some("fake-token")).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn mine_pull_requests_is_empty_with_a_non_github_remote() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q"]);
        git(
            &root,
            &[
                "remote",
                "add",
                "origin",
                "https://gitlab.com/owner/repo.git",
            ],
        );

        let records = mine_pull_requests(&root, Some("fake-token")).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn git_remote_url_reports_none_without_a_configured_remote() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q"]);

        assert_eq!(git_remote_url(&root), None);
    }

    #[test]
    fn git_remote_url_reports_the_configured_origin() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q"]);
        git(
            &root,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/owner/repo.git",
            ],
        );

        assert_eq!(
            git_remote_url(&root),
            Some("https://github.com/owner/repo.git".to_string())
        );
    }
}
