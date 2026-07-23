//! Git-history-derived analytics: churn, bug-fix frequency, co-change
//! coupling, and per-file ownership — plus hotspot scoring that combines
//! churn with the complexity `repowise-parser` already computed.
//!
//! This shells out to the `git` CLI rather than embedding a git
//! implementation: simplest option, and `git` is already a hard
//! dependency of any repo this tool indexes (it's how the repo got here).

mod blame;
mod log;

use repowise_core::RepoIndex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Commit messages containing one of these (case-insensitive) are
/// treated as bug fixes. A heuristic, not ground truth: fixes described
/// without any of these words won't be counted, and any commit that
/// happens to mention one (e.g. "add typo-fixing script") will be.
const BUGFIX_KEYWORDS: &[&str] = &["fix", "bug", "hotfix", "patch"];

/// Skip commits touching more than this many files when building
/// co-change pairs. A huge commit (a rename sweep, a vendor bump) would
/// otherwise flood every touched file's coupling list with noise.
const MAX_COCHANGE_COMMIT_FILES: usize = 50;

/// Git-history analytics for a repository, collected fresh from `git log`
/// / `git blame` output rather than cached — see the README for why.
pub struct GitAnalytics {
    churn: HashMap<PathBuf, usize>,
    bugfix_commits: HashMap<PathBuf, usize>,
    co_change: HashMap<(PathBuf, PathBuf), usize>,
    /// (short hash, author) of the most recent commit known to touch
    /// each file. `git log`'s default order is newest-first, so this is
    /// set on each file's *first* occurrence during the walk.
    last_touch: HashMap<PathBuf, (String, String)>,
    pub commit_count: usize,
}

impl GitAnalytics {
    /// Walk the full commit history of the repo containing `root`.
    pub fn collect(root: &Path) -> anyhow::Result<Self> {
        let commits = log::collect_history(root)?;
        let mut churn: HashMap<PathBuf, usize> = HashMap::new();
        let mut bugfix_commits: HashMap<PathBuf, usize> = HashMap::new();
        let mut co_change: HashMap<(PathBuf, PathBuf), usize> = HashMap::new();
        let mut last_touch: HashMap<PathBuf, (String, String)> = HashMap::new();

        for commit in &commits {
            let is_bugfix = is_bugfix_message(&commit.message);
            for file in &commit.files {
                *churn.entry(file.clone()).or_insert(0) += 1;
                if is_bugfix {
                    *bugfix_commits.entry(file.clone()).or_insert(0) += 1;
                }
                last_touch
                    .entry(file.clone())
                    .or_insert_with(|| (short_hash(&commit.hash), commit.author.clone()));
            }
            if commit.files.len() >= 2 && commit.files.len() <= MAX_COCHANGE_COMMIT_FILES {
                for i in 0..commit.files.len() {
                    for j in (i + 1)..commit.files.len() {
                        let pair = ordered_pair(&commit.files[i], &commit.files[j]);
                        *co_change.entry(pair).or_insert(0) += 1;
                    }
                }
            }
        }

        Ok(GitAnalytics {
            churn,
            bugfix_commits,
            co_change,
            last_touch,
            commit_count: commits.len(),
        })
    }

    pub fn churn_of(&self, file: &Path) -> usize {
        self.churn.get(file).copied().unwrap_or(0)
    }

    pub fn bugfix_commits_of(&self, file: &Path) -> usize {
        self.bugfix_commits.get(file).copied().unwrap_or(0)
    }

    /// (short hash, author) of the most recent commit known to touch
    /// `file`, if any.
    pub fn last_touch_of(&self, file: &Path) -> Option<(&str, &str)> {
        self.last_touch
            .get(file)
            .map(|(hash, author)| (hash.as_str(), author.as_str()))
    }

    /// Files that most often change in the same commit as `file`, most
    /// coupled first.
    pub fn coupled_files(&self, file: &Path, top_n: usize) -> Vec<(PathBuf, usize)> {
        let mut out: Vec<(PathBuf, usize)> = self
            .co_change
            .iter()
            .filter_map(|((a, b), count)| {
                if a == file {
                    Some((b.clone(), *count))
                } else if b == file {
                    Some((a.clone(), *count))
                } else {
                    None
                }
            })
            .collect();
        out.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        out.truncate(top_n);
        out
    }
}

fn ordered_pair(a: &Path, b: &Path) -> (PathBuf, PathBuf) {
    if a <= b {
        (a.to_path_buf(), b.to_path_buf())
    } else {
        (b.to_path_buf(), a.to_path_buf())
    }
}

fn is_bugfix_message(message: &str) -> bool {
    let lower = message.to_lowercase();
    BUGFIX_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

fn short_hash(hash: &str) -> String {
    hash.chars().take(7).collect()
}

/// Per-author share of a file's lines, from `git blame`.
#[derive(Debug, Clone)]
pub struct Ownership {
    pub author: String,
    pub lines: usize,
    pub percentage: f64,
}

/// Blame `file` (an absolute path under `root`) and return per-author
/// ownership, highest share first.
pub fn ownership_of(root: &Path, file: &Path) -> anyhow::Result<Vec<Ownership>> {
    blame::blame_file(root, file)
}

/// A file's hotspot score: churn × total cyclomatic complexity of its
/// symbols. Deterministic, no recency weighting or decay — a simple,
/// legible starting point matching the original repowise's "hotspots =
/// churn × complexity" framing.
#[derive(Debug, Clone)]
pub struct Hotspot {
    pub file: PathBuf,
    pub churn: usize,
    pub total_complexity: usize,
    pub bugfix_commits: usize,
    pub score: usize,
    /// (short hash, author) of the most recent commit touching this file.
    pub last_touch: Option<(String, String)>,
}

/// Rank every indexed file with nonzero churn by hotspot score,
/// highest first.
pub fn hotspots(index: &RepoIndex, analytics: &GitAnalytics) -> Vec<Hotspot> {
    let mut out: Vec<Hotspot> = index
        .files
        .iter()
        .map(|f| {
            let total_complexity: usize = f.symbols.iter().map(|s| s.complexity).sum();
            let churn = analytics.churn_of(&f.path);
            let bugfix_commits = analytics.bugfix_commits_of(&f.path);
            let last_touch = analytics
                .last_touch_of(&f.path)
                .map(|(hash, author)| (hash.to_string(), author.to_string()));
            Hotspot {
                file: f.path.clone(),
                churn,
                total_complexity,
                bugfix_commits,
                score: churn * total_complexity,
                last_touch,
            }
        })
        .filter(|h| h.churn > 0)
        .collect();
    out.sort_by_key(|h| std::cmp::Reverse(h.score));
    out
}
