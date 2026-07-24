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

/// Same as `git`, but backdates the commit's author/committer date so
/// recency-decay tests don't have to wait for real time to pass.
fn git_commit_at(dir: &Path, message: &str, iso_date: &str) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-q", "-m", message])
        .env_remove("GIT_AUTHOR_NAME")
        .env_remove("GIT_AUTHOR_EMAIL")
        .env_remove("GIT_COMMITTER_NAME")
        .env_remove("GIT_COMMITTER_EMAIL")
        .env("GIT_AUTHOR_DATE", iso_date)
        .env("GIT_COMMITTER_DATE", iso_date)
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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
fn top_co_changed_pairs_ranks_the_whole_repo_by_coupling_count() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);

    // a+b co-change twice; a+c co-change once -- and never all three
    // together, so exactly two distinct pairs exist.
    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    git(&root, &["add", "a.txt"]);
    git(&root, &["commit", "-q", "-m", "Add a alone"]);

    std::fs::write(root.join("a.txt"), "a\na2\n").unwrap();
    std::fs::write(root.join("b.txt"), "b\n").unwrap();
    git(&root, &["add", "a.txt", "b.txt"]);
    git(&root, &["commit", "-q", "-m", "Add b, touch a"]);

    std::fs::write(root.join("a.txt"), "a\na2\na3\n").unwrap();
    std::fs::write(root.join("c.txt"), "c\n").unwrap();
    git(&root, &["add", "a.txt", "c.txt"]);
    git(&root, &["commit", "-q", "-m", "Add c, touch a"]);

    std::fs::write(root.join("a.txt"), "a\na2\na3\na4\n").unwrap();
    std::fs::write(root.join("b.txt"), "b\nb2\n").unwrap();
    git(&root, &["add", "a.txt", "b.txt"]);
    git(&root, &["commit", "-q", "-m", "Touch a and b again"]);

    let analytics = GitAnalytics::collect(&root).unwrap();
    let a_path = root.join("a.txt");
    let b_path = root.join("b.txt");
    let c_path = root.join("c.txt");

    let top = analytics.top_co_changed_pairs(10);

    assert_eq!(top.len(), 2);
    let (a, b, count) = &top[0];
    assert_eq!((a, b), (&a_path, &b_path));
    assert_eq!(*count, 2);
    let (a, c, count) = &top[1];
    assert_eq!((a, c), (&a_path, &c_path));
    assert_eq!(*count, 1);
}

#[test]
fn top_co_changed_pairs_respects_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);

    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    std::fs::write(root.join("b.txt"), "b\n").unwrap();
    std::fs::write(root.join("c.txt"), "c\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "Add a, b, c together"]);

    let analytics = GitAnalytics::collect(&root).unwrap();
    let top = analytics.top_co_changed_pairs(1);

    assert_eq!(top.len(), 1);
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
        max_nesting_depth: 0,
        bumpy_road_bumps: 0,
        complex_conditionals: Vec::new(),
        io_in_loop: Vec::new(),
        string_concat_in_loop: Vec::new(),
        resource_construction_in_loop: Vec::new(),
        lock_in_loop: Vec::new(),
        list_insert_zero_in_loop: Vec::new(),
        param_count: 0,
        primitive_param_count: 0,
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
            field_accesses: Vec::new(),
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

/// Issue #70's "Structural tier" (Objective-C, R, Zig, Julia, Elm,
/// OCaml, Crystal, Nim, D): these languages get a zero-symbol
/// `FileRecord` (see `repowise_parser::parse_file`'s own doc comment)
/// rather than being folded into `RepoIndex.other_files`, so a file in
/// one of them shows up in `hotspots()` output (churn is real, from
/// `git log`) with a `score` of `0` (no symbols to sum complexity
/// over) -- visible, just never ranked above a file with any measured
/// complexity.
#[test]
fn hotspots_includes_a_structural_tier_file_with_a_zero_score() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);

    std::fs::write(root.join("main.zig"), "const x = 1;\n").unwrap();
    git(&root, &["add", "main.zig"]);
    git(&root, &["commit", "-q", "-m", "Add main.zig"]);
    std::fs::write(root.join("main.zig"), "const x = 2;\n").unwrap();
    git(&root, &["add", "main.zig"]);
    git(&root, &["commit", "-q", "-m", "Tweak main.zig"]);

    let zig_path = root.join("main.zig");
    let index = RepoIndex {
        root: root.clone(),
        files: vec![FileRecord {
            path: zig_path.clone(),
            language: Language::Zig,
            lines: 1,
            symbols: Vec::new(),
            imports: Vec::new(),
            calls: Vec::new(),
            field_accesses: Vec::new(),
        }],
        other_files: 0,
    };

    let analytics = GitAnalytics::collect(&root).unwrap();
    let hotspots = repowise_git::hotspots(&index, &analytics);

    assert_eq!(hotspots.len(), 1);
    assert_eq!(hotspots[0].file, zig_path);
    assert_eq!(hotspots[0].churn, 2);
    assert_eq!(hotspots[0].total_complexity, 0);
    assert_eq!(hotspots[0].score, 0);
}

#[test]
fn decayed_score_ranks_recent_churn_above_equally_old_churn() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root);

    // `old.rs`: same churn/complexity as `new.rs`, but both its commits
    // are backdated ~3 years, many half-lives past the 90-day decay —
    // its recency-weighted churn should be effectively zero.
    std::fs::write(root.join("old.rs"), "fn old() {}\n").unwrap();
    git(&root, &["add", "old.rs"]);
    git_commit_at(&root, "Add old.rs", "2022-01-01T00:00:00");
    std::fs::write(root.join("old.rs"), "fn old() { 1 }\n").unwrap();
    git(&root, &["add", "old.rs"]);
    git_commit_at(&root, "Tweak old.rs", "2022-01-02T00:00:00");

    // `new.rs`: committed "now" (no backdating).
    std::fs::write(root.join("new.rs"), "fn new_fn() {}\n").unwrap();
    git(&root, &["add", "new.rs"]);
    git(&root, &["commit", "-q", "-m", "Add new.rs"]);
    std::fs::write(root.join("new.rs"), "fn new_fn() { 1 }\n").unwrap();
    git(&root, &["add", "new.rs"]);
    git(&root, &["commit", "-q", "-m", "Tweak new.rs"]);

    let old_path = root.join("old.rs");
    let new_path = root.join("new.rs");
    let make_symbol = |file: &Path, name: &str| Symbol {
        id: Symbol::make_id(file, name, 1),
        name: name.to_string(),
        kind: SymbolKind::Function,
        file: file.to_path_buf(),
        start_line: 1,
        end_line: 1,
        parent: None,
        complexity: 5,
        max_nesting_depth: 0,
        bumpy_road_bumps: 0,
        complex_conditionals: Vec::new(),
        io_in_loop: Vec::new(),
        string_concat_in_loop: Vec::new(),
        resource_construction_in_loop: Vec::new(),
        lock_in_loop: Vec::new(),
        list_insert_zero_in_loop: Vec::new(),
        param_count: 0,
        primitive_param_count: 0,
        body_hash: None,
    };
    let index = RepoIndex {
        root: root.clone(),
        files: vec![
            FileRecord {
                path: old_path.clone(),
                language: Language::Rust,
                lines: 1,
                symbols: vec![make_symbol(&old_path, "old")],
                imports: Vec::new(),
                calls: Vec::new(),
                field_accesses: Vec::new(),
            },
            FileRecord {
                path: new_path.clone(),
                language: Language::Rust,
                lines: 1,
                symbols: vec![make_symbol(&new_path, "new_fn")],
                imports: Vec::new(),
                calls: Vec::new(),
                field_accesses: Vec::new(),
            },
        ],
        other_files: 0,
    };

    let analytics = GitAnalytics::collect(&root).unwrap();
    let hotspots = repowise_git::hotspots(&index, &analytics);

    let old_hotspot = hotspots.iter().find(|h| h.file == old_path).unwrap();
    let new_hotspot = hotspots.iter().find(|h| h.file == new_path).unwrap();

    // Equal raw churn/complexity, so equal raw score...
    assert_eq!(old_hotspot.churn, new_hotspot.churn);
    assert_eq!(old_hotspot.score, new_hotspot.score);
    // ...but the recently-touched file ranks higher on decayed score.
    assert!(new_hotspot.decayed_score > old_hotspot.decayed_score);
    // Recent commits decay by a negligible amount over a test run.
    assert!(new_hotspot.decayed_score > old_hotspot.score as f64 - 0.01);
    // Old commits (~3 years, dozens of half-lives past 90 days) decay to
    // effectively nothing.
    assert!(old_hotspot.decayed_score < 0.01);

    // Ranking (not just the raw values) reflects recency: `new.rs` sorts
    // ahead of `old.rs` despite identical churn/complexity.
    let new_rank = hotspots.iter().position(|h| h.file == new_path).unwrap();
    let old_rank = hotspots.iter().position(|h| h.file == old_path).unwrap();
    assert!(new_rank < old_rank);
}
