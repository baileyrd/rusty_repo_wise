//! `repowise doctor` — setup diagnostics (issue #240).
//!
//! This port has a growing number of environment-dependent, degrade-softly
//! paths, and today each one only reveals itself when you happen to run the
//! command that needs it: `hotspots`/`ownership`/`coupled`/`risk` all quietly
//! do less without git history, and both the linked-issue bug-fix heuristic
//! and the PR-body decision source silently fall back to weaker signals
//! without `REPOWISE_GITHUB_TOKEN`.
//!
//! `doctor` collects those into one place. It is **diagnostic only** — it
//! reports, it never mutates state.
//!
//! The pass/warn/fail split matters here: a degraded-but-working setup is a
//! **warn**, never a fail. Missing an optional token is not an error, and
//! reporting it as one would train people to ignore the whole command.

use repowise_core::RepoIndex;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    /// Working, but with a capability unavailable or degraded. Never an
    /// error -- see the module doc.
    Warn,
    Fail,
}

impl Verdict {
    fn label(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Warn => "warn",
            Verdict::Fail => "FAIL",
        }
    }
}

#[derive(Debug)]
pub struct Check {
    pub name: &'static str,
    pub verdict: Verdict,
    pub detail: String,
    /// What to do about it. Empty for a passing check.
    pub remedy: String,
}

impl Check {
    fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Check {
            name,
            verdict: Verdict::Pass,
            detail: detail.into(),
            remedy: String::new(),
        }
    }
    fn warn(name: &'static str, detail: impl Into<String>, remedy: impl Into<String>) -> Self {
        Check {
            name,
            verdict: Verdict::Warn,
            detail: detail.into(),
            remedy: remedy.into(),
        }
    }
    fn fail(name: &'static str, detail: impl Into<String>, remedy: impl Into<String>) -> Self {
        Check {
            name,
            verdict: Verdict::Fail,
            detail: detail.into(),
            remedy: remedy.into(),
        }
    }
}

pub fn run_checks(root: &Path) -> Vec<Check> {
    let mut checks = vec![check_git_binary(), check_git_repo(root)];
    if let Some(c) = check_shallow(root) {
        checks.push(c);
    }
    checks.push(check_index(root));
    checks.push(check_github_token());
    checks.push(check_llm_endpoint());
    checks.push(check_webhook_secret());
    checks.push(check_workspace_env());
    checks
}

fn check_git_binary() -> Check {
    match Command::new("git").arg("--version").output() {
        Ok(o) if o.status.success() => Check::pass(
            "git available",
            String::from_utf8_lossy(&o.stdout).trim().to_string(),
        ),
        _ => Check::warn(
            "git available",
            "the `git` binary could not be run",
            "install git -- hotspots/ownership/coupled/risk and ADR commit mining need it",
        ),
    }
}

fn check_git_repo(root: &Path) -> Check {
    if root.join(".git").exists() {
        Check::pass("git repository", "`.git` present")
    } else {
        Check::warn(
            "git repository",
            "not a git repository",
            "git-history analytics (hotspots/ownership/coupled/risk) report nothing here",
        )
    }
}

/// A shallow clone silently truncates every history-derived number --
/// churn, hotspots, co-change. Worth surfacing because the commands
/// still "work", they just quietly under-report.
fn check_shallow(root: &Path) -> Option<Check> {
    if !root.join(".git").exists() {
        return None;
    }
    let shallow = root.join(".git").join("shallow").exists();
    Some(if shallow {
        Check::warn(
            "git history depth",
            "shallow clone -- churn, hotspots, and co-change see only part of history",
            "`git fetch --unshallow` for complete history-derived numbers",
        )
    } else {
        Check::pass("git history depth", "full history")
    })
}

fn check_index(root: &Path) -> Check {
    match RepoIndex::load(root) {
        Ok(index) => Check::pass("index", format!("{} file(s) indexed", index.files.len())),
        Err(_) => Check::fail(
            "index",
            "no readable index at .repowise/index.json",
            "run `repowise init` -- most commands need it",
        ),
    }
}

fn check_github_token() -> Check {
    match std::env::var("REPOWISE_GITHUB_TOKEN") {
        Ok(t) if !t.is_empty() => Check::pass(
            "REPOWISE_GITHUB_TOKEN",
            "set -- linked-issue bug-fix detection and PR-body decision mining enabled",
        ),
        _ => Check::warn(
            "REPOWISE_GITHUB_TOKEN",
            "not set -- bug-fix detection falls back to commit-message keywords only, \
             and the PR-body decision source is skipped",
            "optional: set it to enable the stronger GitHub-backed signals",
        ),
    }
}

fn check_llm_endpoint() -> Check {
    match std::env::var("REPOWISE_LLM_BASE_URL") {
        Ok(v) if !v.is_empty() => Check::pass("REPOWISE_LLM_BASE_URL", format!("set to {v}")),
        _ => Check::warn(
            "REPOWISE_LLM_BASE_URL",
            "not set -- `repowise generate` (LLM wiki summaries) is unavailable",
            "optional: every other command is deterministic and needs no LLM",
        ),
    }
}

/// Only relevant to `repowise serve-dashboard` (issue #335's GitHub/
/// GitLab webhook receivers) -- most `repowise doctor` checks apply to
/// every command, this one to a single opt-in server flag, but it's
/// still worth surfacing here rather than only failing loudly the first
/// time someone points a forge's webhook at an unconfigured server.
/// `REPOWISE_WORKSPACE` (issue #333): how `serve`/`serve-dashboard`
/// find a workspace file when no `--workspace` flag is passed -- the
/// only route available to the Claude Code plugin, whose `.mcp.json`
/// args are static.
fn check_workspace_env() -> Check {
    match std::env::var("REPOWISE_WORKSPACE") {
        Ok(v) if !v.trim().is_empty() => {
            let path = std::path::PathBuf::from(v.trim());
            if path.exists() {
                Check::pass(
                    "REPOWISE_WORKSPACE",
                    format!("set to {} -- workspace tools and `repo=` are available to `serve`/`serve-dashboard` without the flag", path.display()),
                )
            } else {
                // A hard fail, not a warn: `serve` refuses to start on
                // this rather than quietly running single-repo, so
                // `doctor` must not imply it is merely suboptimal.
                Check::fail(
                    "REPOWISE_WORKSPACE",
                    format!("set to {}, which does not exist", path.display()),
                    "unset it, or point it at a workspace TOML file -- `serve` and \
                     `serve-dashboard` refuse to start rather than quietly running single-repo",
                )
            }
        }
        _ => Check::warn(
            "REPOWISE_WORKSPACE",
            "not set -- `serve`/`serve-dashboard` run single-repo unless given `--workspace`, \
             so MCP tools' `repo` parameter and `list_repos` report no workspace configured",
            "optional: only needed for multi-repo workspaces",
        ),
    }
}

fn check_webhook_secret() -> Check {
    match std::env::var("REPOWISE_WEBHOOK_SECRET") {
        Ok(v) if !v.is_empty() => Check::pass(
            "REPOWISE_WEBHOOK_SECRET",
            "set -- `serve-dashboard`'s GitHub/GitLab webhook receivers are enabled",
        ),
        _ => Check::warn(
            "REPOWISE_WEBHOOK_SECRET",
            "not set -- `serve-dashboard`'s /api/webhook/github and /api/webhook/gitlab \
             report 503; the post-commit hook and `repowise watch` are unaffected",
            "optional: only needed for webhook-triggered auto-sync via a running server",
        ),
    }
}

/// True when any check hard-failed. Drives the exit code -- warns
/// deliberately do not, since a degraded setup is still a working one.
pub fn any_failed(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.verdict == Verdict::Fail)
}

pub fn render(checks: &[Check], root: &Path) -> String {
    let mut out = format!("Repowise doctor for {}\n", root.display());
    for c in checks {
        out.push_str(&format!(
            "  [{}] {:<24} {}\n",
            c.verdict.label(),
            c.name,
            c.detail
        ));
        if !c.remedy.is_empty() {
            out.push_str(&format!("         -> {}\n", c.remedy));
        }
    }

    let warns = checks.iter().filter(|c| c.verdict == Verdict::Warn).count();
    let fails = checks.iter().filter(|c| c.verdict == Verdict::Fail).count();
    out.push_str(&match (fails, warns) {
        (0, 0) => "  all checks passed\n".to_string(),
        (0, w) => format!("  {w} warning(s) -- setup works, some capabilities degraded\n"),
        (f, w) => format!("  {f} failure(s), {w} warning(s)\n"),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_degraded_setup_warns_rather_than_fails() {
        // Missing an optional token must never read as an error --
        // otherwise people learn to ignore doctor entirely.
        let checks = vec![
            Check::pass("git available", "git version 2.0"),
            Check::warn("REPOWISE_GITHUB_TOKEN", "not set", "optional"),
        ];
        assert!(!any_failed(&checks));
        let out = render(&checks, Path::new("/repo"));
        assert!(out.contains("1 warning(s)"), "{out}");
        assert!(out.contains("setup works"), "{out}");
        assert!(!out.contains("FAIL"), "{out}");
    }

    #[test]
    fn a_missing_index_is_a_hard_failure() {
        let checks = vec![Check::fail(
            "index",
            "no readable index",
            "run `repowise init`",
        )];
        assert!(any_failed(&checks));
        let out = render(&checks, Path::new("/repo"));
        assert!(out.contains("FAIL"), "{out}");
        assert!(out.contains("1 failure(s)"), "{out}");
    }

    #[test]
    fn every_non_passing_check_carries_a_remedy() {
        let checks = vec![
            Check::warn("a warn", "detail", "do this"),
            Check::fail("a fail", "detail", "do that"),
        ];
        let out = render(&checks, Path::new("/repo"));
        assert!(out.contains("-> do this"), "{out}");
        assert!(out.contains("-> do that"), "{out}");
    }

    #[test]
    fn a_clean_run_says_so() {
        let checks = vec![Check::pass("git available", "git version 2.0")];
        let out = render(&checks, Path::new("/repo"));
        assert!(out.contains("all checks passed"), "{out}");
        // A passing check has no remedy arrow.
        assert!(!out.contains("->"), "{out}");
    }

    #[test]
    fn shallow_check_is_skipped_entirely_outside_a_git_repo() {
        // Reporting "full history" for a directory with no git at all
        // would be a misleading pass.
        let root = std::env::temp_dir().join("repowise-doctor-test-nogit");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(check_shallow(&root).is_none());
    }

    #[test]
    fn shallow_clone_warns_that_history_numbers_under_report() {
        let root: PathBuf = std::env::temp_dir().join("repowise-doctor-test-shallow");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git").join("shallow"), "abc123\n").unwrap();

        let check = check_shallow(&root).expect("git repo should produce a depth check");
        assert_eq!(check.verdict, Verdict::Warn);
        assert!(check.detail.contains("shallow"), "{}", check.detail);
    }
}
