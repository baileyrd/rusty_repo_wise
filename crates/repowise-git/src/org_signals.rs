//! Assembles [`OrgSignals`] from data this crate already computes, for
//! `repowise-health`'s fixed-penalty Organizational-signal markers
//! (issue #313, split from #62). No new git plumbing beyond one extra
//! `git log` walk (for the recent-author window) and one `git blame`
//! per indexed file (for bus factor, via the existing [`crate::ownership_of`]);
//! everything else reuses [`GitAnalytics`]'s already-computed data.
//!
//! # Not a cheap call
//!
//! Measured at ~3.5 seconds for `git blame` alone across this port's
//! own ~90 tracked Rust files -- comparable to `repowise_health::
//! find_near_duplicates`'s own cost (#304), and for the same reason:
//! a per-file external-tool invocation multiplied by file count. This
//! is meant for `repowise health`/`get_health`'s full-repo report, which
//! already isn't a cheap lookup, not for anything called per-request on
//! a hot path.

use crate::GitAnalytics;
use repowise_core::org_signals::OrgSignals;
use repowise_core::RepoIndex;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

/// Minimum co-change count for a pair to count as coupled at all. Below
/// this, two files landing in the same commit once or twice is noise,
/// not a signal -- the same floor the reference applies before treating
/// co-change as meaningful.
pub const MIN_CO_CHANGE: usize = 3;

/// How many days of commit history count as "recent" for
/// `developer_congestion`. Matches `GitAnalytics`'s own hotspot-decay
/// half-life, so "recent" means the same thing across every
/// recency-based signal this crate computes.
pub const RECENT_WINDOW_DAYS: f64 = 90.0;

const SECONDS_PER_DAY: f64 = 86_400.0;

/// Build [`OrgSignals`] for every file in `index`.
///
/// Ownership (and therefore bus factor) degrades to "no data" (an empty
/// blame, giving `bus_factor_of` == 0) per file rather than failing the
/// whole call -- a file `git blame` can't resolve (binary, moved,
/// gitattributes-excluded) shouldn't take every other file's signal down
/// with it, the same per-file resilience `repowise-adr`'s sources use.
pub fn collect_org_signals(
    root: &Path,
    index: &RepoIndex,
    analytics: &GitAnalytics,
) -> anyhow::Result<OrgSignals> {
    let mut churn = BTreeMap::new();
    let mut bugfix_commits = BTreeMap::new();
    let mut bus_factor = BTreeMap::new();
    let mut co_change_partner_count = BTreeMap::new();

    for file in &index.files {
        let path = &file.path;
        churn.insert(path.clone(), analytics.churn_of(path));
        bugfix_commits.insert(path.clone(), analytics.bugfix_commits_of(path));

        let ownership = crate::ownership_of(root, path).unwrap_or_default();
        bus_factor.insert(path.clone(), crate::bus_factor(&ownership));

        let partners = analytics
            .coupled_files(path, usize::MAX)
            .into_iter()
            .filter(|(_, count)| *count >= MIN_CO_CHANGE)
            .count();
        co_change_partner_count.insert(path.clone(), partners);
    }

    let co_changed_pairs = analytics
        .top_co_changed_pairs(usize::MAX)
        .into_iter()
        .filter(|(_, _, count)| *count >= MIN_CO_CHANGE)
        .collect();

    let recent_author_count = recent_authors_per_file(root)?;

    Ok(OrgSignals {
        churn,
        bugfix_commits,
        bus_factor,
        co_change_partner_count,
        co_changed_pairs,
        recent_author_count,
    })
}

/// Distinct authors per file across commits within
/// [`RECENT_WINDOW_DAYS`] of collection time.
///
/// A second `git log` walk rather than folding this into
/// `GitAnalytics::collect` -- that struct only retains aggregates, not
/// per-commit author/timestamp detail, and re-walking history here (the
/// same thing `repowise_adr::mine` already does via
/// `repowise_git::collect_commits` for its own commit-message mining)
/// is simpler than widening `GitAnalytics`'s internals for one signal.
fn recent_authors_per_file(root: &Path) -> anyhow::Result<BTreeMap<std::path::PathBuf, usize>> {
    let commits = crate::collect_commits(root)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let cutoff = now - (RECENT_WINDOW_DAYS * SECONDS_PER_DAY) as i64;

    let mut authors: HashMap<std::path::PathBuf, HashSet<String>> = HashMap::new();
    for commit in &commits {
        if commit.timestamp < cutoff {
            continue;
        }
        for file in &commit.files {
            authors
                .entry(file.clone())
                .or_default()
                .insert(commit.author.clone());
        }
    }
    Ok(authors.into_iter().map(|(f, a)| (f, a.len())).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env_remove("GIT_AUTHOR_NAME")
            .env_remove("GIT_AUTHOR_EMAIL")
            .env_remove("GIT_COMMITTER_NAME")
            .env_remove("GIT_COMMITTER_EMAIL")
            .output()
            .expect("failed to run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn commit(dir: &Path, name: &str, body: &str, author: &str, message: &str) {
        std::fs::write(dir.join(name), body).unwrap();
        git(dir, &["add", name]);
        git(
            dir,
            &[
                "-c",
                &format!("user.name={author}"),
                "-c",
                &format!("user.email={author}@example.com"),
                "commit",
                "-q",
                "-m",
                message,
            ],
        );
    }

    fn index_of(root: &Path, files: &[&str]) -> RepoIndex {
        RepoIndex {
            root: root.to_path_buf(),
            files: files
                .iter()
                .map(|f| repowise_core::FileRecord {
                    path: root.join(f),
                    language: repowise_core::Language::Rust,
                    lines: 1,
                    symbols: vec![],
                    imports: vec![],
                    calls: vec![],
                    field_accesses: vec![],
                })
                .collect(),
            other_files: 0,
            indexed_commit: None,
        }
    }

    #[test]
    fn churn_and_bugfix_commits_match_the_underlying_analytics() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q"]);
        commit(&root, "a.rs", "one", "Alice", "add a");
        commit(&root, "a.rs", "two", "Alice", "fix: crash in a");

        let analytics = GitAnalytics::collect(&root).unwrap();
        let index = index_of(&root, &["a.rs"]);
        let signals = collect_org_signals(&root, &index, &analytics).unwrap();

        let a = root.join("a.rs");
        assert_eq!(signals.churn_of(&a), 2);
        assert_eq!(signals.bugfix_commits_of(&a), 1);
        // One author, all lines -- bus factor 1.
        assert_eq!(signals.bus_factor_of(&a), 1);
    }

    #[test]
    fn a_file_with_no_git_history_gets_zeroed_signals_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q"]);
        commit(&root, "a.rs", "one", "Alice", "add a");

        let analytics = GitAnalytics::collect(&root).unwrap();
        // "untracked.rs" is indexed but was never committed.
        let index = index_of(&root, &["a.rs", "untracked.rs"]);
        let signals = collect_org_signals(&root, &index, &analytics).unwrap();

        let untracked = root.join("untracked.rs");
        assert_eq!(signals.churn_of(&untracked), 0);
        assert_eq!(signals.bugfix_commits_of(&untracked), 0);
        assert_eq!(signals.bus_factor_of(&untracked), 0);
    }

    #[test]
    fn co_change_below_the_floor_is_not_a_partner() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q"]);
        // a.rs and b.rs co-change together twice -- below MIN_CO_CHANGE.
        for i in 0..2 {
            std::fs::write(root.join("a.rs"), format!("v{i}")).unwrap();
            std::fs::write(root.join("b.rs"), format!("v{i}")).unwrap();
            git(&root, &["add", "a.rs", "b.rs"]);
            git(
                &root,
                &[
                    "-c",
                    "user.name=Alice",
                    "-c",
                    "user.email=alice@example.com",
                    "commit",
                    "-q",
                    "-m",
                    &format!("update both {i}"),
                ],
            );
        }

        let analytics = GitAnalytics::collect(&root).unwrap();
        let index = index_of(&root, &["a.rs", "b.rs"]);
        let signals = collect_org_signals(&root, &index, &analytics).unwrap();

        assert_eq!(signals.co_change_partner_count_of(&root.join("a.rs")), 0);
        assert!(
            signals.co_changed_pairs.is_empty(),
            "{:?}",
            signals.co_changed_pairs
        );
    }

    #[test]
    fn co_change_at_or_above_the_floor_is_a_partner_and_a_pair() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q"]);
        for i in 0..MIN_CO_CHANGE {
            std::fs::write(root.join("a.rs"), format!("v{i}")).unwrap();
            std::fs::write(root.join("b.rs"), format!("v{i}")).unwrap();
            git(&root, &["add", "a.rs", "b.rs"]);
            git(
                &root,
                &[
                    "-c",
                    "user.name=Alice",
                    "-c",
                    "user.email=alice@example.com",
                    "commit",
                    "-q",
                    "-m",
                    &format!("update both {i}"),
                ],
            );
        }

        let analytics = GitAnalytics::collect(&root).unwrap();
        let index = index_of(&root, &["a.rs", "b.rs"]);
        let signals = collect_org_signals(&root, &index, &analytics).unwrap();

        assert_eq!(signals.co_change_partner_count_of(&root.join("a.rs")), 1);
        assert_eq!(signals.co_changed_pairs.len(), 1);
    }

    #[test]
    fn recent_authors_excludes_authors_outside_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q"]);
        // Alice's commit uses the real current time (the default), so
        // it's inside the recent window regardless of when this test
        // runs.
        commit(&root, "a.rs", "one", "Alice", "add a");

        // Bob's edit is backdated to a fixed date far in the past --
        // `--date` sets the *author* date, which is what
        // `CommitInfo.timestamp` (and this aggregation) reads, so this
        // is deterministic without depending on the test's own runtime.
        std::fs::write(root.join("a.rs"), "two").unwrap();
        git(&root, &["add", "a.rs"]);
        git(
            &root,
            &[
                "-c",
                "user.name=Bob",
                "-c",
                "user.email=bob@example.com",
                "commit",
                "-q",
                "-m",
                "old edit",
                "--date",
                "2000-01-01T00:00:00",
            ],
        );

        let analytics = GitAnalytics::collect(&root).unwrap();
        let index = index_of(&root, &["a.rs"]);
        let signals = collect_org_signals(&root, &index, &analytics).unwrap();

        assert_eq!(
            signals.recent_author_count_of(&root.join("a.rs")),
            1,
            "only Alice's commit is within the recent window"
        );
    }
}
