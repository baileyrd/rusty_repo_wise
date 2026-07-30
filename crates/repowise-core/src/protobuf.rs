//! Protobuf model — issue #324, the buildable follow-up to #319's
//! design decision. A protobuf message/service/RPC isn't a function or
//! class in this port's own sense: it carries none of `Symbol`'s
//! per-symbol metrics (complexity, nesting depth, ...), so it gets its
//! own small parallel type instead of stretching `SymbolKind` to fit —
//! the same call already made for Docker build stages (#318), SQL/dbt
//! objects (#317), and OpenAPI objects (#323).
//!
//! A `Service` and its RPCs don't share one struct the way a schema
//! shares one with its properties: a service is a *named container*
//! for RPCs, not itself a "type with fields" the way a message is, so
//! each RPC gets its own flat `ProtoObject` entry (named
//! `"Service.Method"`) rather than being nested inside the service's
//! own object — the same flat-list shape `OpenApiObject` already uses
//! for endpoints (never nested under a resource).
//!
//! Computed on demand by `repowise_protobuf::collect_protobuf`, the
//! same way `repowise_sql::collect_sql`/`repowise_openapi::
//! collect_openapi` are — not folded into `RepoIndex`, so adding this
//! doesn't touch the many call sites across the workspace that
//! construct one.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtoObjectKind {
    Message,
    Service,
    Rpc,
}

impl ProtoObjectKind {
    pub fn label(&self) -> &'static str {
        match self {
            ProtoObjectKind::Message => "message",
            ProtoObjectKind::Service => "service",
            ProtoObjectKind::Rpc => "rpc",
        }
    }
}

/// A message, service, or RPC defined in a `.proto` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtoObject {
    /// A message's or service's own name, or `"Service.Method"` for an
    /// RPC.
    pub name: String,
    pub kind: ProtoObjectKind,
    pub file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    /// Field names, for a `Message`. `[input_type, output_type]` for an
    /// `Rpc` (its request/response message names). Empty for a
    /// `Service` itself -- its RPCs are their own separate objects, not
    /// nested here.
    pub fields: Vec<String>,
}
