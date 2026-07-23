use std::path::{Path, PathBuf};
use std::process::Command;

/// A single commit: its hash, author name, subject line, and the files it
/// touched (paths absolute, resolved against the repo's real top-level
/// directory — see `collect_history`).
pub struct CommitInfo {
    pub hash: String,
    pub author: String,
    pub message: String,
    pub files: Vec<PathBuf>,
}

/// ASCII record/unit separators: virtually never appear in real commit
/// messages/author names, unlike commas or pipes.
const RECORD_SEP: char = '\u{1e}';
const FIELD_SEP: char = '\u{1f}';

/// Resolve the actual git top-level directory for `root`. `git log
/// --name-only` always reports file paths relative to this, even when
/// `root` is a subdirectory of a larger repo.
fn git_toplevel(root: &Path) -> anyhow::Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("--show-toplevel")
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

/// Walk the full commit history of the repo containing `root`, returning
/// one `CommitInfo` per commit with its changed files as absolute paths.
/// Merge commits are included but (per git's default `--name-only`
/// behavior) have no file list, so they don't contribute to churn.
pub fn collect_history(root: &Path) -> anyhow::Result<Vec<CommitInfo>> {
    let toplevel = git_toplevel(root)?;

    let format = format!("--pretty=format:{RECORD_SEP}%H{FIELD_SEP}%an{FIELD_SEP}%s");
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("log")
        .arg(format)
        .arg("--name-only")
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git log failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut commits = Vec::new();
    for record in text.split(RECORD_SEP) {
        if record.trim().is_empty() {
            continue;
        }
        let mut lines = record.lines();
        let header = lines.next().unwrap_or_default();
        let mut parts = header.splitn(3, FIELD_SEP);
        let hash = parts.next().unwrap_or_default().to_string();
        let author = parts.next().unwrap_or_default().to_string();
        let message = parts.next().unwrap_or_default().to_string();
        if hash.is_empty() {
            continue;
        }
        let files = lines
            .filter(|l| !l.trim().is_empty())
            .map(|l| toplevel.join(l))
            .collect();
        commits.push(CommitInfo {
            hash,
            author,
            message,
            files,
        });
    }
    Ok(commits)
}
