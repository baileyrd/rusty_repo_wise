//! Filters for symbol/path search, shared by the `search` CLI command
//! and the `search_codebase` MCP tool so the two surfaces can't drift.
//!
//! # What's deliberately absent
//!
//! The reference also offers a `semantic`/`concept` mode, and this
//! doesn't. It is **not** stubbed: a `--mode semantic` that quietly
//! fell back to substring matching would be worse than not offering the
//! flag, because the caller would believe they had asked a different
//! question and got an answer to it.
//!
//! The reason is **persistence, not capability** -- a distinction an
//! earlier version of this module got wrong. `repowise_llm::embed` and
//! `cosine_similarity` exist, and `POST /api/chat` uses them for real
//! semantic retrieval today. What's missing is a stored embedding
//! index: that endpoint re-embeds the entire corpus on every call,
//! which is a defensible tradeoff for an occasional chat message and an
//! indefensible one for `search`, which is meant to be cheap and
//! frequent. See issue #302.

use repowise_core::{FileRecord, Language, SymbolKind};
use std::path::Path;

/// What a query is matched against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    /// Symbol names only. The historical behavior, and still the
    /// default so existing invocations are unaffected.
    #[default]
    Symbol,
    /// File paths only. Previously impossible to express -- paths
    /// weren't searchable at all, so "which file is the config loader
    /// in" had no query that answered it.
    Path,
    /// Both, merged.
    Hybrid,
}

impl SearchMode {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "symbol" => Ok(SearchMode::Symbol),
            "path" => Ok(SearchMode::Path),
            "hybrid" => Ok(SearchMode::Hybrid),
            // Named explicitly rather than lumped in with typos: a
            // caller reaching for `semantic` has a specific expectation,
            // and "unknown mode" would read as a spelling mistake when
            // the real answer is that this port can't do it at all.
            "semantic" | "concept" => Err(
                "semantic/concept search isn't available: this port computes embeddings \
                 (see `repowise-llm`) but doesn't persist an index of them, and \
                 re-embedding the corpus per query would make search far too expensive \
                 to be worth it (see issue #302). It is deliberately not offered rather \
                 than silently falling back to substring matching, which would answer a \
                 different question than the one asked."
                    .to_string(),
            ),
            other => Err(format!(
                "unknown mode {other:?} -- expected symbol, path, or hybrid"
            )),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SearchMode::Symbol => "symbol",
            SearchMode::Path => "path",
            SearchMode::Hybrid => "hybrid",
        }
    }
}

/// Coarse role of a file, inferred from its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Implementation,
    Test,
    Config,
    Doc,
    /// Nothing in the path said anything either way.
    ///
    /// A real bucket, not a dumping ground: this classification is
    /// path-convention guesswork, and a file that matches no convention
    /// must be visible as unclassified rather than silently defaulting
    /// into `Implementation` and being wrongly included or excluded by
    /// a filter.
    Unknown,
}

impl FileKind {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "implementation" | "impl" => Ok(FileKind::Implementation),
            "test" => Ok(FileKind::Test),
            "config" => Ok(FileKind::Config),
            "doc" | "docs" => Ok(FileKind::Doc),
            "unknown" => Ok(FileKind::Unknown),
            other => Err(format!(
                "unknown kind {other:?} -- expected implementation, test, config, doc, or unknown"
            )),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            FileKind::Implementation => "implementation",
            FileKind::Test => "test",
            FileKind::Config => "config",
            FileKind::Doc => "doc",
            FileKind::Unknown => "unknown",
        }
    }
}

/// Classify a file by path convention.
///
/// Heuristic, and only as good as the conventions it knows. Anything it
/// can't place lands in [`FileKind::Unknown`] rather than being guessed
/// into a bucket -- see that variant's doc.
pub fn classify(file: &FileRecord, root: &Path) -> FileKind {
    let rel = file
        .path
        .strip_prefix(root)
        .unwrap_or(&file.path)
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();

    let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
    let segments: Vec<&str> = rel.split('/').collect();

    // Tests first: a test file is still a test when it lives under
    // `src/`, and the reverse mistake (calling it implementation) is
    // the one that makes `--kind implementation` results wrong.
    let test_dir = segments
        .iter()
        .any(|s| matches!(*s, "tests" | "test" | "__tests__" | "spec" | "specs"));
    let test_name = name.starts_with("test_")
        || name.contains("_test.")
        || name.contains(".test.")
        || name.contains(".spec.")
        || name.contains("_spec.");
    if test_dir || test_name {
        return FileKind::Test;
    }

    if matches!(file.language, Language::Other) {
        // Unparsed files are classified by name alone; there's no
        // language signal to lean on.
        if name.ends_with(".md") || segments.contains(&"docs") || segments.contains(&"doc") {
            return FileKind::Doc;
        }
        return FileKind::Unknown;
    }

    if name.ends_with(".md") {
        return FileKind::Doc;
    }
    if segments.contains(&"docs") || segments.contains(&"doc") {
        return FileKind::Doc;
    }
    if matches!(
        name.as_str(),
        "build.rs" | "conf.py" | "setup.py" | "gulpfile.js" | "webpack.config.js"
    ) || name.ends_with(".config.js")
        || name.ends_with(".config.ts")
    {
        return FileKind::Config;
    }

    FileKind::Implementation
}

/// Does `path` contain `query` (case-insensitive), compared
/// repo-relative so a query never accidentally matches the machine's
/// own directory layout above the repo root?
pub fn path_matches(file: &Path, root: &Path, query: &str) -> bool {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .to_lowercase()
        .contains(&query.to_lowercase())
}

/// Parse a `SymbolKind` filter value.
pub fn parse_symbol_kind(s: &str) -> Result<SymbolKind, String> {
    match s.to_lowercase().as_str() {
        "function" => Ok(SymbolKind::Function),
        "method" => Ok(SymbolKind::Method),
        "struct" => Ok(SymbolKind::Struct),
        "enum" => Ok(SymbolKind::Enum),
        "trait" => Ok(SymbolKind::Trait),
        "class" => Ok(SymbolKind::Class),
        "module" => Ok(SymbolKind::Module),
        "mixin" => Ok(SymbolKind::Mixin),
        other => Err(format!(
            "unknown symbol_kind {other:?} -- expected function, method, struct, enum, \
             trait, class, module, or mixin"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::FileRecord;
    use std::path::PathBuf;

    fn record(rel: &str, language: Language) -> FileRecord {
        FileRecord {
            path: PathBuf::from("/repo").join(rel),
            language,
            lines: 10,
            symbols: Vec::new(),
            imports: Vec::new(),
            calls: Vec::new(),
            field_accesses: Vec::new(),
        }
    }

    fn kind_of(rel: &str, language: Language) -> FileKind {
        classify(&record(rel, language), Path::new("/repo"))
    }

    #[test]
    fn tests_are_recognized_by_directory_and_by_name() {
        assert_eq!(kind_of("tests/api.rs", Language::Rust), FileKind::Test);
        assert_eq!(kind_of("src/foo_test.go", Language::Go), FileKind::Test);
        assert_eq!(
            kind_of("src/test_thing.py", Language::Python),
            FileKind::Test
        );
        assert_eq!(
            kind_of("src/a.test.ts", Language::TypeScript),
            FileKind::Test
        );
        assert_eq!(kind_of("spec/thing.rb", Language::Ruby), FileKind::Test);
    }

    /// A test living under `src/` is still a test. Getting this
    /// backwards is what makes `--kind implementation` quietly wrong.
    #[test]
    fn a_test_under_src_is_still_a_test() {
        assert_eq!(
            kind_of("src/deep/nested/thing_test.rs", Language::Rust),
            FileKind::Test
        );
    }

    #[test]
    fn ordinary_source_is_implementation() {
        assert_eq!(
            kind_of("src/lib.rs", Language::Rust),
            FileKind::Implementation
        );
        assert_eq!(
            kind_of("app/models/user.rb", Language::Ruby),
            FileKind::Implementation
        );
    }

    #[test]
    fn docs_and_config_are_recognized() {
        assert_eq!(kind_of("docs/guide.rs", Language::Rust), FileKind::Doc);
        assert_eq!(kind_of("build.rs", Language::Rust), FileKind::Config);
        assert_eq!(
            kind_of("webpack.config.js", Language::JavaScript),
            FileKind::Config
        );
    }

    /// The bucket that keeps the heuristic honest.
    #[test]
    fn an_unrecognized_unparsed_file_is_unknown_not_implementation() {
        assert_eq!(
            kind_of("assets/blob.bin", Language::Other),
            FileKind::Unknown
        );
    }

    #[test]
    fn mode_parsing_rejects_semantic_with_a_specific_reason() {
        assert_eq!(SearchMode::parse("symbol"), Ok(SearchMode::Symbol));
        assert_eq!(SearchMode::parse("PATH"), Ok(SearchMode::Path));
        let err = SearchMode::parse("semantic").unwrap_err();
        assert!(
            err.contains("doesn't persist an index"),
            "a caller asking for semantic search deserves the real reason -- the gap is \
             persistence, not capability -- and not 'unknown mode': {err}"
        );
        assert!(
            !err.contains("doesn't have"),
            "must not repeat the earlier claim that this port lacks embeddings: {err}"
        );
        assert!(SearchMode::parse("nonsense")
            .unwrap_err()
            .contains("unknown mode"));
    }

    #[test]
    fn path_matching_is_relative_to_the_repo_root() {
        let root = Path::new("/home/someone/repo");
        let file = root.join("src/config.rs");
        assert!(path_matches(&file, root, "config"));
        assert!(path_matches(&file, root, "SRC/CON"));
        // Must not match on directories above the root -- otherwise a
        // query silently matches the producing machine's layout.
        assert!(!path_matches(&file, root, "someone"));
    }

    #[test]
    fn symbol_kind_parsing_names_the_valid_values() {
        assert_eq!(parse_symbol_kind("Function"), Ok(SymbolKind::Function));
        let err = parse_symbol_kind("widget").unwrap_err();
        assert!(err.contains("expected function"), "{err}");
    }
}
