//! Exercises `change_risk` against real, disposable git repos built with
//! the `git` CLI directly, so the actual `git show`/`git diff --numstat`/
//! `git rev-list` output shapes are what's under test.

use repowise_git::change_risk;
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

#[test]
fn single_commit_reports_lines_files_and_one_subsystem() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);

    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    git(&root, &["add", "a.txt"]);
    git(&root, &["commit", "-q", "-m", "Add a"]);

    std::fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(&root, &["add", "a.txt"]);
    git(&root, &["commit", "-q", "-m", "Grow a"]);

    // No revspec given -> defaults to HEAD (the most recent commit).
    let risk = change_risk(&root, None).unwrap();
    assert_eq!(risk.revspec, "HEAD");
    assert_eq!(risk.lines_added, 2);
    assert_eq!(risk.lines_deleted, 0);
    assert_eq!(risk.files_touched, 1);
    assert_eq!(risk.subsystems_touched, 1);
    // A single touched file has no distribution to be uneven or even.
    assert_eq!(risk.concentration, 0.0);
    assert_eq!(risk.author, "default@example.com");
}

#[test]
fn commit_range_aggregates_across_commits_and_directories() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("docs")).unwrap();

    std::fs::write(root.join("src/a.rs"), "one\n").unwrap();
    std::fs::write(root.join("docs/readme.md"), "one\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "Initial"]);
    git(&root, &["tag", "base"]);

    std::fs::write(root.join("src/a.rs"), "one\ntwo\n").unwrap();
    git(&root, &["commit", "-q", "-am", "Touch src"]);
    std::fs::write(root.join("docs/readme.md"), "one\ntwo\nthree\n").unwrap();
    git(&root, &["commit", "-q", "-am", "Touch docs"]);

    let risk = change_risk(&root, Some("base..HEAD")).unwrap();
    assert_eq!(risk.revspec, "base..HEAD");
    assert_eq!(risk.lines_added, 3);
    assert_eq!(risk.files_touched, 2);
    // "src" and "docs" are two distinct top-level directories.
    assert_eq!(risk.subsystems_touched, 2);
}

#[test]
fn concentration_is_higher_when_changes_are_spread_evenly() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);

    std::fs::write(root.join("a.txt"), "").unwrap();
    std::fs::write(root.join("b.txt"), "").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "Add empty files"]);

    // Even split: both files get the same number of added lines.
    std::fs::write(root.join("a.txt"), "1\n2\n").unwrap();
    std::fs::write(root.join("b.txt"), "1\n2\n").unwrap();
    git(&root, &["commit", "-q", "-am", "Even change"]);
    let even = change_risk(&root, None).unwrap();
    assert_eq!(even.concentration, 1.0);

    // Lopsided split: almost all the change lands in one file.
    std::fs::write(root.join("a.txt"), "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n").unwrap();
    std::fs::write(root.join("b.txt"), "1\n2\n3\n").unwrap();
    git(&root, &["commit", "-q", "-am", "Lopsided change"]);
    let lopsided = change_risk(&root, None).unwrap();
    assert!(lopsided.concentration < even.concentration);
}

#[test]
fn author_with_no_prior_history_scores_higher_than_an_established_author() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);

    // Default Author builds up a long history of unrelated commits.
    for i in 0..10 {
        std::fs::write(root.join(format!("f{i}.txt")), "x\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", &format!("Commit {i}")]);
    }
    std::fs::write(root.join("shared.txt"), "one\n").unwrap();
    git(&root, &["add", "shared.txt"]);
    git(
        &root,
        &["commit", "-q", "-m", "Established author touches shared"],
    );
    let established = change_risk(&root, None).unwrap();
    assert_eq!(established.author_prior_commits, 10);

    // A brand-new author's very first commit, identical diff shape.
    std::fs::write(root.join("shared2.txt"), "one\n").unwrap();
    git(&root, &["add", "shared2.txt"]);
    git(
        &root,
        &[
            "commit",
            "-q",
            "-m",
            "Newcomer's first commit",
            "--author",
            "Newcomer <newcomer@example.com>",
        ],
    );
    let newcomer = change_risk(&root, None).unwrap();
    assert_eq!(newcomer.author_prior_commits, 0);

    assert!(newcomer.score > established.score);
}

#[test]
fn score_is_clamped_to_the_zero_to_ten_range() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);

    std::fs::write(root.join("a.txt"), "\n").unwrap();
    git(&root, &["add", "a.txt"]);
    git(&root, &["commit", "-q", "-m", "Add a"]);

    let risk = change_risk(&root, None).unwrap();
    assert!((0.0..=10.0).contains(&risk.score));
}
