//! SQL/dbt model — issue #317, the buildable follow-up to #67's design
//! decision. A SQL table/view/function/procedure isn't a function or
//! class in this port's own sense: it carries none of `Symbol`'s
//! per-symbol metrics (complexity, nesting depth, ...), so it gets its
//! own small parallel type instead of stretching `SymbolKind` to fit —
//! the same call #318 made for Docker build stages.
//!
//! dbt's `ref()`/`source()` lineage is data lineage (this model *reads
//! from* that table), a different semantic from `ImportRef`/`CallRef`
//! ("this file's code invokes that file's code") that would quietly
//! corrupt hotspot/health/coupling scoring if merged into those. It gets
//! its own edge type, `LineageEdge`, read only by whatever consumes it.
//!
//! Computed on demand by `repowise_sql::collect_sql`, the same way
//! `repowise_git::collect_commits`/`repowise_adr::mine`/
//! `repowise_parser::collect_docker_stages` are — not folded into
//! `RepoIndex`, so adding this doesn't touch the many call sites across
//! the workspace that construct one.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SqlObjectKind {
    Table,
    View,
    Function,
    Procedure,
}

impl SqlObjectKind {
    pub fn label(&self) -> &'static str {
        match self {
            SqlObjectKind::Table => "table",
            SqlObjectKind::View => "view",
            SqlObjectKind::Function => "function",
            SqlObjectKind::Procedure => "procedure",
        }
    }
}

/// A SQL object defined in a `.sql` file: either a literal `CREATE
/// TABLE`/`VIEW`/`FUNCTION`/`PROCEDURE` statement, or (for a dbt model
/// file -- one containing `{{ ... }}` Jinja templating, which this port
/// doesn't attempt to compile) the whole file, treated as one `View`
/// named after its file stem, per dbt's own "the file *is* the model"
/// convention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlObject {
    pub name: String,
    pub kind: SqlObjectKind,
    pub file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    /// Column names, where the source makes them cheap to read off
    /// (a `CREATE TABLE`'s column list). Best-effort and
    /// parser-dependent: empty for views/functions/procedures, and for
    /// a `CREATE TABLE ... AS SELECT` whose columns come from the
    /// query rather than an explicit list.
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineageKind {
    /// dbt's `{{ ref('model_name') }}` -- a reference to another dbt
    /// model in this same project, resolvable against the other
    /// `SqlObject`s this port discovers.
    Ref,
    /// dbt's `{{ source('source_name', 'table_name') }}` -- a reference
    /// to a raw external table declared in a `.yml` source config, not
    /// a `.sql` file. Always unresolved in this port: there is no
    /// `.sql` file for it to point to, matching Swift's/Dart's own
    /// unresolved-imports precedent (see `repowise-graph`).
    Source,
}

/// A dbt `ref()`/`source()` lineage reference, resolved when possible
/// against another `SqlObject` discovered in the same repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageEdge {
    pub from: PathBuf,
    pub kind: LineageKind,
    /// The raw name(s) passed to `ref()`/`source()`, joined with `.`
    /// for `source('src', 'table')`'s two-argument form.
    pub name: String,
    pub resolved_file: Option<PathBuf>,
    pub line: usize,
}
