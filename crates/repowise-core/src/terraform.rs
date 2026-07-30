//! Terraform model — issue #326, the buildable follow-up to #319's
//! design decision. Unlike SQL/dbt objects, OpenAPI objects, protobuf
//! objects, and GraphQL objects -- all some flavor of "a named type
//! with fields" -- a Terraform `resource` block isn't a schema at all:
//! it's a named instance whose shape Terraform can't know statically
//! (that requires loading the provider plugin). A `module` block is
//! different again: a reference to another Terraform configuration by
//! source path, not a type or an instance. Neither fits the
//! `name`/`kind`/`fields` shape the other three schema-format crates
//! share, so this gets two small, genuinely different parallel types
//! instead of forcing a shared one -- the same restraint that already
//! kept `DockerStage` separate from `SqlObject`.
//!
//! Computed on demand by `repowise_terraform::collect_terraform`, the
//! same way the other three schema-format crates' `collect_*`
//! functions are -- not folded into `RepoIndex`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A `resource "<resource_type>" "<name>" { ... }` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerraformResource {
    /// The provider-specific resource type, e.g. `aws_instance`.
    pub resource_type: String,
    pub name: String,
    pub file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
}

/// A `module "<name>" { source = "..." }` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerraformModule {
    pub name: String,
    /// The literal `source` attribute's value, if the block has one and
    /// it's a plain string (not an interpolated expression). `None`
    /// covers both "no `source` attribute" and "not a plain string" --
    /// this port doesn't evaluate HCL expressions.
    pub source: Option<String>,
    pub file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
}
