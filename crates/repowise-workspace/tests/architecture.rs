//! Exercises `workspace_architecture`/`workspace_blast_radius` against
//! real, disposable multi-crate scratch directories parsed with
//! `repowise-parser` -- hand-constructing `ImportRef`s would bypass the
//! actual `use`-statement parsing this is meant to test.

use repowise_core::{discover_files, FileRecord, Language, RepoIndex};
use repowise_workspace::{
    detect_workspace_cycles, workspace_architecture, workspace_blast_radius, ResolvedWorkspaceRepo,
};
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

fn find_file<'a>(index: &'a RepoIndex, suffix: &str) -> &'a FileRecord {
    index
        .files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(suffix))
        .unwrap_or_else(|| panic!("no indexed file ending in {suffix}"))
}

fn write_crate(root: &Path, crate_name: &str, files: &[(&str, &str)]) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        format!("[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
    )
    .unwrap();
    for (rel_path, contents) in files {
        let path = root.join(rel_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }
}

#[test]
fn workspace_architecture_resolves_a_cross_repo_use_import() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let repo_a_path = root.join("repo-a");
    write_crate(
        &repo_a_path,
        "repo-a",
        &[("src/foo.rs", "pub fn bar() -> i32 { 42 }\n")],
    );
    let index_a = index_dir(&repo_a_path);

    let repo_b_path = root.join("repo-b");
    write_crate(
        &repo_b_path,
        "repo-b",
        &[(
            "src/lib.rs",
            "use repo_a::foo::bar;\n\nfn top() -> i32 {\n    bar()\n}\n",
        )],
    );
    index_dir(&repo_b_path);

    let repos = vec![
        ResolvedWorkspaceRepo {
            name: "repo-a".to_string(),
            path: repo_a_path,
            index: None,
        },
        ResolvedWorkspaceRepo {
            name: "repo-b".to_string(),
            path: repo_b_path,
            index: None,
        },
    ];

    let report = workspace_architecture(&repos);

    assert_eq!(report.repos.len(), 2);
    assert!(report.repos.iter().all(|r| r.indexed));

    assert_eq!(report.edges.len(), 1);
    let edge = &report.edges[0];
    assert_eq!(edge.from_repo, "repo-b");
    assert_eq!(edge.to_repo, "repo-a");
    assert_eq!(edge.to_file, find_file(&index_a, "foo.rs").path);
    assert_eq!(edge.import_path, "repo_a::foo::bar");

    assert_eq!(report.repo_edges.len(), 1);
    assert_eq!(report.repo_edges[0].from_repo, "repo-b");
    assert_eq!(report.repo_edges[0].to_repo, "repo-a");
    assert_eq!(report.repo_edges[0].edge_count, 1);
}

#[test]
fn workspace_architecture_resolves_same_repo_imports_first_even_with_a_naming_collision() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    // One repo containing two sibling crates: crate-y imports crate-x,
    // fully resolvable within this same repo.
    let repo_a_path = root.join("repo-a");
    fs::create_dir_all(&repo_a_path).unwrap();
    write_crate(
        &repo_a_path.join("crate-x"),
        "crate-x",
        &[("src/lib.rs", "pub fn shared() -> i32 { 1 }\n")],
    );
    write_crate(
        &repo_a_path.join("crate-y"),
        "crate-y",
        &[(
            "src/lib.rs",
            "use crate_x::shared;\n\nfn top() -> i32 {\n    shared()\n}\n",
        )],
    );
    // Index each crate directory separately (as `repowise init` would
    // per-crate), then hand-assemble one combined RepoIndex rooted at
    // repo-a covering both, mirroring how a real multi-crate repo's
    // single top-level `repowise init` would see every file underneath.
    let discovered = discover_files(&repo_a_path).unwrap();
    let mut files = Vec::new();
    for entry in discovered {
        if matches!(entry.language, Language::Other) {
            continue;
        }
        let source = fs::read_to_string(&entry.path).unwrap();
        if let Some(record) =
            repowise_parser::parse_file(&entry.path, entry.language, &source).unwrap()
        {
            files.push(record);
        }
    }
    let combined = RepoIndex {
        root: repo_a_path.clone(),
        files,
        other_files: 0,
        indexed_commit: None,
    };
    combined.save(&repo_a_path).unwrap();

    // A second, adversarial repo that ALSO defines a module at the
    // dotted path `crate_x::shared` -- if the own-repo check were
    // missing, crate-y's import could spuriously resolve here instead.
    let repo_b_path = root.join("repo-b");
    write_crate(
        &repo_b_path,
        "crate_x",
        &[("src/shared.rs", "pub fn shared() -> i32 { 2 }\n")],
    );
    index_dir(&repo_b_path);

    let repos = vec![
        ResolvedWorkspaceRepo {
            name: "repo-a".to_string(),
            path: repo_a_path,
            index: None,
        },
        ResolvedWorkspaceRepo {
            name: "repo-b".to_string(),
            path: repo_b_path,
            index: None,
        },
    ];

    let report = workspace_architecture(&repos);

    assert!(
        report.edges.is_empty(),
        "expected no cross-repo edges (import resolves within repo-a itself), got {:?}",
        report.edges
    );
}

#[test]
fn workspace_architecture_ignores_relative_import_languages() {
    // TypeScript resolves its imports directly against the filesystem at
    // parse time rather than through a name -> file module map (see
    // `repowise-graph::cross_repo::MODULE_MAP_LANGUAGES`'s own doc
    // comment), so it has no cross-repo equivalent here -- unlike
    // Python, which joined the module-map languages this pass covers.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let repo_a_path = root.join("repo-a");
    fs::create_dir_all(&repo_a_path).unwrap();
    fs::write(repo_a_path.join("a.ts"), "export const x = 1;\n").unwrap();
    index_dir(&repo_a_path);

    let repo_b_path = root.join("repo-b");
    fs::create_dir_all(&repo_b_path).unwrap();
    fs::write(repo_b_path.join("b.ts"), "import { x } from './a';\n").unwrap();
    index_dir(&repo_b_path);

    let repos = vec![
        ResolvedWorkspaceRepo {
            name: "repo-a".to_string(),
            path: repo_a_path,
            index: None,
        },
        ResolvedWorkspaceRepo {
            name: "repo-b".to_string(),
            path: repo_b_path,
            index: None,
        },
    ];

    let report = workspace_architecture(&repos);
    assert!(report.edges.is_empty());
}

#[test]
fn workspace_architecture_resolves_a_cross_repo_python_import() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let repo_a_path = root.join("repo-a");
    fs::create_dir_all(&repo_a_path).unwrap();
    fs::write(repo_a_path.join("utils.py"), "def bar():\n    return 42\n").unwrap();
    let index_a = index_dir(&repo_a_path);

    let repo_b_path = root.join("repo-b");
    fs::create_dir_all(&repo_b_path).unwrap();
    fs::write(
        repo_b_path.join("main.py"),
        "from utils import bar\n\ndef top():\n    return bar()\n",
    )
    .unwrap();
    index_dir(&repo_b_path);

    let repos = vec![
        ResolvedWorkspaceRepo {
            name: "repo-a".to_string(),
            path: repo_a_path,
            index: None,
        },
        ResolvedWorkspaceRepo {
            name: "repo-b".to_string(),
            path: repo_b_path,
            index: None,
        },
    ];

    let report = workspace_architecture(&repos);

    // `from utils import bar` produces two `ImportRef`s (one for the
    // `utils` module path itself, one for the named `utils.bar` import
    // -- see `python.rs`'s `import_from_statement` handling), and this
    // pass reports one edge per resolved import site rather than
    // deduping to one edge per file pair, matching its existing
    // (pre-Python) behavior for a multi-item Rust `use` list.
    assert_eq!(report.edges.len(), 2);
    assert!(report
        .edges
        .iter()
        .all(|e| e.from_repo == "repo-b" && e.to_repo == "repo-a"));
    let utils_py = find_file(&index_a, "utils.py").path.clone();
    assert!(report.edges.iter().all(|e| e.to_file == utils_py));
    let mut import_paths: Vec<&str> = report
        .edges
        .iter()
        .map(|e| e.import_path.as_str())
        .collect();
    import_paths.sort();
    assert_eq!(import_paths, vec!["utils", "utils.bar"]);
}

#[test]
fn workspace_architecture_resolves_a_cross_repo_go_import() {
    // Go uses `/`-separated module paths (unlike Python/JVM/C#'s `.`) --
    // exercises `cross_repo::separator` picking the right one per
    // language rather than hardcoding Rust's `::`.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let repo_a_path = root.join("repo-a");
    fs::create_dir_all(&repo_a_path).unwrap();
    fs::write(
        repo_a_path.join("go.mod"),
        "module example.com/repoa\n\ngo 1.21\n",
    )
    .unwrap();
    fs::write(
        repo_a_path.join("bar.go"),
        "package repoa\n\nfunc Bar() int {\n\treturn 42\n}\n",
    )
    .unwrap();
    let index_a = index_dir(&repo_a_path);

    let repo_b_path = root.join("repo-b");
    fs::create_dir_all(&repo_b_path).unwrap();
    fs::write(
        repo_b_path.join("go.mod"),
        "module example.com/repob\n\ngo 1.21\n",
    )
    .unwrap();
    fs::write(
        repo_b_path.join("main.go"),
        "package main\n\nimport \"example.com/repoa\"\n\nfunc top() int {\n\treturn repoa.Bar()\n}\n",
    )
    .unwrap();
    index_dir(&repo_b_path);

    let repos = vec![
        ResolvedWorkspaceRepo {
            name: "repo-a".to_string(),
            path: repo_a_path,
            index: None,
        },
        ResolvedWorkspaceRepo {
            name: "repo-b".to_string(),
            path: repo_b_path,
            index: None,
        },
    ];

    let report = workspace_architecture(&repos);

    assert_eq!(report.edges.len(), 1);
    let edge = &report.edges[0];
    assert_eq!(edge.from_repo, "repo-b");
    assert_eq!(edge.to_repo, "repo-a");
    assert_eq!(edge.to_file, find_file(&index_a, "bar.go").path);
}

#[test]
fn workspace_architecture_reports_an_unindexed_repo_without_crashing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let repo_a_path = root.join("repo-a");
    write_crate(&repo_a_path, "repo-a", &[("src/lib.rs", "\n")]);
    index_dir(&repo_a_path);

    let repo_b_path = root.join("repo-b");
    fs::create_dir_all(&repo_b_path).unwrap(); // never indexed

    let repos = vec![
        ResolvedWorkspaceRepo {
            name: "repo-a".to_string(),
            path: repo_a_path,
            index: None,
        },
        ResolvedWorkspaceRepo {
            name: "repo-b".to_string(),
            path: repo_b_path,
            index: None,
        },
    ];

    let report = workspace_architecture(&repos);

    assert_eq!(report.repos.len(), 2);
    let repo_b_status = report.repos.iter().find(|r| r.name == "repo-b").unwrap();
    assert!(!repo_b_status.indexed);
    assert!(report.edges.is_empty());
}

#[test]
fn workspace_blast_radius_filters_to_a_specific_repo_and_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let repo_a_path = root.join("repo-a");
    write_crate(
        &repo_a_path,
        "repo-a",
        &[("src/foo.rs", "pub fn bar() -> i32 { 42 }\n")],
    );
    let index_a = index_dir(&repo_a_path);
    let foo_rs = find_file(&index_a, "foo.rs").path.clone();

    let repo_b_path = root.join("repo-b");
    write_crate(
        &repo_b_path,
        "repo-b",
        &[("src/lib.rs", "use repo_a::foo::bar;\n")],
    );
    index_dir(&repo_b_path);

    let repos = vec![
        ResolvedWorkspaceRepo {
            name: "repo-a".to_string(),
            path: repo_a_path.clone(),
            index: None,
        },
        ResolvedWorkspaceRepo {
            name: "repo-b".to_string(),
            path: repo_b_path,
            index: None,
        },
    ];

    let importers = workspace_blast_radius(&repos, "repo-a", &foo_rs);
    assert_eq!(importers.len(), 1);
    assert_eq!(importers[0].from_repo, "repo-b");

    let unrelated = workspace_blast_radius(&repos, "repo-a", &repo_a_path.join("src/nope.rs"));
    assert!(unrelated.is_empty());
}

#[test]
fn detect_workspace_cycles_finds_a_mutual_cross_repo_dependency() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let repo_a_path = root.join("repo-a");
    write_crate(
        &repo_a_path,
        "repo-a",
        &[(
            "src/lib.rs",
            "use repo_b::thing;\n\npub fn a_thing() -> i32 { 1 }\n",
        )],
    );
    index_dir(&repo_a_path);

    let repo_b_path = root.join("repo-b");
    write_crate(
        &repo_b_path,
        "repo-b",
        &[(
            "src/lib.rs",
            "use repo_a::a_thing;\n\npub fn thing() -> i32 { 2 }\n",
        )],
    );
    index_dir(&repo_b_path);

    let repos = vec![
        ResolvedWorkspaceRepo {
            name: "repo-a".to_string(),
            path: repo_a_path,
            index: None,
        },
        ResolvedWorkspaceRepo {
            name: "repo-b".to_string(),
            path: repo_b_path,
            index: None,
        },
    ];

    let cycles = detect_workspace_cycles(&repos);
    assert_eq!(cycles.len(), 1);
    let mut cycle = cycles[0].clone();
    cycle.sort();
    assert_eq!(cycle, vec!["repo-a".to_string(), "repo-b".to_string()]);
}
