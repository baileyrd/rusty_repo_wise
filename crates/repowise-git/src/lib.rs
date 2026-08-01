//! Git-history-derived analytics: churn, bug-fix frequency, co-change
//! coupling, and per-file ownership — plus hotspot scoring that combines
//! churn with the complexity `repowise-parser` already computed.
//!
//! This shells out to the `git` CLI rather than embedding a git
//! implementation: simplest option, and `git` is already a hard
//! dependency of any repo this tool indexes (it's how the repo got here).

mod blame;
mod change_risk;
mod changed_lines;
mod issue_refs;
mod log;
pub mod org_signals;

pub use change_risk::{change_risk, ChangeRisk};
pub use changed_lines::{changed_lines, ChangedLines};
pub use log::CommitInfo;
pub use org_signals::collect_org_signals;

use repowise_core::RepoIndex;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Walk the full commit history of the repo containing `root`, exposed
/// for consumers (e.g. `repowise-adr`'s commit-message decision mining)
/// that need raw commit data rather than `GitAnalytics`'s aggregates.
pub fn collect_commits(root: &Path) -> anyhow::Result<Vec<CommitInfo>> {
    log::collect_history(root)
}

/// The `limit` most recent commits, newest first -- cheap even on a
/// long history, unlike [`collect_commits`] (issue #356's dashboard
/// Commits view, and any other "what just happened here" consumer that
/// doesn't need the whole history).
pub fn collect_recent_commits(root: &Path, limit: usize) -> anyhow::Result<Vec<CommitInfo>> {
    log::collect_recent(root, limit)
}

/// Commit messages containing one of these (case-insensitive) are
/// treated as bug fixes. A heuristic, not ground truth: fixes described
/// without any of these words won't be counted, and any commit that
/// happens to mention one (e.g. "add typo-fixing script") will be.
/// Complemented (not replaced) by a stronger, GitHub-issue-reference-based
/// signal -- see `issue_refs` and `linked_bugfix_issue_numbers`.
const BUGFIX_KEYWORDS: &[&str] = &["fix", "bug", "hotfix", "patch"];

/// Skip commits touching more than this many files when building
/// co-change pairs. A huge commit (a rename sweep, a vendor bump) would
/// otherwise flood every touched file's coupling list with noise.
const MAX_COCHANGE_COMMIT_FILES: usize = 50;

/// Half-life (in days) for recency-weighted churn: a commit this many
/// days old contributes half as much as a commit made today, decaying
/// exponentially. 90 days is a deliberately simple, documented choice —
/// long enough that a quarter's worth of steady activity still registers,
/// short enough that a burst of churn from a year ago reads as cold today.
const HOTSPOT_HALF_LIFE_DAYS: f64 = 90.0;
const SECONDS_PER_DAY: f64 = 86_400.0;

/// Git-history analytics for a repository, collected fresh from `git log`
/// / `git blame` output rather than cached — see the README for why.
pub struct GitAnalytics {
    churn: HashMap<PathBuf, usize>,
    /// Sum of `exp(-age_days / HOTSPOT_HALF_LIFE_DAYS)` per commit
    /// touching the file, `age_days` measured from `now` (collection
    /// time) to each commit's author-date. See `decayed_churn_of`.
    decayed_churn: HashMap<PathBuf, f64>,
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
        let token = std::env::var("REPOWISE_GITHUB_TOKEN")
            .ok()
            .filter(|t| !t.is_empty());
        let linked_bugfix_issues = linked_bugfix_issue_numbers(root, &commits, token.as_deref());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let mut churn: HashMap<PathBuf, usize> = HashMap::new();
        let mut decayed_churn: HashMap<PathBuf, f64> = HashMap::new();
        let mut bugfix_commits: HashMap<PathBuf, usize> = HashMap::new();
        let mut co_change: HashMap<(PathBuf, PathBuf), usize> = HashMap::new();
        let mut last_touch: HashMap<PathBuf, (String, String)> = HashMap::new();

        for commit in &commits {
            let is_bugfix = is_bugfix_message(&commit.message)
                || issue_refs::parse_issue_refs(&commit.message)
                    .iter()
                    .any(|n| linked_bugfix_issues.contains(n));
            let age_days = (now - commit.timestamp).max(0) as f64 / SECONDS_PER_DAY;
            let weight = (-age_days / HOTSPOT_HALF_LIFE_DAYS).exp();
            for file in &commit.files {
                *churn.entry(file.clone()).or_insert(0) += 1;
                *decayed_churn.entry(file.clone()).or_insert(0.0) += weight;
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
            decayed_churn,
            bugfix_commits,
            co_change,
            last_touch,
            commit_count: commits.len(),
        })
    }

    pub fn churn_of(&self, file: &Path) -> usize {
        self.churn.get(file).copied().unwrap_or(0)
    }

    /// Recency-weighted churn: each commit touching `file` contributes
    /// `exp(-age_days / HOTSPOT_HALF_LIFE_DAYS)` rather than a flat `1`,
    /// so old activity counts for less than recent activity even when the
    /// raw commit count (`churn_of`) is the same.
    pub fn decayed_churn_of(&self, file: &Path) -> f64 {
        self.decayed_churn.get(file).copied().unwrap_or(0.0)
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

    /// The `top_n` file pairs that most often change together across the
    /// whole walked history, highest coupling count first (alphabetical
    /// tiebreak for determinism). Unlike `coupled_files`, which is scoped
    /// to one file, this ranks every pair in the repo -- the shape a
    /// repo-level "most coupled files" view needs.
    pub fn top_co_changed_pairs(&self, top_n: usize) -> Vec<(PathBuf, PathBuf, usize)> {
        let mut out: Vec<(PathBuf, PathBuf, usize)> = self
            .co_change
            .iter()
            .map(|((a, b), count)| (a.clone(), b.clone(), *count))
            .collect();
        out.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| a.0.cmp(&b.0))
                .then_with(|| a.1.cmp(&b.1))
        });
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

/// Cross-references every `#N` issue reference across `commits`'
/// messages against the GitHub API, returning the subset confirmed
/// closed with a bug-like label. Degrades to an empty set (keyword-only
/// bug-fix detection) when there's no token, no GitHub-hosted `origin`
/// remote, or a lookup fails -- the same "optional data source"
/// tradeoff `repowise-adr`'s PR-body decision mining already makes, kept
/// as a pure function of its inputs (rather than reading the env var
/// itself) so it stays a plain, testable unit.
fn linked_bugfix_issue_numbers(
    root: &Path,
    commits: &[CommitInfo],
    token: Option<&str>,
) -> HashSet<u64> {
    let Some(token) = token else {
        return HashSet::new();
    };
    let Some(remote_url) = git_remote_url(root) else {
        return HashSet::new();
    };
    let Some((owner, repo)) = issue_refs::parse_github_owner_repo(&remote_url) else {
        return HashSet::new();
    };

    let mut referenced: HashSet<u64> = HashSet::new();
    for commit in commits {
        referenced.extend(issue_refs::parse_issue_refs(&commit.message));
    }

    referenced
        .into_iter()
        .filter(|&n| {
            issue_refs::is_closed_bug_issue(issue_refs::GITHUB_API_BASE, &owner, &repo, n, token)
                == Some(true)
        })
        .collect()
}

/// The `origin` remote's configured URL, read via `git config --get`
/// rather than `git remote get-url` for the same reason
/// `repowise-adr`'s copy of this helper does: the latter applies any
/// configured `url.<base>.insteadOf` rewrite, which is the wrong thing
/// here -- this needs the actual GitHub host, not wherever `insteadOf`
/// happens to redirect fetches/pushes to.
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

fn short_hash(hash: &str) -> String {
    hash.chars().take(7).collect()
}

/// Length of the SHA prefix [`head_sha`] returns.
///
/// 12, not the 7 [`short_hash`] uses for display. These two are for
/// different jobs: 7 characters is plenty to eyeball a commit in a list,
/// but this one gets *stored in an index and compared for equality
/// later*, and a prefix collision there would silently report a stale
/// index as current -- the exact failure the comparison exists to catch.
/// 12 matches the reference repowise's `indexed_commit`/`live_head`.
const HEAD_SHA_LEN: usize = 12;

/// The commit `HEAD` currently points at, as a 12-character prefix.
///
/// `None` -- never an error and never a placeholder string -- when git
/// isn't available, `root` isn't a repository, or the repository has no
/// commits yet. Every one of those is a legitimate state for a repo this
/// tool indexes, and callers need to be able to tell "no commit to
/// record" apart from "recorded a commit", because reporting an unknown
/// commit as a mismatch would flag every index in a non-git directory as
/// stale.
pub fn head_sha(root: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sha.len() < HEAD_SHA_LEN {
        // A short-but-nonempty answer means something other than a SHA
        // came back; truncating it would fabricate a plausible-looking
        // commit id out of whatever it actually was.
        return None;
    }
    Some(sha.chars().take(HEAD_SHA_LEN).collect())
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

/// How many weeks of history the activity trend covers.
const TREND_WEEKS: usize = 26;

/// Commit activity aggregates for `GET /api/stats` (issue #262).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitActivity {
    /// `[day][hour]` commit counts. Day 0 is Sunday, hour 0 is midnight,
    /// **both in UTC** -- see [`commit_activity`].
    pub punch_card: Vec<Vec<usize>>,
    /// Commits per week for the last [`TREND_WEEKS`] weeks, oldest
    /// first. The final entry is the week containing `now`.
    pub weekly_trend: Vec<usize>,
    pub commit_count: usize,
}

/// Bucket commit timestamps into a day×hour punch card and a weekly
/// trend.
///
/// **Everything is UTC.** Git stores an author timestamp plus a separate
/// offset; this port only carries the timestamp, so a local-time punch
/// card isn't derivable without data we don't have. Bucketing silently
/// in whatever timezone the *server* happens to run in would be worse
/// than picking one and saying so -- a punch card whose meaning shifts
/// with the host's `TZ` is actively misleading. Callers surface "UTC" in
/// the UI.
///
/// Pure, so it needs no repo and no clock of its own: `now` is passed
/// in, which also keeps it deterministically testable.
pub fn commit_activity(timestamps: &[i64], now: i64) -> CommitActivity {
    let mut punch_card = vec![vec![0usize; 24]; 7];
    let mut weekly_trend = vec![0usize; TREND_WEEKS];

    for &ts in timestamps {
        if ts < 0 {
            continue;
        }
        let days = ts / SECONDS_PER_DAY as i64;
        // 1970-01-01 was a Thursday; +4 shifts the epoch so 0 = Sunday.
        let dow = (((days + 4) % 7) + 7) % 7;
        let hour = (ts % SECONDS_PER_DAY as i64) / 3600;
        punch_card[dow as usize][hour as usize] += 1;

        let weeks_ago = (now - ts).max(0) / (SECONDS_PER_DAY as i64 * 7);
        if (weeks_ago as usize) < TREND_WEEKS {
            // Oldest first, so the last element is the current week.
            let idx = TREND_WEEKS - 1 - weeks_ago as usize;
            weekly_trend[idx] += 1;
        }
    }

    CommitActivity {
        punch_card,
        weekly_trend,
        commit_count: timestamps.len(),
    }
}

/// Whether the repo containing `root` is a shallow clone.
///
/// Worth exposing because a shallow clone doesn't make history-derived
/// numbers *fail*, it makes them quietly under-report -- and an activity
/// trend is exactly where truncated history misleads most.
pub fn is_shallow(root: &Path) -> bool {
    root.join(".git").join("shallow").exists()
}

/// Share of a file's lines that the smallest owning group must hold
/// before `bus_factor` stops counting authors into it.
///
/// 50% — a simple majority. The reference repowise documents "bus
/// factor" in its computed glossary but doesn't publish the threshold it
/// uses, so this picks the most defensible round number rather than
/// inventing a tuned one: the smallest set of people who between them
/// wrote most of the file. Anything higher (say 80%) measures "who wrote
/// nearly all of it", which is a different and less actionable question.
const BUS_FACTOR_SHARE: f64 = 50.0;

/// The smallest number of authors whose combined line share reaches
/// [`BUS_FACTOR_SHARE`] — i.e. how many people would have to leave
/// before most of this file has no author left who has touched it.
///
/// `1` means a single author wrote the majority of the file. A higher
/// number means the knowledge is spread wider.
///
/// Takes the same `Ownership` slice [`ownership_of`] already returns, so
/// this adds no git invocation of its own. Expects that slice ordered
/// highest-share-first (as `ownership_of` returns it) and sorts
/// defensively rather than assuming it.
///
/// Returns `0` for an empty slice: a file with no blameable lines has no
/// bus factor, which is distinct from a file whose bus factor is 1.
pub fn bus_factor(ownership: &[Ownership]) -> usize {
    if ownership.is_empty() {
        return 0;
    }
    let mut shares: Vec<f64> = ownership.iter().map(|o| o.percentage).collect();
    shares.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    let mut cumulative = 0.0;
    for (i, share) in shares.iter().enumerate() {
        cumulative += share;
        if cumulative >= BUS_FACTOR_SHARE {
            return i + 1;
        }
    }
    // Percentages that never reach the threshold (rounding loss, or a
    // blame that didn't attribute every line) — every author is needed.
    shares.len()
}

/// A file's hotspot score: churn × total cyclomatic complexity of its
/// symbols. A simple, legible starting point matching the original
/// repowise's "hotspots = churn × complexity" framing. See `decayed_score`
/// for the recency-weighted variant used to rank results.
#[derive(Debug, Clone)]
pub struct Hotspot {
    pub file: PathBuf,
    pub churn: usize,
    pub total_complexity: usize,
    pub bugfix_commits: usize,
    pub score: usize,
    /// `decayed_churn_of(file) × total_complexity` — the same formula as
    /// `score`, but with recency-weighted churn instead of a raw commit
    /// count, so old activity contributes less than recent activity.
    /// Used to order the results `hotspots()` returns.
    pub decayed_score: f64,
    /// (short hash, author) of the most recent commit touching this file.
    pub last_touch: Option<(String, String)>,
}

/// Rank every indexed file with nonzero churn by (recency-weighted)
/// hotspot score, highest first.
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
                decayed_score: analytics.decayed_churn_of(&f.path) * total_complexity as f64,
                last_touch,
            }
        })
        .filter(|h| h.churn > 0)
        .collect();
    out.sort_by(|a, b| {
        b.decayed_score
            .partial_cmp(&a.decayed_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
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
    fn linked_bugfix_issue_numbers_is_empty_with_no_token() {
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
        let numbers = linked_bugfix_issue_numbers(&root, &[], None);
        assert!(numbers.is_empty());
    }

    #[test]
    fn linked_bugfix_issue_numbers_is_empty_with_no_git_remote() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-q"]);

        let numbers = linked_bugfix_issue_numbers(&root, &[], Some("fake-token"));
        assert!(numbers.is_empty());
    }

    #[test]
    fn linked_bugfix_issue_numbers_is_empty_with_a_non_github_remote() {
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

        let numbers = linked_bugfix_issue_numbers(&root, &[], Some("fake-token"));
        assert!(numbers.is_empty());
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

    fn owners(shares: &[f64]) -> Vec<Ownership> {
        shares
            .iter()
            .enumerate()
            .map(|(i, pct)| Ownership {
                author: format!("author{i}"),
                lines: *pct as usize,
                percentage: *pct,
            })
            .collect()
    }

    #[test]
    fn bus_factor_is_one_when_a_single_author_owns_the_majority() {
        assert_eq!(bus_factor(&owners(&[95.0, 5.0])), 1);
        assert_eq!(bus_factor(&owners(&[100.0])), 1);
        // Exactly at the threshold still counts as reached.
        assert_eq!(bus_factor(&owners(&[50.0, 30.0, 20.0])), 1);
    }

    #[test]
    fn bus_factor_grows_as_ownership_spreads() {
        // 25 each: needs two to clear 50%.
        assert_eq!(bus_factor(&owners(&[25.0, 25.0, 25.0, 25.0])), 2);
        // 20 each: needs three.
        assert_eq!(bus_factor(&owners(&[20.0, 20.0, 20.0, 20.0, 20.0])), 3);
    }

    #[test]
    fn bus_factor_does_not_assume_the_input_is_sorted() {
        assert_eq!(bus_factor(&owners(&[5.0, 95.0])), 1);
        assert_eq!(bus_factor(&owners(&[20.0, 25.0, 30.0, 25.0])), 2);
    }

    #[test]
    fn bus_factor_of_an_unblameable_file_is_zero_not_one() {
        assert_eq!(bus_factor(&[]), 0);
    }

    #[test]
    fn bus_factor_falls_back_to_every_author_when_shares_never_reach_the_threshold() {
        // Partial attribution: shares sum to 30%, never crossing 50%.
        assert_eq!(bus_factor(&owners(&[10.0, 10.0, 10.0])), 3);
    }

    /// 2024-01-07 00:00:00 UTC was a Sunday.
    const SUNDAY_MIDNIGHT: i64 = 1_704_585_600;

    #[test]
    fn punch_card_buckets_by_utc_day_and_hour() {
        let a = commit_activity(&[SUNDAY_MIDNIGHT], SUNDAY_MIDNIGHT);
        assert_eq!(a.punch_card[0][0], 1, "Sunday midnight");

        // +14h same day, and +1 day.
        let b = commit_activity(
            &[SUNDAY_MIDNIGHT + 14 * 3600, SUNDAY_MIDNIGHT + 86_400],
            SUNDAY_MIDNIGHT + 86_400,
        );
        assert_eq!(b.punch_card[0][14], 1, "Sunday 14:00");
        assert_eq!(b.punch_card[1][0], 1, "Monday midnight");
    }

    #[test]
    fn punch_card_covers_a_full_week_and_day() {
        let a = commit_activity(&[], 0);
        assert_eq!(a.punch_card.len(), 7);
        assert!(a.punch_card.iter().all(|d| d.len() == 24));
    }

    #[test]
    fn weekly_trend_puts_the_current_week_last() {
        let now = SUNDAY_MIDNIGHT + 100 * 86_400;
        let week = 7 * 86_400;
        let a = commit_activity(&[now, now - week, now - 2 * week], now);
        let n = a.weekly_trend.len();
        assert_eq!(a.weekly_trend[n - 1], 1, "this week");
        assert_eq!(a.weekly_trend[n - 2], 1, "last week");
        assert_eq!(a.weekly_trend[n - 3], 1, "two weeks ago");
        assert_eq!(a.weekly_trend.iter().sum::<usize>(), 3);
    }

    #[test]
    fn commits_older_than_the_trend_window_are_dropped_from_it_but_still_counted() {
        // They still belong on the punch card -- the window bounds the
        // trend chart, not the history.
        let now = SUNDAY_MIDNIGHT + 1_000 * 86_400;
        let ancient = SUNDAY_MIDNIGHT;
        let a = commit_activity(&[ancient], now);
        assert_eq!(a.weekly_trend.iter().sum::<usize>(), 0);
        assert_eq!(a.commit_count, 1);
        assert_eq!(a.punch_card[0][0], 1);
    }

    #[test]
    fn no_commits_yields_an_empty_but_well_formed_activity() {
        let a = commit_activity(&[], 12_345);
        assert_eq!(a.commit_count, 0);
        assert_eq!(a.weekly_trend.len(), TREND_WEEKS);
        assert!(a.punch_card.iter().flatten().all(|c| *c == 0));
    }

    #[test]
    fn a_future_or_negative_timestamp_does_not_panic() {
        let now = SUNDAY_MIDNIGHT;
        let a = commit_activity(&[-1, now + 86_400 * 999], now);
        assert_eq!(a.commit_count, 2, "still counted");
        // The future commit lands in the current week, not out of bounds.
        assert_eq!(a.weekly_trend[TREND_WEEKS - 1], 1);
    }
}
