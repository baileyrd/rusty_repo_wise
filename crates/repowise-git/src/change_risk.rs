use std::path::{Path, PathBuf};
use std::process::Command;

/// Weight given to each normalized diff-shape component in the final
/// 0-10 score. Fixed, documented, and deliberately simple — see the
/// module doc comment for why this isn't a trained model.
const WEIGHT_FILES: f64 = 0.20;
const WEIGHT_LINES: f64 = 0.25;
const WEIGHT_SUBSYSTEMS: f64 = 0.20;
const WEIGHT_CONCENTRATION: f64 = 0.15;
const WEIGHT_AUTHOR: f64 = 0.20;

/// Component saturation points: the point past which more of a raw
/// metric no longer increases that component's contribution. Chosen as
/// round, legible numbers rather than tuned against any corpus.
const FILES_SATURATION: f64 = 20.0;
const LINES_SATURATION: f64 = 500.0;
const SUBSYSTEMS_SATURATION: f64 = 8.0;
const AUTHOR_EXPERIENCE_SATURATION: f64 = 50.0;

/// Deterministic diff-shape risk assessment for one commit or commit
/// range. A documented fixed-weight heuristic, **not** the reference
/// repowise's ML-calibrated score (see the module doc comment) — treat
/// `score` as a rough approximation, not a calibrated probability.
#[derive(Debug, Clone)]
pub struct ChangeRisk {
    /// The commit range or single commit this was computed against.
    pub revspec: String,
    pub lines_added: usize,
    pub lines_deleted: usize,
    pub files_touched: usize,
    /// Count of distinct top-level path components among the touched
    /// files: the subdirectory name for a nested file, or the file's own
    /// name for a file directly under the repo root (with no directory
    /// to group by, each such file stands in as its own subsystem).
    pub subsystems_touched: usize,
    /// Shannon entropy of the touched files' share of total lines
    /// changed, normalized to `0.0..=1.0` by the maximum possible entropy
    /// for that file count (`log2(files_touched)`). `0.0` means the
    /// change is entirely concentrated in one file; `1.0` means it's
    /// spread perfectly evenly across every touched file.
    pub concentration: f64,
    /// Author of the revspec's head commit (the right-hand side of a
    /// `..` range, or the commit itself for a single revspec).
    pub author: String,
    /// Commits by `author` prior to the head commit, repo-wide. Used as
    /// a proxy for "author experience" with this codebase.
    pub author_prior_commits: usize,
    /// `0.0..=10.0`, higher is riskier. See the module doc comment for
    /// the formula.
    pub score: f64,
}

/// Compute diff-shape metrics for `revspec` (a single commit, or a
/// `base..head` range) against the repo containing `root`, and combine
/// them into a 0-10 heuristic risk score. `revspec` defaults to `"HEAD"`
/// (the most recent commit) when `None`.
pub fn change_risk(root: &Path, revspec: Option<&str>) -> anyhow::Result<ChangeRisk> {
    let revspec = revspec.unwrap_or("HEAD").to_string();
    let toplevel = git_toplevel(root)?;
    let head = revspec
        .rsplit_once("..")
        .map(|(_, h)| h)
        .unwrap_or(&revspec);

    let files = diff_numstat(root, &toplevel, &revspec)?;
    let lines_added: usize = files.iter().map(|f| f.added).sum();
    let lines_deleted: usize = files.iter().map(|f| f.deleted).sum();
    let files_touched = files.len();

    let subsystems_touched = files
        .iter()
        .map(|f| top_level_component(&f.path, &toplevel))
        .collect::<std::collections::HashSet<_>>()
        .len();

    let concentration = concentration_entropy(&files);

    let author = commit_author(root, head)?;
    let author_prior_commits = prior_commits_by_author(root, head, &author)?;

    let score = score_of(
        files_touched,
        lines_added + lines_deleted,
        subsystems_touched,
        concentration,
        author_prior_commits,
    );

    Ok(ChangeRisk {
        revspec,
        lines_added,
        lines_deleted,
        files_touched,
        subsystems_touched,
        concentration,
        author,
        author_prior_commits,
        score,
    })
}

fn score_of(
    files_touched: usize,
    lines_changed: usize,
    subsystems_touched: usize,
    concentration: f64,
    author_prior_commits: usize,
) -> f64 {
    let files_component = (files_touched as f64 / FILES_SATURATION).min(1.0);
    let lines_component = (lines_changed as f64 / LINES_SATURATION).min(1.0);
    let subsystems_component = (subsystems_touched as f64 / SUBSYSTEMS_SATURATION).min(1.0);
    let author_component =
        1.0 - (author_prior_commits as f64 / AUTHOR_EXPERIENCE_SATURATION).min(1.0);

    let raw = WEIGHT_FILES * files_component
        + WEIGHT_LINES * lines_component
        + WEIGHT_SUBSYSTEMS * subsystems_component
        + WEIGHT_CONCENTRATION * concentration
        + WEIGHT_AUTHOR * author_component;

    // raw is in 0.0..=1.0 (weights sum to 1.0); scale to 0..10 and round
    // to one decimal place.
    let score = raw * 10.0;
    (score * 10.0).round() / 10.0
}

struct FileDiff {
    path: PathBuf,
    added: usize,
    deleted: usize,
}

/// `git diff --numstat --no-renames` for a `base..head` range, or
/// `git show --numstat --no-renames` for a single commit. `--no-renames`
/// keeps numstat's output to the simple `<added>\t<deleted>\t<path>`
/// shape — no `{old => new}` rename syntax to parse.
fn diff_numstat(root: &Path, toplevel: &Path, revspec: &str) -> anyhow::Result<Vec<FileDiff>> {
    let output = if revspec.contains("..") {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["diff", "--numstat", "--no-renames", revspec])
            .output()?
    } else {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "show",
                "--numstat",
                "--no-renames",
                "--pretty=format:",
                revspec,
            ])
            .output()?
    };
    if !output.status.success() {
        anyhow::bail!(
            "git diff/show failed for {revspec}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut files = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, '\t');
        let added = parts.next().unwrap_or_default();
        let deleted = parts.next().unwrap_or_default();
        let Some(path) = parts.next() else { continue };
        files.push(FileDiff {
            path: toplevel.join(path),
            added: added.parse().unwrap_or(0),
            deleted: deleted.parse().unwrap_or(0),
        });
    }
    Ok(files)
}

fn top_level_component(path: &Path, toplevel: &Path) -> String {
    let rel = path.strip_prefix(toplevel).unwrap_or(path);
    match rel.components().next() {
        Some(std::path::Component::Normal(part)) => part.to_string_lossy().to_string(),
        _ => ".".to_string(),
    }
}

/// `-sum(p_i * log2(p_i))` over each file's share of total lines
/// changed, normalized by `log2(files_touched)` (the maximum entropy for
/// that many files) so the result is comparable across diffs touching
/// different numbers of files. `0.0` for a diff touching 0 or 1 files —
/// there's no distribution to be uneven or even.
fn concentration_entropy(files: &[FileDiff]) -> f64 {
    if files.len() < 2 {
        return 0.0;
    }
    let total: usize = files.iter().map(|f| f.added + f.deleted).sum();
    if total == 0 {
        return 0.0;
    }
    let entropy: f64 = files
        .iter()
        .map(|f| (f.added + f.deleted) as f64 / total as f64)
        .filter(|p| *p > 0.0)
        .map(|p| -p * p.log2())
        .sum();
    let max_entropy = (files.len() as f64).log2();
    if max_entropy == 0.0 {
        0.0
    } else {
        (entropy / max_entropy).clamp(0.0, 1.0)
    }
}

fn git_toplevel(root: &Path) -> anyhow::Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "not a git repository (or no commits yet): {}\n{}",
            root.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

fn commit_author(root: &Path, revspec: &str) -> anyhow::Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["log", "-1", "--pretty=format:%ae", revspec])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git log failed for {revspec}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Commits by `author_email` reachable from `revspec`, not counting the
/// commit at `revspec` itself. `--author` matches as a regex against
/// each commit's "Name <email>" line, so the email is escaped first —
/// an address containing regex metacharacters (`+`, `.`) shouldn't be
/// able to widen the match or fail the command.
fn prior_commits_by_author(
    root: &Path,
    revspec: &str,
    author_email: &str,
) -> anyhow::Result<usize> {
    if author_email.is_empty() {
        return Ok(0);
    }
    let pattern = escape_regex(author_email);
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-list", "--count", "--author", &pattern, revspec])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git rev-list failed for {revspec}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let total: usize = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    Ok(total.saturating_sub(1))
}

fn escape_regex(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
