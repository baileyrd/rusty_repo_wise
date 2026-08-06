//! A slim, hook-facing sidecar: which files import each indexed file
//! (issue #333).
//!
//! `PostToolUse` fires after *every* matching tool call, so whatever it
//! reads has to be cheap on the order of milliseconds. The dependents of
//! a file are exactly the useful thing to say after an edit — "changing
//! this breaks N others" — but getting them the normal way means
//! `RepoIndex::load` (about 2s in a release build on this repo, since
//! import resolution needs the whole index) followed by
//! `RepoGraph::build`. Paid on every edit, that is unshippable.
//!
//! So the resolved dependent edges are written once, at index time, into
//! `.repowise/dependents.json`: 11.6 KB against an 8.4 MB index on this
//! repo, and the graph build that produces it costs 0.02s on top of an
//! index the caller already has.
//!
//! Lives in this crate rather than in `repowise-core` because import
//! resolution *is* this crate — `repowise-core` deliberately depends on
//! no other `repowise-*` crate. Callers that have just built an index
//! and a graph write the sidecar; everything else degrades to saying
//! nothing rather than to saying something stale.

use crate::RepoGraph;
use repowise_core::RepoIndex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// File name under `RepoIndex::INDEX_DIR`.
pub const DEPENDENTS_FILE: &str = "dependents.json";

/// Resolved reverse-import edges, keyed by repo-relative path.
///
/// Repo-relative rather than absolute so the file survives a clone into
/// a different directory, the same choice ADR-0002's portable index
/// makes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependents {
    pub schema_version: u32,
    /// Repo-relative path -> the repo-relative paths that import it.
    /// Files with no dependents are omitted rather than stored empty.
    pub dependents: BTreeMap<String, Vec<String>>,
}

impl Dependents {
    /// A reader that doesn't recognize the version ignores the file
    /// rather than guessing at its shape (ADR-0002's stance).
    pub const SCHEMA_VERSION: u32 = 1;

    pub fn path(root: &Path) -> PathBuf {
        root.join(RepoIndex::INDEX_DIR).join(DEPENDENTS_FILE)
    }

    /// Who imports `file`, or an empty slice when nothing does or the
    /// file isn't indexed. The distinction between "no dependents" and
    /// "not indexed" is deliberately not made here: a caller that needs
    /// it should ask the index.
    pub fn of(&self, relative_path: &str) -> &[String] {
        self.dependents
            .get(relative_path)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

/// Write the sidecar for an index the caller has already built a graph
/// for.
///
/// Best effort: a failure costs a hook its enrichment, never
/// correctness, so callers ignore the result the same way
/// `RepoIndex::save` treats its own status sidecar.
pub fn write_dependents(root: &Path, index: &RepoIndex, graph: &RepoGraph) -> std::io::Result<()> {
    let rel = |p: &Path| -> String {
        p.strip_prefix(&index.root)
            .unwrap_or(p)
            .display()
            .to_string()
    };

    let mut dependents = BTreeMap::new();
    for file in &index.files {
        let mut who: Vec<String> = graph
            .dependents_of(&file.path)
            .into_iter()
            .map(|p| rel(&p))
            .collect();
        if who.is_empty() {
            continue;
        }
        who.sort();
        dependents.insert(rel(&file.path), who);
    }

    let payload = Dependents {
        schema_version: Dependents::SCHEMA_VERSION,
        dependents,
    };
    let dir = root.join(RepoIndex::INDEX_DIR);
    std::fs::create_dir_all(&dir)?;
    let file = std::fs::File::create(dir.join(DEPENDENTS_FILE))?;
    serde_json::to_writer(file, &payload).map_err(std::io::Error::other)
}

/// Read the sidecar, or `None` when it is absent, unreadable, of an
/// unrecognized schema version, or **older than the index**.
///
/// The mtime check is the load-bearing one. A sidecar that predates the
/// index describes a previous state of the repo, and a hook reporting
/// last week's dependents as this edit's blast radius is worse than a
/// hook that says nothing: silence is obviously incomplete, a stale
/// number reads as fact.
pub fn load_dependents(root: &Path) -> Option<Dependents> {
    let sidecar = Dependents::path(root);
    let index = RepoIndex::index_path(root);

    let modified = |p: &Path| std::fs::metadata(p).ok()?.modified().ok();
    if modified(&sidecar)? < modified(&index)? {
        return None;
    }

    let file = std::fs::File::open(&sidecar).ok()?;
    let loaded: Dependents = serde_json::from_reader(file).ok()?;
    (loaded.schema_version == Dependents::SCHEMA_VERSION).then_some(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::{FileRecord, ImportRef, Language};

    /// `b.rs` declares `mod a;`, which Rust resolves deterministically
    /// to the sibling file -- so the graph has one import edge and `a.rs`
    /// has exactly one dependent.
    fn two_file_index(root: &Path) -> RepoIndex {
        std::fs::write(root.join("a.rs"), "pub fn a() {}\n").unwrap();
        std::fs::write(root.join("b.rs"), "mod a;\n").unwrap();
        RepoIndex {
            root: root.to_path_buf(),
            files: vec![
                FileRecord {
                    path: root.join("a.rs"),
                    language: Language::Rust,
                    lines: 1,
                    symbols: Vec::new(),
                    imports: Vec::new(),
                    calls: Vec::new(),
                    field_accesses: Vec::new(),
                },
                FileRecord {
                    path: root.join("b.rs"),
                    language: Language::Rust,
                    lines: 1,
                    symbols: Vec::new(),
                    imports: vec![ImportRef {
                        path: "a".to_string(),
                        line: 1,
                        resolved_file: Some(root.join("a.rs")),
                    }],
                    calls: Vec::new(),
                    field_accesses: Vec::new(),
                },
            ],
            other_files: 0,
            indexed_commit: None,
        }
    }

    #[test]
    fn writes_and_reads_back_repo_relative_dependents() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let index = two_file_index(&root);
        index.save(&root).unwrap();
        let graph = RepoGraph::build(&index);
        write_dependents(&root, &index, &graph).unwrap();

        let loaded = load_dependents(&root).expect("sidecar reads back");
        assert_eq!(loaded.of("a.rs"), ["b.rs".to_string()]);
        assert!(
            loaded.of("b.rs").is_empty(),
            "nothing imports b.rs, so it is omitted"
        );
        assert!(
            !loaded
                .dependents
                .values()
                .flatten()
                .any(|p| p.starts_with('/')),
            "paths must be repo-relative so the sidecar survives a clone: {:?}",
            loaded.dependents
        );
    }

    /// The check that matters: a sidecar older than the index describes
    /// a previous state, and reporting it as an edit's blast radius
    /// would be confidently wrong.
    #[test]
    fn a_sidecar_older_than_the_index_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let index = two_file_index(&root);
        index.save(&root).unwrap();
        write_dependents(&root, &index, &RepoGraph::build(&index)).unwrap();
        assert!(load_dependents(&root).is_some(), "fresh sidecar reads");

        let index_path = RepoIndex::index_path(&root);
        let newer = std::fs::metadata(&index_path).unwrap().modified().unwrap()
            + std::time::Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&index_path)
            .unwrap()
            .set_modified(newer)
            .unwrap();

        assert!(load_dependents(&root).is_none());
    }

    #[test]
    fn an_absent_sidecar_is_none_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        two_file_index(&root).save(&root).unwrap();
        assert!(load_dependents(&root).is_none());
    }

    #[test]
    fn an_unknown_schema_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let index = two_file_index(&root);
        index.save(&root).unwrap();
        write_dependents(&root, &index, &RepoGraph::build(&index)).unwrap();

        let path = Dependents::path(&root);
        let mut v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["schema_version"] = serde_json::json!(Dependents::SCHEMA_VERSION + 1);
        std::fs::write(&path, serde_json::to_string(&v).unwrap()).unwrap();

        assert!(load_dependents(&root).is_none());
    }
}
