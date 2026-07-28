//! Line-level diff extraction for `repowise impacted-tests` (issue #242).
//!
//! `change_risk` already resolves a revspec to changed *files* via
//! `--numstat`, but impacted-test selection needs changed *lines*: which
//! specific lines a diff touches, so they can be intersected against the
//! per-test coverage map. This parses `git diff -U0`'s hunk headers,
//! which is the cheapest way to get exactly that.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Lines touched on the *new* side of a diff, per file.
///
/// Deletion-only hunks contribute no new-side lines, which is correct
/// for coverage intersection: a coverage map records lines that exist
/// and ran, and a deleted line exists in neither.
pub type ChangedLines = BTreeMap<PathBuf, BTreeSet<usize>>;

/// Resolve `revspec` (a single commit, or `base..head`; defaults to
/// `HEAD`) to the set of new-side lines it touches, keyed by absolute
/// path under the repo.
pub fn changed_lines(root: &Path, revspec: Option<&str>) -> anyhow::Result<ChangedLines> {
    let revspec = revspec.unwrap_or("HEAD");
    let toplevel = toplevel(root)?;

    // -U0 gives hunk headers with no context lines, so every line the
    // header reports is genuinely part of the change.
    let output = if revspec.contains("..") {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["diff", "-U0", "--no-renames", revspec])
            .output()?
    } else {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["show", "-U0", "--no-renames", "--pretty=format:", revspec])
            .output()?
    };
    if !output.status.success() {
        anyhow::bail!(
            "git diff/show failed for {revspec}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(parse_unified_diff(
        &String::from_utf8_lossy(&output.stdout),
        &toplevel,
    ))
}

fn toplevel(root: &Path) -> anyhow::Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "not a git repository (or no commits yet): {}",
            root.display()
        );
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

/// Parse `+++ b/<path>` markers and `@@ -a,b +c,d @@` hunk headers.
///
/// Split out from the `git` invocation so it can be tested against
/// fixture diffs without a repo.
pub fn parse_unified_diff(diff: &str, toplevel: &Path) -> ChangedLines {
    let mut out: ChangedLines = BTreeMap::new();
    let mut current: Option<PathBuf> = None;

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            let rest = rest.trim();
            // `/dev/null` is a deletion; it has no new-side file.
            current = if rest == "/dev/null" {
                None
            } else {
                // Strip git's `b/` prefix.
                let path = rest.strip_prefix("b/").unwrap_or(rest);
                Some(toplevel.join(path))
            };
        } else if line.starts_with("@@") {
            let Some(path) = current.clone() else {
                continue;
            };
            if let Some((start, count)) = parse_hunk_new_side(line) {
                let entry = out.entry(path).or_default();
                for l in start..start + count {
                    entry.insert(l);
                }
            }
        }
    }
    out
}

/// Extract `(start, count)` from the `+c,d` half of an `@@` header.
/// `+c` with no comma means a single line. `+c,0` is a pure deletion and
/// yields no lines.
fn parse_hunk_new_side(header: &str) -> Option<(usize, usize)> {
    let plus = header.split('+').nth(1)?;
    let spec = plus.split_whitespace().next()?;
    let mut parts = spec.split(',');
    let start: usize = parts.next()?.parse().ok()?;
    let count: usize = match parts.next() {
        Some(c) => c.parse().ok()?,
        None => 1,
    };
    Some((start, count))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,0 +11,2 @@ fn context
+added one
+added two
@@ -40 +42 @@
-old
+new
diff --git a/src/gone.rs b/src/gone.rs
--- a/src/gone.rs
+++ /dev/null
@@ -1,5 +0,0 @@
";

    #[test]
    fn collects_new_side_lines_per_file() {
        let changed = parse_unified_diff(DIFF, Path::new("/repo"));
        let lib = &changed[Path::new("/repo/src/lib.rs")];
        assert!(lib.contains(&11));
        assert!(lib.contains(&12));
        assert!(lib.contains(&42), "single-line hunk without a comma");
        assert!(!lib.contains(&13));
    }

    #[test]
    fn a_deleted_file_contributes_no_new_side_lines() {
        // Its `+++` is /dev/null, so there is no new-side file at all.
        let changed = parse_unified_diff(DIFF, Path::new("/repo"));
        assert!(!changed.contains_key(Path::new("/repo/src/gone.rs")));
    }

    #[test]
    fn a_pure_deletion_hunk_adds_nothing() {
        let diff = "+++ b/src/a.rs\n@@ -5,3 +4,0 @@\n";
        let changed = parse_unified_diff(diff, Path::new("/repo"));
        assert!(changed
            .get(Path::new("/repo/src/a.rs"))
            .is_none_or(|s| s.is_empty()));
    }

    #[test]
    fn parses_both_hunk_header_shapes() {
        assert_eq!(parse_hunk_new_side("@@ -1,2 +3,4 @@"), Some((3, 4)));
        assert_eq!(parse_hunk_new_side("@@ -1 +3 @@"), Some((3, 1)));
        assert_eq!(parse_hunk_new_side("@@ -1,2 +3,0 @@"), Some((3, 0)));
    }

    #[test]
    fn hunk_headers_before_any_file_marker_are_ignored() {
        let changed = parse_unified_diff("@@ -1 +1 @@\n", Path::new("/repo"));
        assert!(changed.is_empty());
    }
}
