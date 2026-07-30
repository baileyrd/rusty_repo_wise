//! Exercises `workspace_contract_changes` end to end: two real calls
//! against the same on-disk workspace, with the producer route removed
//! in between, over a real `.repowise-workspace/contracts.json`
//! snapshot on disk -- not a hand-built `ContractKey` set.

use repowise_core::{discover_files, Language, RepoIndex};
use repowise_workspace::{workspace_contract_changes, ResolvedWorkspaceRepo};
use std::fs;
use std::path::Path;

fn index_dir(root: &Path) -> RepoIndex {
    let discovered = discover_files(root).unwrap();
    let mut files = Vec::new();
    let mut other_files = 0;
    for entry in discovered {
        if matches!(entry.language, Language::Other) {
            other_files += 1;
            continue;
        }
        let source = fs::read_to_string(&entry.path).unwrap();
        match repowise_parser::parse_file(&entry.path, entry.language, &source).unwrap() {
            Some(record) => files.push(record),
            None => other_files += 1,
        }
    }
    let index = RepoIndex {
        root: root.to_path_buf(),
        files,
        other_files,
        indexed_commit: None,
    };
    index.save(root).unwrap();
    index
}

#[test]
fn a_removed_producer_route_is_reported_broken_on_the_next_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let state_dir = root.join(".repowise-workspace");

    let server_repo = root.join("server");
    fs::create_dir_all(&server_repo).unwrap();
    fs::write(
        server_repo.join("routes.rs"),
        r#"fn build_router() {
    let router = Router::new().route("/api/hotspots", get(get_hotspots));
}
"#,
    )
    .unwrap();
    index_dir(&server_repo);

    let client_repo = root.join("client");
    fs::create_dir_all(&client_repo).unwrap();
    fs::write(
        client_repo.join("app.js"),
        r#"async function load() {
    const res = await fetch("/api/hotspots");
    return res.json();
}
"#,
    )
    .unwrap();
    index_dir(&client_repo);

    let repos = vec![
        ResolvedWorkspaceRepo {
            name: "server".to_string(),
            path: server_repo.clone(),
        },
        ResolvedWorkspaceRepo {
            name: "client".to_string(),
            path: client_repo,
        },
    ];

    // First run: matches, and nothing broken -- no prior snapshot to
    // compare against.
    let (first_report, first_broken) = workspace_contract_changes(&repos, &state_dir);
    assert_eq!(first_report.matches.len(), 1);
    assert!(first_broken.is_empty());
    assert!(
        state_dir.join("contracts.json").is_file(),
        "the snapshot should be written after the first run"
    );

    // The producer removes its route entirely -- the client's call now
    // resolves to nothing anywhere in the workspace.
    fs::write(
        server_repo.join("routes.rs"),
        "fn build_router() {\n    let router = Router::new();\n}\n",
    )
    .unwrap();
    index_dir(&server_repo);

    let (second_report, second_broken) = workspace_contract_changes(&repos, &state_dir);
    assert!(second_report.matches.is_empty());
    assert_eq!(second_broken.len(), 1);
    assert_eq!(second_broken[0].key.path, "/api/hotspots");
    assert_eq!(second_broken[0].key.producer_repo, "server");
    assert_eq!(second_broken[0].key.consumer_repo, "client");
    assert_eq!(
        second_broken[0].reason,
        Some(repowise_workspace::UnmatchedReason::NoProducerAnywhere)
    );

    // A third run with nothing changed should report the (now-empty)
    // baseline honestly: nothing broke, because nothing resolved before
    // either.
    let (third_report, third_broken) = workspace_contract_changes(&repos, &state_dir);
    assert!(third_report.matches.is_empty());
    assert!(third_broken.is_empty());
}

#[test]
fn a_first_run_with_no_prior_snapshot_reports_nothing_broken() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let state_dir = root.join(".repowise-workspace");

    let repo_path = root.join("solo");
    fs::create_dir_all(&repo_path).unwrap();
    fs::write(repo_path.join("app.js"), "async function f() {}\n").unwrap();
    index_dir(&repo_path);

    let repos = vec![ResolvedWorkspaceRepo {
        name: "solo".to_string(),
        path: repo_path,
    }];

    let (report, broken) = workspace_contract_changes(&repos, &state_dir);
    assert!(report.matches.is_empty());
    assert!(broken.is_empty());
}
