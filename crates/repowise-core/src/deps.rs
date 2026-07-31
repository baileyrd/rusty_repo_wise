//! External (third-party, package-manager) dependency model — issue
//! #353, upstream's Architecture section's Dependencies sub-view. A
//! parallel model to `Symbol`/`FileRecord`, computed on demand by
//! `repowise_external_deps::collect_dependencies`, not part of `RepoIndex` --
//! the same "no `Language` variant, no wiki/graph integration yet"
//! treatment the other schema-format crates (`repowise_sql`,
//! `repowise_openapi`, `repowise_protobuf`, `repowise_graphql`,
//! `repowise_terraform`) already use.
//!
//! **Declared, not resolved.** This lists what a manifest *says* a repo
//! depends on -- the version constraint as written, e.g. `^1.2.3` or
//! `*`. It does not walk a lockfile, does not resolve transitive
//! dependencies, and does not detect version conflicts. `cargo
//! tree`/`npm ls`/`pip list` already do full resolution per ecosystem;
//! this port's value-add is a single cross-language view of what's
//! *declared*, not a second implementation of what those tools already
//! do better.
//!
//! **Third-party only.** A workspace-internal path dependency (e.g. this
//! very repo's `repowise-cli` depending on `repowise-server` via `{
//! path = "../repowise-server" }`) is not a third-party package and is
//! excluded -- see each ecosystem's extraction function in
//! `repowise_external_deps` for the exact filter.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Which section of the manifest a dependency was declared in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyKind {
    /// A normal runtime/production dependency.
    Direct,
    /// A development-only dependency (Cargo's `[dev-dependencies]`,
    /// npm's `devDependencies`, Composer's `require-dev`).
    Dev,
    /// A build-time-only dependency (Cargo's `[build-dependencies]`).
    Build,
}

impl DependencyKind {
    pub fn label(&self) -> &'static str {
        match self {
            DependencyKind::Direct => "direct",
            DependencyKind::Dev => "dev",
            DependencyKind::Build => "build",
        }
    }
}

/// One third-party package declared in a dependency manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalDependency {
    pub name: String,
    /// The version constraint exactly as written in the manifest (e.g.
    /// `^1.2.3`, `>=2.0,<3.0`, `*`). `None` when the manifest declares
    /// the package with no version constraint at all (e.g. a bare
    /// `requests` line in `requirements.txt`).
    pub version: Option<String>,
    pub kind: DependencyKind,
    /// The package ecosystem/registry this dependency resolves against:
    /// `"cargo"`, `"npm"`, `"pypi"`, `"go"`, or `"composer"`.
    pub ecosystem: &'static str,
    /// The manifest file this dependency was declared in.
    pub file: PathBuf,
    pub line: usize,
}
