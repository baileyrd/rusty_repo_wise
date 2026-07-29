//! Post-commit git hook install/uninstall/status — the cheapest slice of
//! the reference repowise's auto-sync story (issue #238).
//!
//! The reference drives auto-sync through five mechanisms (post-commit
//! hook, file watcher, GitHub webhook, GitLab webhook, polling). This
//! implements only the first, deliberately: it's the one that needs no
//! new dependency, no daemon, and no server — just a small shell script
//! written into `.git/hooks/post-commit`.
//!
//! **This never overwrites a hook it didn't write.** A `post-commit`
//! hook is a place users and other tools legitimately put things, so
//! anything without our marker line is treated as foreign and left
//! strictly alone.

use std::path::{Path, PathBuf};

/// Marker identifying a hook this tool wrote. Presence of this exact
/// line is the *only* thing that authorizes `install --force` to
/// overwrite or `uninstall` to delete — see the module doc.
const HOOK_MARKER: &str = "# installed by repowise (repowise hook install)";

/// What we found at `.git/hooks/post-commit`.
#[derive(Debug, PartialEq, Eq)]
pub enum HookState {
    /// No `post-commit` hook at all.
    Absent,
    /// A hook carrying our marker — ours to manage.
    Ours,
    /// A hook that exists but isn't ours. Never touched.
    Foreign,
}

/// The hook body. Runs `repowise update` detached and silenced so a slow
/// index can't delay or fail the commit that triggered it -- a hook that
/// made `git commit` hang would be worse than no auto-sync at all.
fn hook_script() -> String {
    format!(
        "#!/bin/sh\n\
         {HOOK_MARKER}\n\
         # Refresh the repowise index after each commit. Detached and\n\
         # silenced on purpose: git waits for post-commit to exit, so\n\
         # anything slow here would stall every commit.\n\
         (repowise update >/dev/null 2>&1 &) || true\n"
    )
}

/// Resolve the hooks directory for the repo rooted at `root`.
///
/// Returns an error when `.git` is a *file* rather than a directory --
/// that's a worktree or submodule, where the real hooks live elsewhere
/// and blindly creating `.git/hooks` would silently do nothing. Better
/// to say so than to report success for a hook that will never fire.
pub fn hooks_dir(root: &Path) -> anyhow::Result<PathBuf> {
    let dot_git = root.join(".git");
    if !dot_git.exists() {
        anyhow::bail!("{} is not a git repository", root.display());
    }
    if dot_git.is_file() {
        anyhow::bail!(
            "{} is a git worktree or submodule (.git is a file, not a directory) -- \
             its hooks live in the parent repository, so install the hook there instead",
            root.display()
        );
    }
    Ok(dot_git.join("hooks"))
}

fn hook_path(root: &Path) -> anyhow::Result<PathBuf> {
    Ok(hooks_dir(root)?.join("post-commit"))
}

/// Classify whatever is (or isn't) at `path`.
pub fn classify(path: &Path) -> HookState {
    match std::fs::read_to_string(path) {
        Err(_) => HookState::Absent,
        Ok(body) if body.contains(HOOK_MARKER) => HookState::Ours,
        Ok(_) => HookState::Foreign,
    }
}

pub fn install(root: &Path) -> anyhow::Result<String> {
    let path = hook_path(root)?;
    match classify(&path) {
        HookState::Foreign => anyhow::bail!(
            "a post-commit hook already exists at {} and was not written by repowise -- \
             refusing to overwrite it. Remove or merge it by hand, then re-run.",
            path.display()
        ),
        // Re-installing our own hook is idempotent, not an error: it
        // refreshes the script if this tool's hook body has changed.
        HookState::Ours | HookState::Absent => {}
    }

    std::fs::create_dir_all(hooks_dir(root)?)?;
    std::fs::write(&path, hook_script())?;
    make_executable(&path)?;
    Ok(format!("installed post-commit hook at {}", path.display()))
}

pub fn uninstall(root: &Path) -> anyhow::Result<String> {
    let path = hook_path(root)?;
    match classify(&path) {
        HookState::Absent => Ok("no post-commit hook to remove".to_string()),
        HookState::Foreign => anyhow::bail!(
            "the post-commit hook at {} was not written by repowise -- \
             refusing to remove it",
            path.display()
        ),
        HookState::Ours => {
            std::fs::remove_file(&path)?;
            Ok(format!("removed post-commit hook at {}", path.display()))
        }
    }
}

pub fn status(root: &Path) -> anyhow::Result<String> {
    let path = hook_path(root)?;
    Ok(match classify(&path) {
        HookState::Absent => {
            "post-commit hook: not installed -- run `repowise hook install`".to_string()
        }
        HookState::Ours => format!("post-commit hook: installed at {}", path.display()),
        HookState::Foreign => format!(
            "post-commit hook: a foreign hook exists at {} (not written by repowise, \
             left untouched)",
            path.display()
        ),
    })
}

#[cfg(unix)]
fn make_executable(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

/// On Windows git runs hooks through its bundled shell, which doesn't
/// consult a POSIX executable bit, so there's nothing to set.
#[cfg(not(unix))]
fn make_executable(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

/// Marker for the command-rewrite hook script.
const REWRITE_MARKER: &str = "# installed by repowise (repowise hook rewrite install)";

/// Where the rewrite hook script lives.
///
/// Under `.repowise/` rather than `.git/hooks/`: this isn't a git hook.
/// Nothing in git fires on "an agent is about to run a command", so this
/// is a script an agent harness is pointed at, and putting it among the
/// git hooks would imply an integration that doesn't exist.
pub fn rewrite_hook_path(root: &Path) -> PathBuf {
    root.join(".repowise").join("rewrite-command.sh")
}

/// The rewrite hook body.
///
/// Two properties do all the work here, and both are about what happens
/// when this *fails*:
///
/// - It only ever prints a replacement command; it never runs anything
///   itself. A hook that executed would be a second, hidden execution
///   path for every command an agent runs.
/// - Every failure path echoes the original command unchanged. If
///   `repowise` is missing, errors, or returns nonsense, the caller
///   gets back exactly what it passed in. A hook that can break
///   arbitrary commands when repowise has a bug is not shippable --
///   the same fail-open rule `distill` applies to filter errors.
fn rewrite_hook_script() -> String {
    format!(
        "#!/bin/sh\n\
         {REWRITE_MARKER}\n\
         # Reads a command line on stdin, prints the command to actually run.\n\
         #\n\
         # Fail-open by construction: on ANY problem -- repowise missing,\n\
         # a nonzero exit, empty output -- the original command is echoed\n\
         # back unchanged. This sits in front of every command an agent\n\
         # runs, so it must never be able to break one.\n\
         original=$(cat)\n\
         [ -z \"$original\" ] && exit 0\n\
         rewritten=$(printf '%s' \"$original\" | repowise hook rewrite apply 2>/dev/null) || {{\n\
         \x20 printf '%s' \"$original\"\n\
         \x20 exit 0\n\
         }}\n\
         if [ -z \"$rewritten\" ]; then\n\
         \x20 printf '%s' \"$original\"\n\
         else\n\
         \x20 printf '%s' \"$rewritten\"\n\
         fi\n"
    )
}

fn classify_rewrite(path: &Path) -> HookState {
    match std::fs::read_to_string(path) {
        Err(_) => HookState::Absent,
        Ok(body) if body.contains(REWRITE_MARKER) => HookState::Ours,
        Ok(_) => HookState::Foreign,
    }
}

pub fn rewrite_install(root: &Path) -> anyhow::Result<String> {
    let path = rewrite_hook_path(root);
    if classify_rewrite(&path) == HookState::Foreign {
        anyhow::bail!(
            "a script already exists at {} and was not written by repowise -- \
             refusing to overwrite it",
            path.display()
        );
    }
    std::fs::create_dir_all(path.parent().unwrap_or(root))?;
    std::fs::write(&path, rewrite_hook_script())?;
    make_executable(&path)?;
    Ok(format!(
        "installed command-rewrite hook at {}\n\
         \n\
         Point your agent's command hook at it. It reads a command line on stdin\n\
         and prints the command to run -- unchanged unless the command is one of:\n\
         \x20 {}\n\
         Compound commands, pipes, redirects and substitutions are never rewritten.\n\
         Any failure echoes the original command back, so this cannot break a command.",
        path.display(),
        repowise_distill::rewrite::rewritable_programs().join(", ")
    ))
}

pub fn rewrite_uninstall(root: &Path) -> anyhow::Result<String> {
    let path = rewrite_hook_path(root);
    match classify_rewrite(&path) {
        HookState::Absent => Ok("no command-rewrite hook to remove".to_string()),
        HookState::Foreign => anyhow::bail!(
            "the script at {} was not written by repowise -- refusing to remove it",
            path.display()
        ),
        HookState::Ours => {
            std::fs::remove_file(&path)?;
            Ok(format!(
                "removed command-rewrite hook at {}",
                path.display()
            ))
        }
    }
}

pub fn rewrite_status(root: &Path) -> anyhow::Result<String> {
    let path = rewrite_hook_path(root);
    Ok(match classify_rewrite(&path) {
        HookState::Absent => {
            "command-rewrite hook: not installed -- run `repowise hook rewrite install`".to_string()
        }
        HookState::Ours => format!(
            "command-rewrite hook: installed at {}\n\
             rewrites only: {}\n\
             never rewrites: compound commands, pipes, redirects, substitutions",
            path.display(),
            repowise_distill::rewrite::rewritable_programs().join(", ")
        ),
        HookState::Foreign => format!(
            "command-rewrite hook: a foreign script exists at {} (not written by \
             repowise, left untouched)",
            path.display()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a throwaway repo-shaped directory with a real `.git/hooks`.
    fn fake_repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("repowise-hook-test-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".git").join("hooks")).unwrap();
        root
    }

    #[test]
    fn installs_then_reports_installed() {
        let root = fake_repo("install");
        install(&root).unwrap();
        let path = hook_path(&root).unwrap();
        assert_eq!(classify(&path), HookState::Ours);
        assert!(status(&root).unwrap().contains("installed at"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("repowise update"), "{body}");
        // Detached and silenced, so a slow index can't stall the commit.
        assert!(body.contains('&'), "{body}");
    }

    #[test]
    fn reinstalling_our_own_hook_is_idempotent() {
        let root = fake_repo("idempotent");
        install(&root).unwrap();
        install(&root).expect("re-install of our own hook should succeed");
        assert_eq!(classify(&hook_path(&root).unwrap()), HookState::Ours);
    }

    #[test]
    fn uninstall_removes_our_hook_and_is_safe_when_absent() {
        let root = fake_repo("uninstall");
        install(&root).unwrap();
        uninstall(&root).unwrap();
        assert_eq!(classify(&hook_path(&root).unwrap()), HookState::Absent);
        // Removing again is a no-op, not an error.
        assert!(uninstall(&root).unwrap().contains("no post-commit hook"));
    }

    #[test]
    fn refuses_to_overwrite_or_remove_a_foreign_hook() {
        let root = fake_repo("foreign");
        let path = hook_path(&root).unwrap();
        std::fs::write(&path, "#!/bin/sh\necho someone elses hook\n").unwrap();

        assert_eq!(classify(&path), HookState::Foreign);
        assert!(install(&root).is_err(), "must not clobber a foreign hook");
        assert!(uninstall(&root).is_err(), "must not delete a foreign hook");

        // And it's still there, byte for byte.
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("someone elses hook"), "{body}");
        assert!(status(&root).unwrap().contains("foreign"));
    }

    #[test]
    fn errors_outside_a_git_repo() {
        let root = std::env::temp_dir().join("repowise-hook-test-nogit");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let err = install(&root).unwrap_err().to_string();
        assert!(err.contains("not a git repository"), "{err}");
    }

    #[test]
    fn errors_clearly_when_dot_git_is_a_file_worktree_or_submodule() {
        let root = std::env::temp_dir().join("repowise-hook-test-worktree");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".git"), "gitdir: ../.git/worktrees/wt\n").unwrap();
        let err = install(&root).unwrap_err().to_string();
        assert!(err.contains("worktree or submodule"), "{err}");
    }
}
