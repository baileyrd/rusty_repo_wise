//! JSON Graph Format export (issue #244), tested the same way as
//! `resolution.rs`: real files on disk -> parse -> build graph -> export.
//! Using a real parse rather than hand-built `Symbol` fixtures means the
//! unresolved-import case below is genuinely unresolved, not simulated.

use repowise_core::{discover_files, Language, RepoIndex};
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

/// A crate with one resolvable internal import (`mod helper;`) and one
/// deliberately unresolvable external one (`use serde::Serialize;`).
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src").join("lib.rs"),
        "mod helper;\nuse serde::Serialize;\npub fn entry() { helper::help(); }\n",
    )
    .unwrap();
    fs::write(
        root.join("src").join("helper.rs"),
        "pub fn help() -> u32 { 1 }\n",
    )
    .unwrap();
    dir
}

fn document(root: &Path) -> serde_json::Value {
    let index = index_dir(root);
    let graph = RepoGraph::build(&index);
    serde_json::to_value(graph.to_json_graph(&index)).unwrap()
}

#[test]
fn emits_jgf_v2_shape() {
    let dir = fixture();
    let json = document(dir.path());
    let g = &json["graph"];
    assert_eq!(g["directed"], true);
    assert_eq!(g["type"], "repowise-dependency-graph");
    assert!(g["nodes"].is_object(), "JGF v2 keys nodes by id");
    assert!(g["edges"].is_array());
}

#[test]
fn nodes_carry_the_metadata_dot_and_mermaid_would_have_discarded() {
    // This is why JGF was chosen over DOT/Mermaid -- if these fields
    // aren't here, the format choice bought nothing.
    let dir = fixture();
    let json = document(dir.path());
    let nodes = json["graph"]["nodes"].as_object().unwrap();

    let file = nodes
        .iter()
        .find(|(k, _)| k.ends_with("src/lib.rs"))
        .expect("lib.rs node")
        .1;
    assert_eq!(file["metadata"]["kind"], "file");
    assert_eq!(file["metadata"]["language"], "Rust");
    assert!(file["metadata"]["lines"].as_u64().unwrap() > 0);

    let symbol = nodes
        .values()
        .find(|n| n["metadata"]["kind"] == "symbol")
        .expect("at least one symbol node");
    assert!(symbol["metadata"]["symbol_kind"].is_string());
    assert!(symbol["metadata"]["start_line"].as_u64().is_some());
    assert!(symbol["metadata"]["complexity"].as_u64().is_some());
}

#[test]
fn paths_are_repo_relative_with_forward_slashes() {
    // An export carrying the producing machine's absolute paths, or
    // Windows separators, wouldn't be portable.
    let dir = fixture();
    let json = document(dir.path());
    let root = dir.path().to_string_lossy().to_string();
    for key in json["graph"]["nodes"].as_object().unwrap().keys() {
        assert!(!key.contains('\\'), "{key}");
        assert!(!key.contains(&root), "absolute path leaked: {key}");
    }
}

#[test]
fn a_resolved_internal_import_becomes_an_edge() {
    let dir = fixture();
    let json = document(dir.path());
    let edges = json["graph"]["edges"].as_array().unwrap();
    assert!(
        edges.iter().any(|e| e["relation"] == "imports"
            && e["source"].as_str().unwrap().ends_with("src/lib.rs")
            && e["target"].as_str().unwrap().ends_with("src/helper.rs")),
        "{edges:#?}"
    );
    assert!(edges.iter().any(|e| e["relation"] == "contains"));
}

#[test]
fn unresolved_references_are_declared_rather_than_silently_dropped() {
    // The property that matters most. `serde` cannot resolve to a node,
    // so it has no edge -- but it must not vanish without trace, or a
    // consumer would read the missing edge as "no such dependency".
    let dir = fixture();
    let json = document(dir.path());
    let unresolved = &json["graph"]["metadata"]["unresolved"];

    assert!(
        unresolved["imports"].as_u64().unwrap() > 0,
        "{unresolved:#?}"
    );
    let stems: Vec<&str> = unresolved["import_stems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        stems
            .iter()
            .any(|s| s.contains("erialize") || *s == "serde"),
        "{stems:?}"
    );
    assert!(unresolved["note"]
        .as_str()
        .unwrap()
        .contains("partial by construction"));
}

#[test]
fn no_edge_points_at_a_node_that_does_not_exist() {
    let dir = fixture();
    let json = document(dir.path());
    let nodes = json["graph"]["nodes"].as_object().unwrap();
    for e in json["graph"]["edges"].as_array().unwrap() {
        for end in ["source", "target"] {
            let id = e[end].as_str().unwrap();
            assert!(nodes.contains_key(id), "dangling {end}: {id}");
        }
    }
}

#[test]
fn output_is_deterministic_so_exports_are_diffable() {
    let dir = fixture();
    let a = serde_json::to_string(&document(dir.path())).unwrap();
    let b = serde_json::to_string(&document(dir.path())).unwrap();
    assert_eq!(a, b);
}
