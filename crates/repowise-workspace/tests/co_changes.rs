use repowise_workspace::{workspace_co_changes, ResolvedWorkspaceRepo};
use std::path::Path;
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

fn init_repo(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "Default Author"]);
    git(dir, &["config", "user.email", "default@example.com"]);
}

#[test]
fn workspace_co_changes_reports_each_repos_own_coupling() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);

    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    std::fs::write(root.join("b.txt"), "b\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "Add a and b together"]);

    let repo = ResolvedWorkspaceRepo {
        name: "test".to_string(),
        path: root.clone(),
    };

    let reports = workspace_co_changes(&[repo], 10);

    assert_eq!(reports.len(), 1);
    assert!(reports[0].available);
    assert_eq!(reports[0].pairs.len(), 1);
    assert_eq!(reports[0].pairs[0].count, 1);
    let expected = [root.join("a.txt"), root.join("b.txt")];
    assert!(expected.contains(&reports[0].pairs[0].file_a));
    assert!(expected.contains(&reports[0].pairs[0].file_b));
}

#[test]
fn workspace_co_changes_reports_unavailable_for_a_non_git_repo() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let repo = ResolvedWorkspaceRepo {
        name: "not-git".to_string(),
        path: root,
    };

    let reports = workspace_co_changes(&[repo], 10);

    assert_eq!(reports.len(), 1);
    assert!(!reports[0].available);
    assert!(reports[0].pairs.is_empty());
}

#[test]
fn workspace_co_changes_respects_the_top_n_limit_per_repo() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);

    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    std::fs::write(root.join("b.txt"), "b\n").unwrap();
    std::fs::write(root.join("c.txt"), "c\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "Add a, b, c together"]);

    let repo = ResolvedWorkspaceRepo {
        name: "test".to_string(),
        path: root,
    };

    let reports = workspace_co_changes(&[repo], 1);

    assert_eq!(reports[0].pairs.len(), 1);
}
