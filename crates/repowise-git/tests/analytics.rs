//! Exercises `GitAnalytics`/`ownership_of`/`hotspots` against real,
//! disposable git repos built with the `git` CLI directly (no test-only
//! crate needed beyond `tempfile`), so the actual `git log`/`git blame`
//! output shapes are what's under test, not a mock of them.

use repowise_core::{FileRecord, Language, RepoIndex, Symbol, SymbolKind};
use repowise_git::GitAnalytics;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    // Clear GIT_AUTHOR_*/GIT_COMMITTER_* so the host environment's identity
    // (this sandbox sets one for its own commits) can't leak into these
    // disposable test repos and override their local `user.name`/`user.email`.
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
fn tracks_churn_bugfix_commits_and_last_touch() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);

    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    git(&root, &["add", "a.txt"]);
    git(&root, &["commit", "-q", "-m", "Add a"]);

    std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
    git(&root, &["add", "a.txt"]);
    git(&root, &["commit", "-q", "-m", "Fix bug in a"]);

    let analytics = GitAnalytics::collect(&root).unwrap();
    let a_path = root.join("a.txt");

    assert_eq!(analytics.churn_of(&a_path), 2);
    assert_eq!(analytics.bugfix_commits_of(&a_path), 1);
    assert_eq!(analytics.commit_count, 2);

    let (_, author) = analytics
        .last_touch_of(&a_path)
        .expect("should have a last touch");
    assert_eq!(author, "Default Author");
}

#[test]
fn builds_co_change_coupling_from_shared_commits() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);

    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    std::fs::write(root.join("b.txt"), "b\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "Add a and b together"]);

    // A solo commit shouldn't add coupling.
    std::fs::write(root.join("a.txt"), "a\na2\n").unwrap();
    git(&root, &["add", "a.txt"]);
    git(&root, &["commit", "-q", "-m", "Touch only a"]);

    let analytics = GitAnalytics::collect(&root).unwrap();
    let a_path = root.join("a.txt");
    let b_path = root.join("b.txt");

    let coupled_with_a = analytics.coupled_files(&a_path, 10);
    assert_eq!(coupled_with_a, vec![(b_path.clone(), 1)]);

    let coupled_with_b = analytics.coupled_files(&b_path, 10);
    assert_eq!(coupled_with_b, vec![(a_path, 1)]);
}

#[test]
fn ownership_splits_lines_by_author() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);

    std::fs::write(root.join("shared.txt"), "line1\nline2\n").unwrap();
    git(&root, &["add", "shared.txt"]);
    git(&root, &["commit", "-q", "-m", "Author A writes two lines"]);

    std::fs::write(root.join("shared.txt"), "line1\nline2\nline3\n").unwrap();
    git(&root, &["add", "shared.txt"]);
    git(
        &root,
        &[
            "commit",
            "-q",
            "-m",
            "Author B adds a third line",
            "--author",
            "Author B <b@example.com>",
        ],
    );

    let file = root.join("shared.txt");
    let ownership = repowise_git::ownership_of(&root, &file).unwrap();

    assert_eq!(ownership.len(), 2);
    let a = ownership
        .iter()
        .find(|o| o.author == "Default Author")
        .unwrap();
    let b = ownership.iter().find(|o| o.author == "Author B").unwrap();
    assert_eq!(a.lines, 2);
    assert_eq!(b.lines, 1);
    assert!((a.percentage - 200.0 / 3.0).abs() < 0.01);
}

#[test]
fn hotspot_score_multiplies_churn_by_complexity() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);

    std::fs::write(root.join("hot.rs"), "fn hot() {}\n").unwrap();
    git(&root, &["add", "hot.rs"]);
    git(&root, &["commit", "-q", "-m", "Add hot.rs"]);
    std::fs::write(root.join("hot.rs"), "fn hot() { 1 }\n").unwrap();
    git(&root, &["add", "hot.rs"]);
    git(&root, &["commit", "-q", "-m", "Tweak hot.rs"]);

    let hot_path = root.join("hot.rs");
    let symbol = Symbol {
        id: Symbol::make_id(&hot_path, "hot", 1),
        name: "hot".to_string(),
        kind: SymbolKind::Function,
        file: hot_path.clone(),
        start_line: 1,
        end_line: 1,
        parent: None,
        complexity: 5,
        param_count: 0,
        body_hash: None,
    };
    let index = RepoIndex {
        root: root.clone(),
        files: vec![FileRecord {
            path: hot_path.clone(),
            language: Language::Rust,
            lines: 1,
            symbols: vec![symbol],
            imports: Vec::new(),
            calls: Vec::new(),
        }],
        other_files: 0,
    };

    let analytics = GitAnalytics::collect(&root).unwrap();
    let hotspots = repowise_git::hotspots(&index, &analytics);

    assert_eq!(hotspots.len(), 1);
    assert_eq!(hotspots[0].file, hot_path);
    assert_eq!(hotspots[0].churn, 2);
    assert_eq!(hotspots[0].total_complexity, 5);
    assert_eq!(hotspots[0].score, 10);
}
