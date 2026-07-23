use crate::Ownership;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Run `git blame --line-porcelain` on `file` and tally lines per author.
/// `--line-porcelain` (unlike plain `--porcelain`) repeats full commit
/// metadata for every line, so counting `author `-prefixed lines gives an
/// exact per-author line count.
pub fn blame_file(root: &Path, file: &Path) -> anyhow::Result<Vec<Ownership>> {
    let rel = file.strip_prefix(root).unwrap_or(file);
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("blame")
        .arg("--line-porcelain")
        .arg(rel)
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git blame failed for {}: {}",
            file.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut total = 0usize;
    for line in text.lines() {
        if let Some(author) = line.strip_prefix("author ") {
            *counts.entry(author.to_string()).or_insert(0) += 1;
            total += 1;
        }
    }

    let mut out: Vec<Ownership> = counts
        .into_iter()
        .map(|(author, lines)| {
            let percentage = if total > 0 {
                (lines as f64 / total as f64) * 100.0
            } else {
                0.0
            };
            Ownership {
                author,
                lines,
                percentage,
            }
        })
        .collect();
    out.sort_by_key(|o| std::cmp::Reverse(o.lines));
    Ok(out)
}
