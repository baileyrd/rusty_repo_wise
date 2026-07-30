//! Docker build-stage model — issue #318, the prototype for #68's
//! config/data-format tier. A Dockerfile stage isn't a function or
//! class: it carries none of `Symbol`'s per-symbol metrics (complexity,
//! nesting depth, ...), so it gets its own small parallel type instead
//! of stretching `SymbolKind` to fit — the same call #317 made for SQL
//! objects.
//!
//! Computed on demand by `repowise_parser::collect_docker_stages`, the
//! same way `repowise_git::collect_commits`/`repowise_adr::mine` are —
//! not folded into `RepoIndex`, so adding this doesn't touch the many
//! call sites across the workspace that construct one.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One `FROM` stage in a Dockerfile, spanning from its `FROM` line to
/// the line before the next `FROM` (or end of file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerStage {
    pub file: PathBuf,
    /// 0-based position among stages in this file — how `COPY
    /// --from=<N>` addresses a stage that was never given a name.
    pub index: usize,
    /// `AS <name>` if the `FROM` line gave one.
    pub name: Option<String>,
    /// The `FROM` line's image argument, verbatim — may itself be
    /// another stage's name (a multi-stage build) or an external image
    /// reference.
    pub base_image: String,
    pub start_line: usize,
    pub end_line: usize,
}

/// A `COPY --from=<stage>` reference that resolved to another stage in
/// the *same* file — the closest thing this format has to an import
/// edge. A `--from` value that doesn't match any earlier stage (an
/// external image, e.g. `COPY --from=alpine:3.18`) produces no edge:
/// there's nothing in this repo for it to point to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerCopyFromEdge {
    pub file: PathBuf,
    /// Index of the stage containing the `COPY --from`.
    pub from_stage: usize,
    /// Index of the referenced stage.
    pub to_stage: usize,
    pub line: usize,
}
