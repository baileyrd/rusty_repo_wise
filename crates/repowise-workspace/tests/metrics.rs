//! Exercises `workspace_metrics` against real, disposable repo
//! directories -- cross-repo edge resolution reads each repo's real
//! `Cargo.toml` and real source, so these can't be faked with
//! hand-built indexes.

use repowise_core::{discover_files, Language, RepoIndex};
use repowise_workspace::{workspace_metrics, Confidence, ResolvedWorkspaceRepo};
use std::fs;
use std::path::Path;

fn index_dir(root: &Path) {
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
    RepoIndex {
        root: root.to_path_buf(),
        files,
        other_files,
        indexed_commit: None,
    }
    .save(root)
    .unwrap();
}

/// A Rust crate named `name` whose `lib.rs` is `source`.
fn rust_repo(root: &Path, name: &str, source: &str) -> ResolvedWorkspaceRepo {
    let path = root.join(name);
    fs::create_dir_all(path.join("src")).unwrap();
    fs::write(
        path.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .unwrap();
    fs::write(path.join("src").join("lib.rs"), source).unwrap();
    index_dir(&path);
    ResolvedWorkspaceRepo {
        name: name.to_string(),
        path,
    }
}

#[test]
fn independent_repos_have_minimal_propagation_cost() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let a = rust_repo(&root, "alpha", "pub fn a() {}\n");
    let b = rust_repo(&root, "beta", "pub fn b() {}\n");

    let m = workspace_metrics(&[a, b]);

    assert_eq!(m.edge_count, 0);
    // Each repo reaches only itself: 2 of 4 ordered pairs.
    assert!(
        (m.propagation_cost - 0.5).abs() < 1e-9,
        "{}",
        m.propagation_cost
    );
    assert!(m.cyclic_core.is_empty());
    // Rust files exist, so resolution genuinely ran and found nothing --
    // that's scorable, unlike the no-Rust case below.
    assert_eq!(m.confidence, Confidence::NoEdgesFound);
    let score = m.complexity_score.expect("scorable when Rust was present");
    assert!(
        (score - 1.0).abs() < 1e-9,
        "fully decoupled scores 1: {score}"
    );
}

#[test]
fn a_dependency_raises_propagation_cost_above_the_floor() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let _lib = rust_repo(&root, "corelib", "pub fn helper() {}\n");
    let app = rust_repo(
        &root,
        "app",
        "use corelib::helper;\npub fn run() { helper(); }\n",
    );
    let lib = ResolvedWorkspaceRepo {
        name: "corelib".to_string(),
        path: root.join("corelib"),
    };

    let m = workspace_metrics(&[app, lib]);

    assert_eq!(m.edge_count, 1, "app -> corelib");
    assert_eq!(m.confidence, Confidence::Resolved);
    // 3 of 4 ordered pairs reachable (both selves, plus app -> corelib).
    assert!(
        (m.propagation_cost - 0.75).abs() < 1e-9,
        "{}",
        m.propagation_cost
    );
    let score = m.complexity_score.unwrap();
    assert!(
        score > 1.0,
        "a real dependency must score above the floor: {score}"
    );
}

#[test]
fn a_two_repo_cycle_lands_in_the_cyclic_core() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let a = rust_repo(&root, "left", "use right::b;\npub fn a() { b(); }\n");
    let b = rust_repo(&root, "right", "use left::a;\npub fn b() { a(); }\n");

    let m = workspace_metrics(&[a, b]);

    assert_eq!(m.cyclic_core.len(), 1, "{:?}", m.cyclic_core);
    assert_eq!(m.repos_in_cyclic_core, 2);
    // Mutually reachable: all 4 ordered pairs.
    assert!(
        (m.propagation_cost - 1.0).abs() < 1e-9,
        "{}",
        m.propagation_cost
    );
    let score = m.complexity_score.unwrap();
    assert!(
        score > 9.0,
        "total coupling plus a full cycle should be near 10: {score}"
    );
}

/// The failure this feature had to avoid: a workspace this port cannot
/// resolve must NOT be reported as perfectly decoupled. Uses TypeScript,
/// not Python -- Python joined the module-map languages this pass
/// resolves, so it no longer demonstrates an unresolvable workspace;
/// TypeScript still resolves imports directly against the filesystem
/// instead, so it's the language that now proves this guarantee.
#[test]
fn a_workspace_with_no_resolvable_language_withholds_the_score() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    for name in ["svc-a", "svc-b"] {
        let path = root.join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("app.ts"), "export function go(): void {}\n").unwrap();
        index_dir(&path);
    }
    let repos = vec![
        ResolvedWorkspaceRepo {
            name: "svc-a".to_string(),
            path: root.join("svc-a"),
        },
        ResolvedWorkspaceRepo {
            name: "svc-b".to_string(),
            path: root.join("svc-b"),
        },
    ];

    let m = workspace_metrics(&repos);

    assert_eq!(m.confidence, Confidence::NoResolvableLanguage);
    assert_eq!(
        m.complexity_score, None,
        "an unmeasurable workspace must not earn the best possible score"
    );
}

#[test]
fn unindexed_repos_are_named() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let a = rust_repo(&root, "indexed", "pub fn a() {}\n");
    let never = root.join("never");
    fs::create_dir_all(&never).unwrap();

    let m = workspace_metrics(&[
        a,
        ResolvedWorkspaceRepo {
            name: "never".to_string(),
            path: never,
        },
    ]);

    assert_eq!(m.unindexed_repos, vec!["never"]);
}
