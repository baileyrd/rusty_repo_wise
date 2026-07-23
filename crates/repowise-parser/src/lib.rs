//! Tree-sitter based symbol/import/call extraction for supported languages.
//!
//! This is intentionally a *lightweight, best-effort* static analysis: names
//! are resolved by textual/AST heuristics rather than full type-checking, in
//! the same spirit as repowise's own tree-sitter-driven approach, but with
//! none of the semantic-analysis machinery a real compiler front-end has.

mod javascript;
mod metrics;
mod python;
mod rust;

use repowise_core::{FileRecord, Language};
use std::path::Path;

/// Parse a single file's `source` and extract its symbols/imports/calls.
/// Returns `None` for languages we don't have an extractor for.
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
        Language::Other => Ok(None),
    }
}

/// Shared helpers used by the per-language extractors.
pub(crate) mod util {
    use tree_sitter::Node;

    pub fn text<'a>(node: Node, source: &'a str) -> &'a str {
        &source[node.byte_range()]
    }
}
