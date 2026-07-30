//! GraphQL model — issue #325, the buildable follow-up to #319's design
//! decision. A GraphQL type/query/mutation isn't a function or class in
//! this port's own sense: it carries none of `Symbol`'s per-symbol
//! metrics (complexity, nesting depth, ...), so it gets its own small
//! parallel type instead of stretching `SymbolKind` to fit — the same
//! call already made for Docker build stages, SQL/dbt objects, OpenAPI
//! objects, and protobuf objects.
//!
//! `Query`/`Mutation`/`Subscription` aren't a distinct GraphQL
//! *syntax* -- they're ordinary `type` definitions the spec treats as
//! schema roots (by name, `Query`/`Mutation`/`Subscription`, unless an
//! explicit `schema { query: X, mutation: Y }` block overrides which
//! type plays which role). Each field on one of those root types gets
//! its own flat `GraphQlObject` entry (named `"Type.field"`) rather
//! than being nested inside the root type's own object -- the same
//! flat-list shape `OpenApiObject`'s endpoints and `ProtoObject`'s RPCs
//! already use, never nested under a parent.
//!
//! Computed on demand by `repowise_graphql::collect_graphql`, the same
//! way the other three schema-format crates' `collect_*` functions
//! are — not folded into `RepoIndex`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphQlObjectKind {
    /// Any named type that isn't a schema root's field: `type`,
    /// `interface`, `union`, `enum`, `input`, `scalar`.
    Type,
    /// A field on the schema's `Query` root type.
    Query,
    /// A field on the schema's `Mutation` root type.
    Mutation,
    /// A field on the schema's `Subscription` root type.
    Subscription,
}

impl GraphQlObjectKind {
    pub fn label(&self) -> &'static str {
        match self {
            GraphQlObjectKind::Type => "type",
            GraphQlObjectKind::Query => "query",
            GraphQlObjectKind::Mutation => "mutation",
            GraphQlObjectKind::Subscription => "subscription",
        }
    }
}

/// A type, query, mutation, or subscription defined in a GraphQL SDL
/// file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphQlObject {
    /// A type's own name, or `"RootType.field"` for a query/mutation/
    /// subscription.
    pub name: String,
    pub kind: GraphQlObjectKind,
    pub file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    /// Field names for an object/interface/input type; value names for
    /// an enum; member type names for a union. Empty for a scalar, and
    /// for a query/mutation/subscription field itself (its own
    /// arguments aren't modeled).
    pub fields: Vec<String>,
}
