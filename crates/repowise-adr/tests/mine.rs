//! End-to-end test of `mine()`: a real temp git repo with an ADR file
//! (superseded by a second ADR) and a decision-like commit, all linking
//! to the same source file via a shared symbol name.

use repowise_adr::{mine, DecisionSource};
use repowise_core::{FileRecord, Language, RepoIndex, Symbol, SymbolKind};
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

#[test]
fn mines_and_links_adrs_and_decision_commits() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("docs/adr")).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();

    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.name", "Fixture Author"]);
    git(&root, &["config", "user.email", "fixture@example.com"]);

    std::fs::write(root.join("src/queue.rs"), "pub struct TaskQueue;\n").unwrap();
    std::fs::write(
        root.join("docs/adr/0001-use-in-memory-queue.md"),
        "# ADR-0001: Use an in-memory queue\n\nStatus: Superseded by ADR-0002\nDate: 2026-01-01\n\n## Decision\nUse TaskQueue backed by a Vec.\n",
    )
    .unwrap();
    std::fs::write(
        root.join("docs/adr/0002-switch-to-sled.md"),
        "# ADR-0002: Persist TaskQueue with sled\n\nStatus: Accepted\nDate: 2026-02-01\n\n## Decision\nPersist TaskQueue state using sled.\n",
    )
    .unwrap();
    // The seed template should be ignored, not mined as a decision.
    std::fs::write(
        root.join("docs/adr/0000-template.md"),
        "# ADR-0000: <Title>\n\nStatus: Proposed\nDate: YYYY-MM-DD\n",
    )
    .unwrap();

    git(&root, &["add", "-A"]);
    git(
        &root,
        &["commit", "-q", "-m", "Add initial in-memory TaskQueue"],
    );
    git(
        &root,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "Decide to switch to sled for TaskQueue persistence",
        ],
    );

    let queue_path = root.join("src/queue.rs");
    let symbol = Symbol {
        id: Symbol::make_id(&queue_path, "TaskQueue", 1),
        name: "TaskQueue".to_string(),
        kind: SymbolKind::Struct,
        file: queue_path.clone(),
        start_line: 1,
        end_line: 1,
        parent: None,
        complexity: 0,
        max_nesting_depth: 0,
        bumpy_road_bumps: 0,
        complex_conditionals: Vec::new(),
        io_in_loop: Vec::new(),
        string_concat_in_loop: Vec::new(),
        resource_construction_in_loop: Vec::new(),
        lock_in_loop: Vec::new(),
        list_insert_zero_in_loop: Vec::new(),
        json_parse_in_loop: Vec::new(),
        regex_compile_in_loop: Vec::new(),
        nested_loop_with_io: Vec::new(),
        nested_loop_quadratic: Vec::new(),
        serial_await_in_loop: Vec::new(),
        pd_concat_in_loop: Vec::new(),
        blocking_sync_in_async: Vec::new(),
        blocking_io_under_lock: Vec::new(),
        array_spread_in_reduce: Vec::new(),
        sql_cartesian_join: Vec::new(),
        defer_in_loop: Vec::new(),
        goroutine_in_unbounded_loop: Vec::new(),
        membership_test_in_loop: Vec::new(),
        sync_io_calls: Vec::new(),
        param_count: 0,
        primitive_param_count: 0,
        body_hash: None,
    };
    let index = RepoIndex {
        root: root.clone(),
        files: vec![FileRecord {
            path: queue_path.clone(),
            language: Language::Other,
            lines: 1,
            symbols: vec![symbol],
            imports: Vec::new(),
            calls: Vec::new(),
            field_accesses: Vec::new(),
        }],
        other_files: 0,
        indexed_commit: None,
    };

    let decisions = mine(&index).unwrap();

    // The unfilled template must not appear.
    assert!(decisions.iter().all(|d| d.id != "ADR-0000"));

    let adr1 = decisions.iter().find(|d| d.id == "ADR-0001").unwrap();
    assert_eq!(adr1.superseded_by.as_deref(), Some("ADR-0002"));
    assert!(adr1.is_superseded());
    assert_eq!(adr1.linked_files, vec![queue_path.clone()]);

    let adr2 = decisions.iter().find(|d| d.id == "ADR-0002").unwrap();
    assert!(!adr2.is_superseded());
    assert_eq!(adr2.status.as_deref(), Some("Accepted"));
    assert_eq!(adr2.linked_files, vec![queue_path.clone()]);

    let commit_decision = decisions
        .iter()
        .find(|d| matches!(&d.source, DecisionSource::CommitMessage { .. }))
        .expect("the decision-like commit should be mined");
    assert!(commit_decision.title.contains("switch to sled"));
    assert_eq!(commit_decision.linked_files, vec![queue_path]);
}

/// The inferred source flowing through `mine()` end to end, alongside a
/// real mined decision — the combination is what matters, since the
/// whole point is that a reader can tell the two apart in one list.
#[test]
fn inferred_decisions_reach_mine_labelled_and_never_masquerade_as_mined() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/queue.rs"),
        "// WHY: bounded so backpressure beats memory growth\npub struct TaskQueue;\nlet q = Bounded::new(128);\n",
    )
    .unwrap();

    repowise_adr::InferredStore {
        model: "fixture-model".to_string(),
        decisions: vec![
            repowise_adr::InferredDecision {
                title: "Bounded task queue".to_string(),
                rationale: "Backpressure is preferred to unbounded memory growth.".to_string(),
                file: "src/queue.rs".to_string(),
                anchor: "let q = Bounded::new(128);".to_string(),
            },
            // Anchored to code that isn't there. Must not survive.
            repowise_adr::InferredDecision {
                title: "Retries with jitter".to_string(),
                rationale: "The code retries with exponential backoff.".to_string(),
                file: "src/queue.rs".to_string(),
                anchor: "retry_with_jitter(5)".to_string(),
            },
        ],
    }
    .save(&root)
    .unwrap();

    let index = RepoIndex {
        root: root.clone(),
        files: vec![FileRecord {
            path: root.join("src/queue.rs"),
            language: Language::Rust,
            lines: 3,
            // No symbols needed: an inferred decision links via its
            // anchor and an inline marker via its enclosing file --
            // neither goes through symbol matching.
            symbols: vec![],
            imports: vec![],
            calls: vec![],
            field_accesses: vec![],
        }],
        other_files: 0,
        indexed_commit: None,
    };

    let (records, state) = repowise_adr::mine_reporting(&index).unwrap();

    let inferred: Vec<_> = records.iter().filter(|d| d.source.is_inferred()).collect();
    assert_eq!(
        inferred.len(),
        1,
        "the unanchored proposal must not survive: {:?}",
        inferred.iter().map(|d| &d.title).collect::<Vec<_>>()
    );
    assert_eq!(inferred[0].title, "Bounded task queue");
    assert_eq!(inferred[0].linked_files, vec![root.join("src/queue.rs")]);

    // The marker source in the same file is mined, and must stay
    // unflagged -- an `inferred` flag that also fires on written
    // artifacts would tell a reader nothing.
    let marker: Vec<_> = records
        .iter()
        .filter(|d| matches!(d.source, DecisionSource::InlineMarker { .. }))
        .collect();
    assert_eq!(marker.len(), 1, "{records:?}");
    assert!(!marker[0].source.is_inferred());

    assert!(state.describe().contains("fixture-model"));
    assert!(
        state.describe().contains("no longer in the file"),
        "the dropped proposal must be reported, not silently swallowed: {}",
        state.describe()
    );
}

/// `mine` and `mine_reporting` must not diverge — the only difference
/// is that one discards the state.
#[test]
fn mine_returns_exactly_what_mine_reporting_returns() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("a.rs"), "// DECISION: use one runtime\n").unwrap();

    let index = RepoIndex {
        root: root.clone(),
        files: vec![FileRecord {
            path: root.join("a.rs"),
            language: Language::Rust,
            lines: 1,
            symbols: vec![],
            imports: vec![],
            calls: vec![],
            field_accesses: vec![],
        }],
        other_files: 0,
        indexed_commit: None,
    };

    let plain = mine(&index).unwrap();
    let (reported, _) = repowise_adr::mine_reporting(&index).unwrap();
    assert_eq!(plain.len(), reported.len());
    assert_eq!(
        plain.iter().map(|d| &d.id).collect::<Vec<_>>(),
        reported.iter().map(|d| &d.id).collect::<Vec<_>>()
    );
}
