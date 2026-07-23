//! End-to-end test: real files on disk, indexed by the actual pipeline,
//! rendered to a real dashboard file. No git repo or docs/adr set up
//! here — that exercises the graceful-degradation paths (hotspots/
//! decisions sections falling back to their "not available" placeholders)
//! that `render`'s unit tests already cover directly.

use repowise_core::{discover_files, FileRecord, Language, RepoIndex};

fn build_and_save_index(root: &std::path::Path) {
    let discovered = discover_files(root).unwrap();
    let mut files: Vec<FileRecord> = Vec::new();
    let mut other_files = 0;
    for entry in discovered {
        if matches!(entry.language, Language::Other) {
            other_files += 1;
            continue;
        }
        let source = std::fs::read_to_string(&entry.path).unwrap();
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
}

#[test]
fn writes_a_dashboard_covering_overview_and_health() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("lib.rs"), "pub fn helper() -> i32 { 1 }\n").unwrap();
    build_and_save_index(&root);

    let path = repowise_dashboard::generate(&root).unwrap();
    assert!(path.ends_with("dashboard/index.html"));

    let html = std::fs::read_to_string(&path).unwrap();
    assert!(html.contains("<title>repowise dashboard</title>"));
    assert!(html.contains("1 indexed file(s)"));
    assert!(html.contains(">function</td><td class=\"num\">1<"));
    assert!(html.contains("No git history found"));
    assert!(html.contains("No decisions found"));
    // Symbols index section: the indexed function shows up in the table
    // with a working kind filter.
    assert!(html.contains("id=\"symbol-kind-filter\""));
    assert!(html.contains(">helper<"));
    assert!(html.contains("data-kind=\"function\""));
}

#[test]
fn links_files_to_their_wiki_page_only_once_docs_has_been_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    // `foo.rs` needs a dependent to show up in the overview's
    // "Most depended-on files" table at all -- `mod foo;` gives it one,
    // same fixture shape used in repowise-graph's own resolution tests.
    std::fs::write(
        root.join("lib.rs"),
        "mod foo;\n\nfn top() -> i32 {\n    foo::bar()\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("foo.rs"), "pub fn bar() -> i32 { 42 }\n").unwrap();
    build_and_save_index(&root);

    // No wiki page yet -- the file's mention must render as plain text,
    // not a broken link.
    let path = repowise_dashboard::generate(&root).unwrap();
    let html = std::fs::read_to_string(&path).unwrap();
    assert!(html.contains(">foo.rs<"));
    assert!(!html.contains("<a href"));

    // `repowise docs` (simulated here directly) writes a wiki page for
    // this file -- regenerating the dashboard should now link to it.
    let wiki_dir = root.join(".repowise").join("wiki");
    std::fs::create_dir_all(&wiki_dir).unwrap();
    std::fs::write(wiki_dir.join("foo.rs.md"), "# foo.rs\n").unwrap();

    let path = repowise_dashboard::generate(&root).unwrap();
    let html = std::fs::read_to_string(&path).unwrap();
    assert!(html.contains("<a href=\"../wiki/foo.rs.md\">foo.rs</a>"));
}
