//! The portable, committable form of a [`RepoIndex`] (issue #378, see
//! `docs/adr/0002-portable-committable-index.md`).
//!
//! A working index is absolute-path and machine-specific: on this repo
//! the indexed root appears **27,884 times** in `.repowise/index.json`,
//! so committing it would be wrong on every machine but the one that
//! wrote it, and would leak one developer's directory layout to everyone
//! who reads it. A [`PortableIndex`] is the same data with every path
//! made repo-relative, sorted canonically, and stamped with a schema
//! version.
//!
//! # One choke point, not 215
//!
//! Every path rebase lives in this module. The rest of the workspace
//! keeps working with absolute paths exactly as before — the ADR
//! rejected rewriting all ~215 absolute-path sites on risk, not on
//! principle, and confining the conversion here is what makes that
//! rejection safe to revisit later.
//!
//! # What actually carries a path
//!
//! Seven fields, not two. Missing any one of them produces an artifact
//! that *looks* portable and silently isn't:
//!
//! | Field | Form |
//! | ----- | ---- |
//! | [`RepoIndex::root`] | `PathBuf` |
//! | `FileRecord::path` | `PathBuf` |
//! | `Symbol::file` | `PathBuf` |
//! | `Symbol::id` | `SymbolId` — the path is *inside* the string |
//! | `ImportRef::resolved_file` | `Option<PathBuf>` |
//! | `CallRef::caller` | `Option<SymbolId>` — path inside the string |
//! | `FieldAccessRef::method` | `SymbolId` — path inside the string |
//!
//! The three `SymbolId` fields are the easy ones to miss: they read as
//! opaque identifiers, and [`crate::Symbol::make_id`] builds them as
//! `{file}::{name}@{line}`.
//!
//! # Forward slashes, always
//!
//! Portable paths use `/` on every platform. A repo indexed on Linux and
//! read on Windows has to resolve to the same files, and `PathBuf::join`
//! accepts `/` on Windows, so normalising on export costs nothing and
//! makes the artifact genuinely cross-platform rather than
//! cross-machine-on-one-OS.
//!
//! # Canonical order
//!
//! Exported records are sorted, because `crate::discover_files` walks via
//! `ignore::WalkBuilder`, whose iteration follows filesystem `readdir`
//! order — reproducible on one machine by accident, not portable by
//! contract. An unsorted artifact would reorder itself between machines
//! and turn every re-export into an unreviewable diff.

use crate::{FileRecord, RepoIndex, SymbolId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Schema version of the portable artifact.
///
/// Bump on any change to the on-disk shape. A committed artifact
/// outlives the binary that wrote it, so a mismatch fails loudly (see
/// [`PortableIndex::into_anchored`]) rather than being silently
/// misparsed — the `serde(default)` leniency the working index uses is
/// right for a file you can always regenerate locally and wrong for one
/// meant to be read by a binary that didn't write it.
pub const PORTABLE_SCHEMA_VERSION: u32 = 1;

/// A [`RepoIndex`] with repo-relative paths, canonical ordering, and a
/// schema version — safe to commit and read on another machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortableIndex {
    pub schema_version: u32,
    /// The index itself. Its `root` is always `"."`: the real root is
    /// supplied by whoever loads it, since only they know where the repo
    /// lives on their machine.
    pub index: RepoIndex,
}

impl PortableIndex {
    /// Rebase `index` onto repo-relative paths, sort it canonically, and
    /// stamp the current schema version.
    ///
    /// Files outside `index.root` (which should not occur, but would
    /// otherwise be silently corrupted) keep their original path rather
    /// than being rewritten into something meaningless.
    pub fn from_index(index: &RepoIndex) -> Self {
        let root = index.root.clone();
        let mut out = index.clone();
        out.root = PathBuf::from(".");
        for file in &mut out.files {
            rebase_file(file, &root);
        }
        sort_index(&mut out);
        Self {
            schema_version: PORTABLE_SCHEMA_VERSION,
            index: out,
        }
    }

    /// Re-anchor onto `root`, producing an index indistinguishable from
    /// one built locally at that root.
    ///
    /// Errors on a schema-version mismatch. That is deliberate: reading
    /// a future artifact with an older binary would misparse quietly,
    /// and quiet misparse of a *committed* analysis is exactly the
    /// failure mode this format exists to prevent.
    pub fn into_anchored(self, root: &Path) -> anyhow::Result<RepoIndex> {
        if self.schema_version != PORTABLE_SCHEMA_VERSION {
            anyhow::bail!(
                "portable index is schema version {}, this repowise understands {} -- \
                 re-export it with a matching repowise, or upgrade",
                self.schema_version,
                PORTABLE_SCHEMA_VERSION
            );
        }
        let mut index = self.index;
        index.root = root.to_path_buf();
        for file in &mut index.files {
            anchor_file(file, root);
        }
        Ok(index)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let file = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path)
            .map_err(|e| anyhow::anyhow!("cannot read portable index {} ({e})", path.display()))?;
        Ok(serde_json::from_reader(file)?)
    }
}

/// `path` relative to `root`, with `/` separators. Paths not under
/// `root` are left alone.
fn to_relative(path: &Path, root: &Path) -> PathBuf {
    let rel = path.strip_prefix(root).unwrap_or(path);
    PathBuf::from(
        rel.components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

/// Rewrite the path prefix embedded in a `SymbolId`
/// (`{file}::{name}@{line}`).
///
/// Done as a string-prefix rewrite rather than by re-deriving the id
/// from its parts, because `CallRef::caller` and `FieldAccessRef::method`
/// are *references* to symbols that may live in another file — there is
/// nothing local to re-derive them from.
fn rebase_id(id: &SymbolId, root_str: &str) -> SymbolId {
    match id.strip_prefix(root_str) {
        Some(rest) => rest
            .trim_start_matches(['/', '\\'])
            .replace('\\', "/")
            .to_string(),
        None => id.clone(),
    }
}

fn anchor_id(id: &SymbolId, root_str: &str) -> SymbolId {
    // An id that already starts with the root is left alone, so
    // anchoring twice is a no-op rather than a corruption.
    if id.starts_with(root_str) {
        return id.clone();
    }
    format!("{root_str}{}{id}", std::path::MAIN_SEPARATOR)
}

fn root_str(root: &Path) -> String {
    root.to_string_lossy()
        .trim_end_matches(['/', '\\'])
        .to_string()
}

fn rebase_file(file: &mut FileRecord, root: &Path) {
    let rs = root_str(root);
    file.path = to_relative(&file.path, root);
    for sym in &mut file.symbols {
        sym.file = to_relative(&sym.file, root);
        sym.id = rebase_id(&sym.id, &rs);
    }
    for import in &mut file.imports {
        if let Some(resolved) = &import.resolved_file {
            import.resolved_file = Some(to_relative(resolved, root));
        }
    }
    for call in &mut file.calls {
        if let Some(caller) = &call.caller {
            call.caller = Some(rebase_id(caller, &rs));
        }
    }
    for access in &mut file.field_accesses {
        access.method = rebase_id(&access.method, &rs);
    }
}

fn anchor_file(file: &mut FileRecord, root: &Path) {
    let rs = root_str(root);
    file.path = root.join(&file.path);
    for sym in &mut file.symbols {
        sym.file = root.join(&sym.file);
        sym.id = anchor_id(&sym.id, &rs);
    }
    for import in &mut file.imports {
        if let Some(resolved) = &import.resolved_file {
            import.resolved_file = Some(root.join(resolved));
        }
    }
    for call in &mut file.calls {
        if let Some(caller) = &call.caller {
            call.caller = Some(anchor_id(caller, &rs));
        }
    }
    for access in &mut file.field_accesses {
        access.method = anchor_id(&access.method, &rs);
    }
}

/// Total, content-derived ordering — never insertion or `readdir` order.
fn sort_index(index: &mut RepoIndex) {
    index.files.sort_by(|a, b| a.path.cmp(&b.path));
    for file in &mut index.files {
        file.symbols
            .sort_by(|a, b| (a.start_line, &a.name, &a.id).cmp(&(b.start_line, &b.name, &b.id)));
        file.imports
            .sort_by(|a, b| (a.line, &a.path).cmp(&(b.line, &b.path)));
        file.calls
            .sort_by(|a, b| (a.line, &a.callee_name).cmp(&(b.line, &b.callee_name)));
        file.field_accesses
            .sort_by(|a, b| (a.line, &a.field_name).cmp(&(b.line, &b.field_name)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallRef, FieldAccessRef, ImportRef, Language, Symbol, SymbolKind};

    fn sym(root: &Path, file: &str, name: &str, line: usize) -> Symbol {
        let path = root.join(file);
        Symbol {
            id: Symbol::make_id(&path, name, line),
            name: name.to_string(),
            kind: SymbolKind::Function,
            file: path,
            start_line: line,
            end_line: line + 3,
            ..Default::default()
        }
    }

    fn fixture(root: &Path) -> RepoIndex {
        let a = root.join("src/a.rs");
        let b = root.join("src/b.rs");
        RepoIndex {
            root: root.to_path_buf(),
            other_files: 2,
            indexed_commit: Some("abc123def456".to_string()),
            files: vec![
                FileRecord {
                    path: b.clone(),
                    language: Language::Rust,
                    lines: 20,
                    symbols: vec![sym(root, "src/b.rs", "beta", 5)],
                    imports: vec![ImportRef {
                        path: "crate::a".to_string(),
                        line: 1,
                        resolved_file: Some(a.clone()),
                    }],
                    calls: vec![CallRef {
                        caller: Some(Symbol::make_id(&b, "beta", 5)),
                        callee_name: "alpha".to_string(),
                        line: 7,
                    }],
                    field_accesses: vec![FieldAccessRef {
                        method: Symbol::make_id(&b, "beta", 5),
                        field_name: "count".to_string(),
                        line: 6,
                    }],
                },
                FileRecord {
                    path: a,
                    language: Language::Rust,
                    lines: 10,
                    symbols: vec![sym(root, "src/a.rs", "alpha", 2)],
                    imports: Vec::new(),
                    calls: Vec::new(),
                    field_accesses: Vec::new(),
                },
            ],
        }
    }

    /// The whole point: no trace of the producing machine survives.
    #[test]
    fn every_path_carrying_field_is_rebased() {
        let root = Path::new("/home/someone/myrepo");
        let portable = PortableIndex::from_index(&fixture(root));
        let json = serde_json::to_string(&portable).unwrap();

        assert!(
            !json.contains("/home/someone"),
            "producing machine's layout leaked into the artifact: {json}"
        );
        assert_eq!(portable.index.root, PathBuf::from("."));

        let b = portable
            .index
            .files
            .iter()
            .find(|f| f.path == Path::new("src/b.rs"))
            .expect("relative path");
        assert_eq!(b.symbols[0].id, "src/b.rs::beta@5");
        assert_eq!(b.symbols[0].file, PathBuf::from("src/b.rs"));
        assert_eq!(b.imports[0].resolved_file, Some(PathBuf::from("src/a.rs")));
        assert_eq!(b.calls[0].caller.as_deref(), Some("src/b.rs::beta@5"));
        assert_eq!(b.field_accesses[0].method, "src/b.rs::beta@5");
    }

    #[test]
    fn round_trip_restores_every_field_exactly() {
        let root = Path::new("/home/someone/myrepo");
        let original = fixture(root);
        let restored = PortableIndex::from_index(&original)
            .into_anchored(root)
            .unwrap();

        assert_eq!(restored.root, original.root);
        assert_eq!(restored.other_files, original.other_files);
        assert_eq!(restored.indexed_commit, original.indexed_commit);
        assert_eq!(restored.files.len(), original.files.len());

        // Compare by path, since export sorts and the fixture is not in
        // sorted order to begin with.
        for orig in &original.files {
            let got = restored
                .files
                .iter()
                .find(|f| f.path == orig.path)
                .unwrap_or_else(|| panic!("{} missing after round trip", orig.path.display()));
            assert_eq!(got.language, orig.language);
            assert_eq!(got.lines, orig.lines);
            assert_eq!(got.symbols.len(), orig.symbols.len());
            for (g, o) in got.symbols.iter().zip(&orig.symbols) {
                assert_eq!(g.id, o.id);
                assert_eq!(g.file, o.file);
            }
            for (g, o) in got.imports.iter().zip(&orig.imports) {
                assert_eq!(g.resolved_file, o.resolved_file);
            }
            for (g, o) in got.calls.iter().zip(&orig.calls) {
                assert_eq!(g.caller, o.caller);
            }
            for (g, o) in got.field_accesses.iter().zip(&orig.field_accesses) {
                assert_eq!(g.method, o.method);
            }
        }
    }

    /// The property that makes the artifact reviewable: the same content
    /// under a different root exports byte-identically.
    #[test]
    fn the_same_repo_at_a_different_root_exports_byte_identically() {
        let one = PortableIndex::from_index(&fixture(Path::new("/home/alice/work/myrepo")));
        let two = PortableIndex::from_index(&fixture(Path::new("/Users/bob/myrepo")));
        assert_eq!(
            serde_json::to_string_pretty(&one).unwrap(),
            serde_json::to_string_pretty(&two).unwrap()
        );
    }

    /// `readdir` order is not portable, so export must impose its own.
    #[test]
    fn export_sorts_files_regardless_of_input_order() {
        let root = Path::new("/repo");
        let mut index = fixture(root);
        index.files.reverse();
        let portable = PortableIndex::from_index(&index);
        let paths: Vec<&Path> = portable
            .index
            .files
            .iter()
            .map(|f| f.path.as_path())
            .collect();
        assert_eq!(paths, vec![Path::new("src/a.rs"), Path::new("src/b.rs")]);
    }

    #[test]
    fn a_schema_version_mismatch_fails_loudly() {
        let root = Path::new("/repo");
        let mut portable = PortableIndex::from_index(&fixture(root));
        portable.schema_version = PORTABLE_SCHEMA_VERSION + 1;

        let err = portable.into_anchored(root).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("schema version"), "{msg}");
        assert!(
            msg.contains("re-export"),
            "the error must say what to do about it: {msg}"
        );
    }

    #[test]
    fn save_and_load_round_trip_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("nested/index.portable.json");
        let root = Path::new("/repo");

        PortableIndex::from_index(&fixture(root))
            .save(&out)
            .unwrap();
        let loaded = PortableIndex::load(&out).unwrap();

        assert_eq!(loaded.schema_version, PORTABLE_SCHEMA_VERSION);
        let restored = loaded.into_anchored(root).unwrap();
        assert_eq!(restored.files.len(), 2);
        assert_eq!(restored.root, root);
    }

    #[test]
    fn loading_a_missing_artifact_names_the_path() {
        let err = PortableIndex::load(Path::new("/definitely/not/here.json")).unwrap_err();
        assert!(err.to_string().contains("here.json"), "{err}");
    }

    /// Anchoring an already-anchored id must not stack root prefixes.
    #[test]
    fn anchoring_is_idempotent_on_ids() {
        let rs = "/repo";
        let once = anchor_id(&"src/a.rs::f@1".to_string(), rs);
        assert_eq!(once, "/repo/src/a.rs::f@1");
        assert_eq!(anchor_id(&once, rs), once);
    }
}
