//! End-to-end tests: real files on disk -> discover -> parse -> build graph.
//! Exercises the directory-layout-based module resolution in `modpath`,
//! which needs actual files (Rust resolution walks up to find `Cargo.toml`).

use repowise_core::{discover_files, FileRecord, Language, RepoIndex};
use repowise_graph::RepoGraph;
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
    RepoIndex {
        root: root.to_path_buf(),
        files,
        other_files,
    }
}

fn find_file<'a>(index: &'a RepoIndex, suffix: &str) -> &'a FileRecord {
    index
        .files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(suffix))
        .unwrap_or_else(|| panic!("no indexed file ending in {suffix}"))
}

#[test]
fn resolves_python_imports_and_calls() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("pkg")).unwrap();
    fs::write(root.join("pkg/__init__.py"), "").unwrap();
    fs::write(
        root.join("pkg/utils.py"),
        "def helper(x):\n    return x + 1\n\nclass Widget:\n    def render(self):\n        return helper(1)\n",
    )
    .unwrap();
    fs::write(
        root.join("main.py"),
        "from pkg.utils import helper, Widget\nimport os.path\n\ndef run():\n    w = Widget()\n    return helper(2)\n",
    )
    .unwrap();

    let index = index_dir(&root);
    let graph = RepoGraph::build(&index);

    let main_py = find_file(&index, "main.py").path.clone();
    let utils_py = find_file(&index, "utils.py").path.clone();

    let deps = graph.dependencies_of(&main_py);
    assert!(
        deps.contains(&utils_py),
        "expected main.py to depend on utils.py, got {deps:?}"
    );

    let dependents = graph.dependents_of(&utils_py);
    assert!(
        dependents.contains(&main_py),
        "expected utils.py to be depended on by main.py, got {dependents:?}"
    );

    // `import os.path` has no matching file in this tiny fixture, so it
    // must not resolve to anything (and should count as unresolved).
    assert!(graph.unresolved_imports >= 1);

    let helper_matches = graph.search("helper");
    assert_eq!(helper_matches.len(), 1);
}

#[test]
fn resolves_rust_module_imports_across_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo-crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "mod foo;\n\nfn top() -> i32 {\n    foo::bar()\n}\n",
    )
    .unwrap();
    fs::write(root.join("src/foo.rs"), "pub fn bar() -> i32 { 42 }\n").unwrap();

    let index = index_dir(&root);
    let graph = RepoGraph::build(&index);

    let lib_rs = find_file(&index, "lib.rs").path.clone();
    let foo_rs = find_file(&index, "foo.rs").path.clone();

    // `mod foo;` is resolved directly via Rust's file-layout convention
    // (see `resolve_mod_file` in repowise-parser), independent of `use`.
    let deps = graph.dependencies_of(&lib_rs);
    assert!(
        deps.contains(&foo_rs),
        "expected lib.rs to depend on foo.rs via `mod foo;`, got {deps:?}"
    );

    let matches = graph.search("bar");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].file, foo_rs);
}

#[test]
fn resolves_typescript_relative_imports_across_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/utils.ts"),
        "export function helper(x: number): number {\n  return x + 1;\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("src/main.ts"),
        "import { helper } from \"./utils\";\nimport \"left-pad\";\n\nexport function run(): number {\n  return helper(1);\n}\n",
    )
    .unwrap();

    let index = index_dir(&root);
    let graph = RepoGraph::build(&index);

    let main_ts = find_file(&index, "main.ts").path.clone();
    let utils_ts = find_file(&index, "utils.ts").path.clone();

    // The extensionless relative import is resolved directly at parse
    // time (see `resolve_relative_import` in `repowise-parser`), same as
    // Rust's `mod foo;`.
    let deps = graph.dependencies_of(&main_ts);
    assert!(
        deps.contains(&utils_ts),
        "expected main.ts to depend on utils.ts, got {deps:?}"
    );

    // A bare (npm-package) specifier has no `node_modules` to resolve
    // against, so it must count as unresolved rather than silently drop.
    assert!(graph.unresolved_imports >= 1);

    let matches = graph.search("helper");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].file, utils_ts);
}
