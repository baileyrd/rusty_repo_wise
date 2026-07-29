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
        indexed_commit: None,
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

/// Create a repo directory with the given files, indexed.
fn repo_with(root: &Path, name: &str, files: &[(&str, &str)]) -> ResolvedWorkspaceRepo {
    let path = root.join(name);
    fs::create_dir_all(&path).unwrap();
    for (file, source) in files {
        fs::write(path.join(file), source).unwrap();
    }
    index_dir(&path);
    ResolvedWorkspaceRepo {
        name: name.to_string(),
        path,
    }
}

/// The distinction this command exists for: a consumer call served
/// only inside its own repo is NOT a missing cross-repo contract, and
/// must not be counted as one.
#[test]
fn diagnostics_separates_same_repo_calls_from_genuinely_unserved_ones() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let app = repo_with(
        &root,
        "app",
        &[
            (
                "routes.rs",
                r#"fn r() { Router::new().route("/api/local", get(h)); }"#,
            ),
            (
                "client.js",
                r#"async function a() {
    await fetch("/api/local");
    await fetch("/api/nowhere");
}"#,
            ),
        ],
    );
    let repos = vec![app];

    let diag = repowise_workspace::workspace_diagnostics(&repos);

    let reasons: Vec<_> = diag
        .unmatched_consumers
        .iter()
        .map(|u| (u.call.path.as_str(), u.reason))
        .collect();
    assert!(
        reasons.contains(&(
            "/api/local",
            repowise_workspace::UnmatchedReason::SameRepoOnly
        )),
        "a route served by the calling repo itself is not a cross-repo gap: {reasons:?}"
    );
    assert!(
        reasons.contains(&(
            "/api/nowhere",
            repowise_workspace::UnmatchedReason::NoProducerAnywhere
        )),
        "{reasons:?}"
    );
}

#[test]
fn diagnostics_reports_a_cross_repo_path_hit_with_a_different_method_as_a_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let server = repo_with(
        &root,
        "server",
        &[(
            "routes.rs",
            r#"fn r() { Router::new().route("/api/thing", post(h)); }"#,
        )],
    );
    let client = repo_with(
        &root,
        "client",
        &[(
            "app.js",
            r#"async function a() { await axios.get("/api/thing"); }"#,
        )],
    );

    let diag = repowise_workspace::workspace_diagnostics(&vec![server, client]);

    assert_eq!(
        diag.unmatched_consumers.len(),
        1,
        "{:?}",
        diag.unmatched_consumers
    );
    assert_eq!(
        diag.unmatched_consumers[0].reason,
        repowise_workspace::UnmatchedReason::MethodMismatch,
        "a path served cross-repo under a different verb is sharper than 'nothing serves this'"
    );
}

/// The failure mode that motivated this command: an unindexed repo
/// contributes nothing at all, and a thin report then looks like an
/// architecture finding.
#[test]
fn diagnostics_names_repos_it_could_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let indexed = repo_with(
        &root,
        "indexed",
        &[(
            "routes.rs",
            r#"fn r() { Router::new().route("/api/x", get(h)); }"#,
        )],
    );
    // Exists on disk but was never indexed.
    let never = root.join("never");
    fs::create_dir_all(&never).unwrap();
    fs::write(
        never.join("app.js"),
        r#"async function a() { await fetch("/api/x"); }"#,
    )
    .unwrap();

    let diag = repowise_workspace::workspace_diagnostics(&vec![
        indexed,
        ResolvedWorkspaceRepo {
            name: "never".to_string(),
            path: never,
        },
    ]);

    assert_eq!(diag.unindexed_repos(), vec!["never"]);
    assert_eq!(
        diag.matches, 0,
        "the call in the unindexed repo was never scanned"
    );
    // And the report must not present that zero as a finding -- the
    // per-repo row says "not indexed" rather than "0 consumers".
    let never_row = diag.repos.iter().find(|r| r.repo == "never").unwrap();
    assert!(!never_row.indexed);
    assert_eq!(never_row.consumers, 0);
}

#[test]
fn diagnostics_reports_producers_nothing_calls() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let server = repo_with(
        &root,
        "server",
        &[(
            "routes.rs",
            r#"fn r() {
    Router::new().route("/api/used", get(a));
    Router::new().route("/api/unused", get(b));
}"#,
        )],
    );
    let client = repo_with(
        &root,
        "client",
        &[(
            "app.js",
            r#"async function a() { await fetch("/api/used"); }"#,
        )],
    );

    let diag = repowise_workspace::workspace_diagnostics(&vec![server, client]);

    let orphans: Vec<_> = diag
        .orphan_producers
        .iter()
        .map(|o| o.route.path.as_str())
        .collect();
    assert!(orphans.contains(&"/api/unused"), "{orphans:?}");
    assert!(
        !orphans.contains(&"/api/used"),
        "a matched producer isn't an orphan: {orphans:?}"
    );
}

/// Diagnostics and contracts share one scan, so their match counts must
/// agree. If they ever diverge, one of the two is lying about the same
/// workspace.
#[test]
fn diagnostics_match_count_agrees_with_workspace_contracts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let server = repo_with(
        &root,
        "server",
        &[(
            "routes.rs",
            r#"fn r() { Router::new().route("/api/shared", get(h)); }"#,
        )],
    );
    let client = repo_with(
        &root,
        "client",
        &[(
            "app.js",
            r#"async function a() { await fetch("/api/shared"); }"#,
        )],
    );
    let repos = vec![server, client];

    let contracts = workspace_contracts(&repos);
    let diag = repowise_workspace::workspace_diagnostics(&repos);

    assert_eq!(contracts.matches.len(), diag.matches);
    assert_eq!(
        contracts.unmatched_consumers.len(),
        diag.unmatched_consumers.len()
    );
}
