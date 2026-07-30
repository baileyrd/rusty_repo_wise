//! Tree-sitter based symbol/import/call extraction for supported languages.
//!
//! This is intentionally a *lightweight, best-effort* static analysis: names
//! are resolved by textual/AST heuristics rather than full type-checking, in
//! the same spirit as repowise's own tree-sitter-driven approach, but with
//! none of the semantic-analysis machinery a real compiler front-end has.

mod c;
mod cpp;
mod csharp;
mod dart;
pub mod docker;
mod go;
mod java;
mod javascript;
mod kotlin;
pub mod lightweight;
mod metrics;
mod php;
mod python;
mod ruby;
mod rust;
mod scala;
mod shell;
mod swift;

use repowise_core::{discover_files, FileRecord, Language, RepoIndex};
use std::path::Path;

/// Parse a single file's `source` and extract its symbols/imports/calls.
/// Returns `None` for languages we don't have an extractor for.
///
/// The 9 "Structural tier" languages (issue #70: Objective-C, R, Zig,
/// Julia, Elm, OCaml, Crystal, Nim, D) get a bare, zero-symbol
/// `FileRecord` via `structural_only` rather than `None` -- no
/// tree-sitter grammar exists for them, but giving them a real
/// `FileRecord` (rather than folding them into the `other_files` count
/// like a genuinely-`Other` file) makes them visible to
/// `RepoIndex.files`-driven views (`hotspots`, per-language file
/// counts, dashboard drill-down). Their hotspot score is always `0`
/// (churn × 0 complexity, since there are no symbols to sum
/// complexity over) -- git-history signal (churn, blame, co-change) is
/// all they get, matching repowise's own "Structural tier: git history
/// only" framing.
pub fn parse_file(
    path: &Path,
    language: Language,
    source: &str,
) -> anyhow::Result<Option<FileRecord>> {
    match language {
        Language::Rust => Ok(Some(rust::extract(path, source)?)),
        Language::Python => Ok(Some(python::extract(path, source)?)),
        Language::TypeScript => Ok(Some(javascript::extract_typescript(path, source)?)),
        Language::JavaScript => Ok(Some(javascript::extract_javascript(path, source)?)),
        Language::Java => Ok(Some(java::extract(path, source)?)),
        Language::Kotlin => Ok(Some(kotlin::extract(path, source)?)),
        Language::Go => Ok(Some(go::extract(path, source)?)),
        Language::Cpp => Ok(Some(cpp::extract(path, source)?)),
        Language::CSharp => Ok(Some(csharp::extract(path, source)?)),
        Language::Scala => Ok(Some(scala::extract(path, source)?)),
        Language::Ruby => Ok(Some(ruby::extract(path, source)?)),
        Language::C => Ok(Some(c::extract(path, source)?)),
        Language::Swift => Ok(Some(swift::extract(path, source)?)),
        Language::Php => Ok(Some(php::extract(path, source)?)),
        Language::Dart => Ok(Some(dart::extract(path, source)?)),
        Language::Shell => Ok(Some(shell::extract(path, source)?)),
        Language::ObjectiveC
        | Language::R
        | Language::Zig
        | Language::Julia
        | Language::Elm
        | Language::OCaml
        | Language::Crystal
        | Language::Nim
        | Language::D => Ok(Some(structural_only(path, language, source))),
        // Same "no grammar, but still visible" treatment as the
        // Structural tier above -- a Dockerfile's real content (build
        // stages, `COPY --from` edges) is the separate `DockerStage`
        // model `collect_docker_stages` produces, not part of
        // `FileRecord` at all.
        Language::Dockerfile => Ok(Some(structural_only(path, language, source))),
        // The "Lightweight" tier (issue #69): unlike the Structural
        // tier above, these get a real (if unresolved) import list --
        // see `lightweight`'s own module doc for why symbols/calls stay
        // empty and imports stay unresolved.
        Language::Elixir
        | Language::Clojure
        | Language::Haskell
        | Language::Lean
        | Language::Erlang
        | Language::FSharp => Ok(Some(lightweight::extract(path, language, source))),
        Language::Other => Ok(None),
    }
}

/// A bare `FileRecord` for a "Structural tier" language (see
/// `parse_file`'s own doc comment): no symbols/imports/calls/field
/// accesses, just the file's identity and line count so it's counted
/// and visible in `RepoIndex.files`-driven views.
fn structural_only(path: &Path, language: Language, source: &str) -> FileRecord {
    FileRecord {
        path: path.to_path_buf(),
        language,
        lines: source.lines().count(),
        symbols: Vec::new(),
        imports: Vec::new(),
        calls: Vec::new(),
        field_accesses: Vec::new(),
    }
}

/// Walk `root`, parse every file in a supported language, and return the
/// resulting index (not yet saved to disk). Shared by `repowise-cli`'s
/// `init`/`update` commands and `repowise-server`'s background reindex
/// job, so both stay in lockstep with exactly one implementation.
pub fn build_index(root: &Path) -> anyhow::Result<RepoIndex> {
    let root = root.canonicalize()?;
    let discovered = discover_files(&root)?;

    let mut files: Vec<FileRecord> = Vec::new();
    let mut other_files = 0usize;

    for entry in discovered {
        if matches!(entry.language, Language::Other) {
            other_files += 1;
            continue;
        }
        let source = match std::fs::read_to_string(&entry.path) {
            Ok(s) => s,
            Err(_) => {
                // Binary or unreadable file that happened to match an
                // extension; count it and move on.
                other_files += 1;
                continue;
            }
        };
        match parse_file(&entry.path, entry.language, &source)? {
            Some(record) => files.push(record),
            None => other_files += 1,
        }
    }

    Ok(RepoIndex {
        root,
        files,
        other_files,
        // Left unset here on purpose: this crate parses source and knows
        // nothing about git, and adding a dependency on `repowise-git`
        // just to stamp a SHA would put version control in the parsing
        // boundary. Whoever *persists* an index stamps it -- see
        // `repowise-cli`'s init/update.
        indexed_commit: None,
    })
}

/// Walk `root` and extract every Dockerfile's build stages and `COPY
/// --from` edges (issue #318). A separate pass from `build_index`,
/// deliberately: `DockerStage`/`DockerCopyFromEdge` are a parallel model
/// to `Symbol`/`FileRecord`, not part of `RepoIndex` -- the same
/// "computed on demand, not persisted in the index" shape as
/// `repowise_git::collect_commits` and `repowise_adr::mine`. Unreadable
/// files are skipped, matching `build_index`'s own tolerance for a
/// binary/unreadable file that happened to match.
pub fn collect_docker_stages(
    root: &Path,
) -> anyhow::Result<(
    Vec<repowise_core::docker::DockerStage>,
    Vec<repowise_core::docker::DockerCopyFromEdge>,
)> {
    let root = root.canonicalize()?;
    let discovered = discover_files(&root)?;

    let mut stages = Vec::new();
    let mut edges = Vec::new();
    for entry in discovered {
        if entry.language != Language::Dockerfile {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&entry.path) else {
            continue;
        };
        let (file_stages, file_edges) = docker::extract_stages(&entry.path, &source);
        stages.extend(file_stages);
        edges.extend(file_edges);
    }
    Ok((stages, edges))
}

/// Shared helpers used by the per-language extractors.
pub(crate) mod util {
    use tree_sitter::Node;

    pub fn text<'a>(node: Node, source: &'a str) -> &'a str {
        &source[node.byte_range()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_file_gives_every_structural_tier_language_a_bare_file_record() {
        let languages = [
            Language::ObjectiveC,
            Language::R,
            Language::Zig,
            Language::Julia,
            Language::Elm,
            Language::OCaml,
            Language::Crystal,
            Language::Nim,
            Language::D,
        ];
        let source = "line one\nline two\nline three\n";
        for language in languages {
            let path = PathBuf::from("example.src");
            let record = parse_file(&path, language, source)
                .unwrap()
                .unwrap_or_else(|| panic!("expected a FileRecord for {language:?}"));
            assert_eq!(record.language, language);
            assert_eq!(record.lines, 3);
            assert!(record.symbols.is_empty());
            assert!(record.imports.is_empty());
            assert!(record.calls.is_empty());
            assert!(record.field_accesses.is_empty());
        }
    }

    #[test]
    fn parse_file_gives_every_lightweight_tier_language_a_bare_record_with_no_matching_imports() {
        let languages = [
            Language::Elixir,
            Language::Clojure,
            Language::Haskell,
            Language::Lean,
            Language::Erlang,
            Language::FSharp,
        ];
        // None of these lines match any of the six import regexes, so
        // every language should still produce a real, empty-imports
        // `FileRecord` -- unlike `Language::Other`, which produces `None`.
        let source = "line one\nline two\nline three\n";
        for language in languages {
            let path = PathBuf::from("example.src");
            let record = parse_file(&path, language, source)
                .unwrap()
                .unwrap_or_else(|| panic!("expected a FileRecord for {language:?}"));
            assert_eq!(record.language, language);
            assert_eq!(record.lines, 3);
            assert!(record.symbols.is_empty());
            assert!(record.imports.is_empty());
            assert!(record.calls.is_empty());
            assert!(record.field_accesses.is_empty());
        }
    }

    #[test]
    fn parse_file_returns_none_for_a_truly_unrecognized_language() {
        let path = PathBuf::from("example.bin");
        let record = parse_file(&path, Language::Other, "whatever").unwrap();
        assert!(record.is_none());
    }

    #[test]
    fn build_index_counts_structural_tier_files_as_indexed_not_other() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("main.zig"), "// a zig file\nconst x = 1;\n").unwrap();

        let index = build_index(&root).unwrap();

        assert_eq!(index.files.len(), 1);
        assert_eq!(index.files[0].language, Language::Zig);
        assert_eq!(index.other_files, 0);
    }

    #[test]
    fn build_index_gives_a_dockerfile_a_bare_zero_symbol_record() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("Dockerfile"), "FROM alpine\nRUN true\n").unwrap();

        let index = build_index(&root).unwrap();

        assert_eq!(index.files.len(), 1);
        assert_eq!(index.files[0].language, Language::Dockerfile);
        assert!(index.files[0].symbols.is_empty());
        assert_eq!(index.other_files, 0);
    }

    #[test]
    fn collect_docker_stages_walks_the_repo_and_finds_a_real_dockerfile() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("Dockerfile"),
            "FROM rust:1.75 AS builder\nRUN cargo build --release\n\n\
             FROM debian:bookworm-slim\nCOPY --from=builder /app /app\n",
        )
        .unwrap();
        // Not a Dockerfile -- must not be picked up.
        std::fs::write(root.join("docker-compose.yml"), "services: {}\n").unwrap();

        let (stages, edges) = collect_docker_stages(&root).unwrap();

        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].name.as_deref(), Some("builder"));
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from_stage, 1);
        assert_eq!(edges[0].to_stage, 0);
    }

    #[test]
    fn build_index_gives_a_lightweight_tier_file_real_but_unresolved_imports() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("lib.ex"),
            "defmodule MyApp do\n  import Foo.Bar\n  alias Foo.Baz\nend\n",
        )
        .unwrap();

        let index = build_index(&root).unwrap();

        assert_eq!(index.files.len(), 1);
        let record = &index.files[0];
        assert_eq!(record.language, Language::Elixir);
        assert!(record.symbols.is_empty());
        assert_eq!(record.imports.len(), 2);
        assert!(record.imports.iter().all(|i| i.resolved_file.is_none()));
        assert_eq!(index.other_files, 0);
    }
}
