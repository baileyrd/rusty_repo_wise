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
        param_count: 0,
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
