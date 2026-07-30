//! Best-effort architectural-decision mining: extracts decisions from
//! eight sources — `docs/adr/*.md` files, commit messages, merged PR
//! bodies, decision-like code comments, explicit inline decision markers
//! (`WHY:`, `DECISION:`, etc.), keep-a-changelog-style CHANGELOG
//! sections, decisions a model inferred from code during `repowise
//! generate`, and decisions the user typed in directly via `repowise
//! decide` — links each to the indexed files/symbols its body mentions
//! (or, for PRs, the files the GitHub API reports it actually touched;
//! or, for code comments/inline markers, the file the comment sits in),
//! and tracks supersession via an ADR's
//! `Status: ... Superseded by ADR-XXXX` line.
//!
//! All 8 of the original repowise's decision sources are now implemented
//! here (see #315/#66 for the `cli` source's history: split off from a
//! bundled issue whose other half, transcript-mining a coding agent's
//! `session`, stays not planned — this port has no such transcript
//! format to mine). The PR-body source is the one place this crate makes
//! a network call at all, and only when a `REPOWISE_GITHUB_TOKEN` env var
//! is set — see the `pull_requests` module doc comment for why that's an
//! explicit opt-in rather than an unauthenticated fallback.
//!
//! Seven of the eight read something a person wrote down. The eighth
//! ([`inferred`]) reads what a model guessed, and is labelled as such at
//! every surface that displays it — see that module for why the
//! distinction is load-bearing rather than cosmetic. This crate still
//! makes no LLM call: `repowise-llm` writes the proposals, and this
//! crate only reads the file, so every decision read path stays
//! deterministic.

mod adr_files;
mod changelog;
mod code_comments;
mod commits;
pub mod inferred;
mod inline_markers;
mod linking;
mod manual;
mod pull_requests;

pub use inferred::{InferredDecision, InferredState, InferredStore};
pub use manual::{ManualDecision, ManualDecisionStore};
pub use pull_requests::{parse_github_owner_repo, GITHUB_API_BASE};

use repowise_core::RepoIndex;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionSource {
    Adr {
        file: PathBuf,
    },
    CommitMessage {
        hash: String,
        author: String,
    },
    PullRequest {
        number: u64,
        author: String,
    },
    CodeComment {
        file: PathBuf,
        line: usize,
    },
    InlineMarker {
        file: PathBuf,
        line: usize,
        marker: String,
    },
    Changelog {
        file: PathBuf,
        section: String,
    },
    /// A decision a **model inferred** from code during `repowise
    /// generate`, anchored to text it quoted from `file`.
    ///
    /// The one variant that isn't a written artifact, and the reason
    /// `DecisionSource` is matched exhaustively at every display site
    /// rather than having a catch-all arm: adding this forced every
    /// surface that shows a decision to decide how to label it, which
    /// is the only way "this was inferred" reaches a reader instead of
    /// sitting in a field nobody renders.
    Inferred {
        file: PathBuf,
        line: usize,
        /// The model that produced it. A reader judging an inferred
        /// claim is entitled to know what inferred it.
        model: String,
    },
    /// A decision the user typed in directly via `repowise decide`
    /// (issue #66's `cli` source; its `session` transcript-mining
    /// sibling was rejected as not planned -- see #66's closing comment).
    ///
    /// The most trustworthy of the eight sources: unlike `Inferred`,
    /// there's no model in the loop and nothing to anchor-check, since
    /// this is the user's own stated intent rather than a guess about
    /// what the code implies.
    Manual {
        /// RFC 3339 timestamp of when it was recorded.
        recorded_at: String,
    },
}

impl DecisionSource {
    /// Whether this decision was inferred by a model rather than read
    /// from something a person wrote.
    ///
    /// Exists so callers ask the question by name instead of matching
    /// on a variant list — a display site that forgets to update its
    /// list would silently start presenting inferred claims as mined
    /// ones.
    pub fn is_inferred(&self) -> bool {
        matches!(self, DecisionSource::Inferred { .. })
    }
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
/// decision-like merged PR bodies, decision-like code comments, inline
/// decision markers, keep-a-changelog-style CHANGELOG sections, the
/// LLM-inferred store, and the manually-recorded store under
/// `index.root`, linking each to the files/symbols its body mentions.
/// Missing `docs/adr/`, an unreadable git history, an
/// unavailable/unauthenticated GitHub API, no changelog file, or no
/// inferred/manual store each degrade to an empty result for that source
/// rather than failing the whole call — all eight sources are
/// independent.
///
/// Use [`mine_reporting`] on any path that *displays* decisions: it also
/// returns what the inferred source contributed, which an empty list
/// can't distinguish from that source never having run.
pub fn mine(index: &RepoIndex) -> anyhow::Result<Vec<DecisionRecord>> {
    Ok(mine_reporting(index)?.0)
}

/// [`mine`], plus what the LLM-inferred source contributed and why.
///
/// Separate entry point rather than a change to `mine`'s signature,
/// because most callers only want the records — but the ones that
/// *display* decisions need the state too. An empty contribution from
/// this source is ambiguous between "nothing inferred" and "the pass
/// that infers things never ran", and only a surface that reports the
/// difference lets a reader tell.
pub fn mine_reporting(index: &RepoIndex) -> anyhow::Result<(Vec<DecisionRecord>, InferredState)> {
    let mut records = adr_files::mine_adr_files(&index.root)?;

    let commits = repowise_git::collect_commits(&index.root).unwrap_or_default();
    records.extend(commits::mine_commit_decisions(&commits));

    let token = std::env::var("REPOWISE_GITHUB_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());
    records.extend(mine_pull_requests(&index.root, token.as_deref()).unwrap_or_default());

    records.extend(code_comments::mine_code_comment_decisions(index));
    records.extend(inline_markers::mine_inline_marker_decisions(index));
    records.extend(changelog::mine_changelog_decisions(&index.root));

    // Read from disk like every other source here -- the inference
    // itself happened at `repowise generate` time. No network call, no
    // model, no nondeterminism on this path.
    let (inferred_records, inferred_state) = inferred::mine_inferred_decisions(&index.root);
    records.extend(inferred_records);

    records.extend(manual::mine_manual_decisions(&index.root));

    for record in &mut records {
        // PR, code-comment, and inline-marker decisions are already
        // linked to their real file (the PR's GitHub-reported file
        // list, or the file the comment/marker sits in, set above) —
        // text-matching would only throw that away, so only the
        // text-linked sources (ADR files, commit messages, changelog
        // entries) get run through the linker here. A changelog entry
        // isn't "about" the changelog file itself the way a PR's diff
        // or a comment's enclosing file is, so it gets the same
        // text-matching treatment as ADR files/commit messages instead
        // of an authoritative self-link.
        // An inferred decision is linked to the file its anchor was
        // found in, for the same reason as a code comment: the link is
        // known, so text matching can only lose information.
        if matches!(
            record.source,
            DecisionSource::PullRequest { .. }
                | DecisionSource::CodeComment { .. }
                | DecisionSource::InlineMarker { .. }
                | DecisionSource::Inferred { .. }
        ) {
            continue;
        }
        record.linked_files = linking::link_to_index(&record.body, index);
    }

    Ok((records, inferred_state))
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
