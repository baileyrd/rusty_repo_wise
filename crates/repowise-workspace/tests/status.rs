use repowise_core::{FileRecord, Language, RepoIndex};
use repowise_workspace::{repo_status, ResolvedWorkspaceRepo};

fn index_with_one_file(root: &std::path::Path) -> RepoIndex {
    let index = RepoIndex {
        root: root.to_path_buf(),
        files: vec![FileRecord {
            path: root.join("a.rs"),
            language: Language::Rust,
            lines: 1,
            symbols: vec![],
            imports: vec![],
            calls: vec![],
            field_accesses: vec![],
        }],
        other_files: 3,
        indexed_commit: None,
    };
    index.save(root).unwrap();
    index
}

#[test]
fn repo_status_reports_indexed_and_file_counts_when_a_prior_index_exists() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    index_with_one_file(&root);
    let repo = ResolvedWorkspaceRepo {
        name: "test".to_string(),
        path: root,
        index: None,
    };

    let status = repo_status(&repo);

    assert_eq!(status.name, "test");
    assert!(status.indexed);
    assert_eq!(status.file_count, Some(1));
    assert_eq!(status.other_file_count, Some(3));
}

#[test]
fn repo_status_is_not_indexed_without_a_prior_repowise_init() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let repo = ResolvedWorkspaceRepo {
        name: "test".to_string(),
        path: root,
        index: None,
    };

    let status = repo_status(&repo);

    assert!(!status.indexed);
    assert_eq!(status.file_count, None);
    assert_eq!(status.other_file_count, None);
}

#[test]
fn repo_status_is_not_indexed_for_a_nonexistent_path_and_does_not_panic() {
    let repo = ResolvedWorkspaceRepo {
        name: "gone".to_string(),
        path: std::path::PathBuf::from("/nonexistent/path/for/this/test"),
        index: None,
    };

    let status = repo_status(&repo);

    assert!(!status.indexed);
}
