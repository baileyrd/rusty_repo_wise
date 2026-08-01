//! Exercises `collect_recent_commits` against a real, disposable git
//! repo built with the `git` CLI directly.

use repowise_git::collect_recent_commits;
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
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "Default Author"]);
    git(dir, &["config", "user.email", "default@example.com"]);
}

fn commit(dir: &Path, file: &str, contents: &str, message: &str) {
    std::fs::write(dir.join(file), contents).unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "-q", "-m", message]);
}

#[test]
fn collect_recent_commits_returns_newest_first_bounded_by_limit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);
    commit(&root, "a.txt", "one\n", "first");
    commit(&root, "a.txt", "one\ntwo\n", "second");
    commit(&root, "a.txt", "one\ntwo\nthree\n", "third");

    let commits = collect_recent_commits(&root, 2).unwrap();

    assert_eq!(commits.len(), 2, "{commits:?}");
    assert_eq!(commits[0].message, "third");
    assert_eq!(commits[1].message, "second");
}

#[test]
fn collect_recent_commits_reports_the_files_each_commit_touched() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);
    commit(&root, "a.txt", "one\n", "add a");

    let commits = collect_recent_commits(&root, 10).unwrap();

    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].files.len(), 1);
    assert_eq!(commits[0].files[0], root.join("a.txt"));
}

#[test]
fn collect_recent_commits_never_returns_more_than_the_full_history() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);
    commit(&root, "a.txt", "one\n", "only commit");

    let commits = collect_recent_commits(&root, 50).unwrap();

    assert_eq!(commits.len(), 1);
}
