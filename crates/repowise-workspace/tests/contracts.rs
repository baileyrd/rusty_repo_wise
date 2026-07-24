//! Exercises `workspace_contracts` against real, disposable repo
//! directories with real source files on disk (the regex scan reads
//! raw file content directly, independent of parsed symbols).

use repowise_core::{discover_files, Language, RepoIndex};
use repowise_workspace::{workspace_contracts, ResolvedWorkspaceRepo};
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
    };
    index.save(root).unwrap();
    index
}

#[test]
fn workspace_contracts_matches_a_producer_and_a_cross_repo_consumer() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

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
            path: server_repo,
        },
        ResolvedWorkspaceRepo {
            name: "client".to_string(),
            path: client_repo,
        },
    ];

    let report = workspace_contracts(&repos);

    assert_eq!(report.matches.len(), 1);
    assert_eq!(report.matches[0].producer_repo, "server");
    assert_eq!(report.matches[0].consumer_repo, "client");
    assert_eq!(report.matches[0].path, "/api/hotspots");
    assert!(report.unmatched_consumers.is_empty());
}

#[test]
fn workspace_contracts_reports_an_unmatched_consumer() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let client_repo = root.join("client");
    fs::create_dir_all(&client_repo).unwrap();
    fs::write(
        client_repo.join("app.js"),
        r#"fetch("/api/unknown-endpoint");
"#,
    )
    .unwrap();
    index_dir(&client_repo);

    let repos = vec![ResolvedWorkspaceRepo {
        name: "client".to_string(),
        path: client_repo,
    }];

    let report = workspace_contracts(&repos);

    assert!(report.matches.is_empty());
    assert_eq!(report.unmatched_consumers.len(), 1);
    assert_eq!(report.unmatched_consumers[0].path, "/api/unknown-endpoint");
}

#[test]
fn workspace_contracts_matches_route_templates_against_a_literal_path() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let server_repo = root.join("server");
    fs::create_dir_all(&server_repo).unwrap();
    fs::write(
        server_repo.join("app.py"),
        "@app.get(\"/api/users/:id\")\ndef get_user():\n    pass\n",
    )
    .unwrap();
    index_dir(&server_repo);

    let client_repo = root.join("client");
    fs::create_dir_all(&client_repo).unwrap();
    fs::write(
        client_repo.join("client.py"),
        "requests.get(\"/api/users/42\")\n",
    )
    .unwrap();
    index_dir(&client_repo);

    let repos = vec![
        ResolvedWorkspaceRepo {
            name: "server".to_string(),
            path: server_repo,
        },
        ResolvedWorkspaceRepo {
            name: "client".to_string(),
            path: client_repo,
        },
    ];

    let report = workspace_contracts(&repos);

    assert_eq!(report.matches.len(), 1);
    assert_eq!(report.matches[0].path, "/api/users/42");
}
