//! OpenAPI model — issue #323, the buildable follow-up to #319's design
//! decision. An OpenAPI endpoint or schema isn't a function or class in
//! this port's own sense: it carries none of `Symbol`'s per-symbol
//! metrics (complexity, nesting depth, ...), so it gets its own small
//! parallel type instead of stretching `SymbolKind` to fit — the same
//! call already made for Docker build stages (#318) and SQL/dbt objects
//! (#317).
//!
//! Computed on demand by `repowise_openapi::collect_openapi`, the same
//! way `repowise_parser::collect_docker_stages`/`repowise_sql::
//! collect_sql` are — not folded into `RepoIndex`, so adding this
//! doesn't touch the many call sites across the workspace that
//! construct one. Unlike those two, there's no `Language::OpenApi`
//! variant either: telling an OpenAPI spec apart from any other YAML/
//! JSON file requires reading its content, not just its extension, and
//! `discover_files`'s per-file classification only ever looks at the
//! extension — see `repowise_openapi`'s own module doc for why that
//! stayed a separate walk instead.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpenApiObjectKind {
    /// A named entry under `components.schemas`.
    Schema,
    /// One HTTP method on one path (`paths.<path>.get`, `.post`, ...).
    Endpoint,
}

impl OpenApiObjectKind {
    pub fn label(&self) -> &'static str {
        match self {
            OpenApiObjectKind::Schema => "schema",
            OpenApiObjectKind::Endpoint => "endpoint",
        }
    }
}

/// A schema or endpoint defined in an OpenAPI 3.x document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenApiObject {
    /// The schema's name (its key under `components.schemas`), or an
    /// endpoint's `operationId` if it has one, else `"METHOD path"`
    /// (e.g. `"GET /orders/{id}"`).
    pub name: String,
    pub kind: OpenApiObjectKind,
    pub file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    /// Property names, for an object-typed schema. Empty for endpoints,
    /// for non-object schema kinds (string/number/array/etc.), and for
    /// `$ref`-only entries this port doesn't follow.
    pub fields: Vec<String>,
}
