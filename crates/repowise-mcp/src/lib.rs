//! An MCP (Model Context Protocol) server exposing this port's index,
//! dependency graph, and health scoring as agent-facing tools over
//! stdio, using the official `rmcp` SDK.
//!
//! Implements `get_overview`, `search_codebase`, `get_context`,
//! `get_risk`, `get_change_risk`, `get_symbol`, `get_why`,
//! `get_dead_code`, plus `list_repos`, `get_architecture` and
//! `get_blast_radius` (the last three have no counterpart in the
//! reference) — the ones
//! whose backing data (the index, the resolved dependency graph, health
//! findings, `repowise-git`'s hotspot/churn/bug-fix and diff-shape data,
//! `repowise-adr`'s mined decisions, or raw source on disk) already
//! exists in this port. `get_change_risk`'s score is a documented
//! fixed-weight heuristic over diff-shape metrics (files/lines touched,
//! subsystems affected, change concentration, author experience) — the
//! original feeds the same kind of metrics into a pre-trained ML model,
//! which this port has no labeled corpus or training pipeline to
//! reproduce (see issue #42 and the category-A "ML-calibrated scoring"
//! issue). `get_dead_code`'s confidence tiers are likewise a documented
//! approximation of the original's model (which also folds in a
//! runtime-load risk factor — reflection, dynamic dispatch, entry
//! points — this port has no way to assess); see
//! `repowise_health::find_dead_code` for the exact tiering logic.
//!
//! Every tool response carries a `_meta` block (see the [`meta`] module)
//! reporting how long the call took and — for tools that answer from the
//! index — which commit that index was built against, whether HEAD has
//! moved past it, and how old it is. Without it, an answer from a
//! months-old index is indistinguishable from one built against HEAD,
//! which is the one way this server can mislead a caller while appearing
//! to work perfectly.
//!
//! `list_repos` was the first slice of issue #64 (multi-repo/workspace
//! support): when this server was started with `--workspace <path>`, it
//! reports every repo that workspace file configures, each with its
//! indexed status and file count if a prior `repowise init`/`update`
//! has run there (via `repowise_workspace::repo_status`). Returns an
//! empty list rather than an error when no workspace was given, same
//! degrade-gracefully shape as every other optional-data tool in this
//! port.
//!
//! `get_architecture`/`get_blast_radius` are the next slice: real
//! cross-repo Rust `use` resolution (via
//! `repowise_workspace::workspace_architecture`/`workspace_blast_radius`,
//! themselves built on `repowise_graph::cross_repo_import_edges`).
//! `get_architecture` degrades to empty lists like `list_repos` (no
//! specific target, nothing to error about). `get_blast_radius` takes a
//! specific repo+file target, so it errors like `get_context` instead:
//! no workspace configured, an unknown repo name, or an unindexed
//! file/repo are all reported as errors rather than an empty result.
//! Both are Rust-only for now -- the only language this port anchors to
//! a `Cargo.toml`-derived crate name; every other language's cross-repo
//! imports are left unresolved, deliberately, for a future slice.

use repowise_core::{RepoIndex, SymbolKind};
use repowise_graph::RepoGraph;
use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    schemars, tool, tool_router,
    transport::stdio,
    ErrorData, ServiceExt,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

mod meta;

pub use meta::{Envelope, Meta};

/// Elapsed milliseconds since `started`, saturating.
///
/// `u64` milliseconds rather than the raw `Duration` because this
/// crosses the wire as JSON, and saturating rather than `as` so a
/// pathologically long call reports a large number instead of wrapping
/// around to a small one.
fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

struct CacheEntry {
    mtime: SystemTime,
    index: RepoIndex,
    graph: RepoGraph,
}

/// Start the MCP server over stdio, indexing `root` (which must already
/// have a `.repowise/index.json` from a prior `repowise init`/`update`).
/// `workspace`, if given, is a workspace TOML file's path (see
/// `repowise-workspace`) -- opts into the `list_repos` tool.
pub async fn run(root: PathBuf, workspace: Option<PathBuf>) -> anyhow::Result<()> {
    let workspace_repos = workspace
        .map(|path| repowise_workspace::load_resolved(&path))
        .transpose()?;
    let server = RepowiseServer::new(root, workspace_repos);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[derive(Clone)]
struct RepowiseServer {
    root: PathBuf,
    workspace_repos: Option<Vec<repowise_workspace::ResolvedWorkspaceRepo>>,
    cache: Arc<Mutex<Option<CacheEntry>>>,
}

impl RepowiseServer {
    fn new(
        root: PathBuf,
        workspace_repos: Option<Vec<repowise_workspace::ResolvedWorkspaceRepo>>,
    ) -> Self {
        Self {
            root,
            workspace_repos,
            cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Load the index and graph, reporting whether the in-memory cache
    /// answered.
    ///
    /// The third element feeds `_meta.cached`. It's returned from here
    /// rather than inferred by callers because this is the only place
    /// that knows — a caller can't tell a cache hit from a fast disk.
    fn load(&self) -> Result<(RepoIndex, RepoGraph, bool), ErrorData> {
        let index_path = self.root.join(".repowise").join("index.json");
        let current_mtime = std::fs::metadata(&index_path)
            .and_then(|m| m.modified())
            .ok();

        if let (Some(mtime), Ok(guard)) = (current_mtime, self.cache.lock()) {
            if let Some(entry) = guard.as_ref() {
                if entry.mtime == mtime {
                    return Ok((entry.index.clone(), entry.graph.clone(), true));
                }
            }
        }

        let index = RepoIndex::load(&self.root).map_err(|e| {
            ErrorData::internal_error(
                format!("failed to load index at {}: {e}", self.root.display()),
                None,
            )
        })?;
        let graph = RepoGraph::build(&index);

        if let (Some(mtime), Ok(mut guard)) = (current_mtime, self.cache.lock()) {
            *guard = Some(CacheEntry {
                mtime,
                index: index.clone(),
                graph: graph.clone(),
            });
        }

        Ok((index, graph, false))
    }

    /// Wrap a payload from a tool that answered **from the index**, so
    /// the response carries that index's provenance and freshness.
    fn indexed<T>(
        &self,
        data: T,
        index: &RepoIndex,
        started: Instant,
        cached: bool,
    ) -> Json<Envelope<T>> {
        Json(Envelope::new(
            data,
            Meta::build(&self.root, index, elapsed_ms(started), cached),
        ))
    }

    /// Wrap a payload from a tool that did **not** consult this
    /// server's index — git-only (`get_change_risk`) or workspace tools
    /// reading other repos entirely. See [`Meta::timing_only`] for why
    /// these deliberately carry no staleness fields.
    fn untracked<T>(&self, data: T, started: Instant) -> Json<Envelope<T>> {
        Json(Envelope::new(data, Meta::timing_only(elapsed_ms(started))))
    }

    fn resolve_file(&self, file: &str) -> PathBuf {
        let path = Path::new(file);
        let target = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        target.canonicalize().unwrap_or(target)
    }

    /// Resolve a `get_why` target to a file path: if it exactly matches
    /// an indexed symbol's id, that symbol's own file; otherwise treated
    /// as a file path (same rules as `resolve_file`).
    fn resolve_target(&self, target: &str, index: &RepoIndex) -> PathBuf {
        index
            .files
            .iter()
            .flat_map(|f| &f.symbols)
            .find(|s| s.id == target)
            .map(|s| s.file.clone())
            .unwrap_or_else(|| self.resolve_file(target))
    }

    /// Resolve a `get_blast_radius` file argument against a NAMED
    /// workspace repo's own root, not `self.root` (this server's own
    /// single indexed root, which is almost always a different repo
    /// entirely from the one a cross-repo blast-radius query targets).
    fn resolve_file_in_repo(&self, file: &str, repo_root: &Path) -> PathBuf {
        let path = Path::new(file);
        let target = if path.is_absolute() {
            path.to_path_buf()
        } else {
            repo_root.join(path)
        };
        target.canonicalize().unwrap_or(target)
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct SearchParams {
    /// Case-insensitive substring to match against symbol names.
    query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct ContextParams {
    /// Path to the file, absolute or relative to the indexed root.
    file: String,
}

fn default_top_n() -> usize {
    10
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RiskParams {
    /// Path to a specific file to assess, absolute or relative to the
    /// indexed root. If omitted, returns the riskiest files repo-wide
    /// instead (ranked by hotspot score).
    #[serde(default)]
    file: Option<String>,
    /// How many files to return when `file` is omitted. Ignored when
    /// `file` is set (exactly one result either way).
    #[serde(default = "default_top_n")]
    top_n: usize,
}

impl Default for RiskParams {
    fn default() -> Self {
        RiskParams {
            file: None,
            top_n: default_top_n(),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct GetSymbolParams {
    /// A symbol's `id`, as returned by `search_codebase`/`get_context`.
    symbol_id: String,
    /// Extra lines of surrounding source to include on each side of the
    /// symbol's own line span, clamped to the file's bounds. Defaults to
    /// `0` (just the symbol's own span).
    #[serde(default)]
    context_lines: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct WhyParams {
    /// File paths (absolute or relative to the indexed root) or symbol
    /// ids (as returned by `search_codebase`/`get_context`) to filter
    /// mined decisions by. A decision matches if its body links to any
    /// target's file. Omit or leave empty to return every mined decision.
    #[serde(default)]
    targets: Vec<String>,
}

fn default_dead_code_limit() -> usize {
    50
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DeadCodeParams {
    /// Minimum confidence tier to include: `"low"`, `"medium"`, or
    /// `"high"` (case-insensitive). Defaults to `"low"` (everything).
    /// Ignored when `safe_only` is set.
    #[serde(default)]
    min_confidence: Option<String>,
    /// When `true`, return only the `"high"` confidence tier — the
    /// closest this tool gets to the reference's "safe to delete"
    /// designation. Even so, this is a claim about this port's own
    /// resolution heuristics finding no in-repo reference, NOT a
    /// guarantee of runtime safety: reflection, dynamic dispatch, and
    /// entry points are all invisible to this port's static call graph.
    #[serde(default)]
    safe_only: bool,
    /// Maximum number of candidates to return. Defaults to 50.
    #[serde(default = "default_dead_code_limit")]
    limit: usize,
}

impl Default for DeadCodeParams {
    fn default() -> Self {
        DeadCodeParams {
            min_confidence: None,
            safe_only: false,
            limit: default_dead_code_limit(),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct ChangeRiskParams {
    /// A single commit, or a `base..head` range, to assess. Defaults to
    /// `HEAD` (the most recent commit) when omitted.
    #[serde(default)]
    revspec: Option<String>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct LanguageCount {
    language: String,
    file_count: usize,
}

#[derive(Serialize, schemars::JsonSchema)]
struct SymbolKindCount {
    kind: String,
    count: usize,
}

#[derive(Serialize, schemars::JsonSchema)]
struct DependedOnFile {
    file: String,
    dependent_count: usize,
}

#[derive(Serialize, schemars::JsonSchema)]
struct OverviewOutput {
    file_count: usize,
    other_file_count: usize,
    total_lines: usize,
    by_language: Vec<LanguageCount>,
    symbol_counts: Vec<SymbolKindCount>,
    import_edges: usize,
    call_edges: usize,
    unresolved_imports: usize,
    unresolved_calls: usize,
    most_depended_on: Vec<DependedOnFile>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct SymbolMatch {
    /// Stable identifier for this symbol, usable with `get_symbol` to
    /// fetch its raw source text.
    id: String,
    name: String,
    kind: String,
    file: String,
    line: usize,
}

#[derive(Serialize, schemars::JsonSchema)]
struct SearchOutput {
    matches: Vec<SymbolMatch>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct HealthFindingOutput {
    kind: String,
    symbol: Option<String>,
    line: Option<usize>,
    detail: String,
}

#[derive(Serialize, schemars::JsonSchema)]
struct ContextOutput {
    file: String,
    symbols: Vec<SymbolMatch>,
    dependencies: Vec<String>,
    dependents: Vec<String>,
    health_score: f64,
    health_findings: Vec<HealthFindingOutput>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct FileRisk {
    file: String,
    /// churn × total cyclomatic complexity of the file's symbols (see
    /// `repowise_git::Hotspot`) — 0 for a file with no git history
    /// (unborn repo, uncommitted file, or `repowise-git` unavailable).
    hotspot_score: usize,
    /// Raw commit count touching this file. 0 under the same conditions
    /// as `hotspot_score`.
    churn: usize,
    /// Commits touching this file whose message matched a bug-fix
    /// keyword (see `repowise-git`). 0 under the same conditions.
    bugfix_commits: usize,
    health_score: f64,
    health_findings: Vec<HealthFindingOutput>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct RiskOutput {
    /// One entry when `file` was given in the request; up to `top_n`
    /// entries (highest hotspot score first) when it was omitted.
    files: Vec<FileRisk>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct ChangeRiskOutput {
    revspec: String,
    lines_added: usize,
    lines_deleted: usize,
    files_touched: usize,
    subsystems_touched: usize,
    /// `0.0..=1.0`; how evenly the changed lines are spread across the
    /// touched files (`0.0` = concentrated in one file, `1.0` = spread
    /// perfectly evenly). See `repowise_git::ChangeRisk` for the formula.
    concentration: f64,
    author: String,
    author_prior_commits: usize,
    /// `0.0..=10.0`, higher is riskier. A documented fixed-weight
    /// heuristic over the fields above — **not** a calibrated
    /// probability, and not the reference repowise's trained-model score
    /// (see the module doc comment).
    score: f64,
}

#[derive(Serialize, schemars::JsonSchema)]
struct GetSymbolOutput {
    id: String,
    name: String,
    kind: String,
    file: String,
    /// The returned `source`'s actual line span, after padding by
    /// `context_lines` and clamping to the file's bounds — not
    /// necessarily equal to the symbol's own `start_line..end_line`.
    start_line: usize,
    end_line: usize,
    source: String,
}

#[derive(Serialize, schemars::JsonSchema)]
struct DecisionOutput {
    id: String,
    title: String,
    /// `"adr:<file>"` or `"commit:<short hash> by <author>"`.
    source: String,
    /// Raw `Status:` line value (ADR source only).
    status: Option<String>,
    /// Normalized `ADR-XXXX` this decision is superseded by, if any.
    superseded_by: Option<String>,
    /// Raw `Date:` line value (ADR source only).
    date: Option<String>,
    linked_files: Vec<String>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct WhyOutput {
    decisions: Vec<DecisionOutput>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct DeadCodeCandidateOutput {
    file: String,
    symbol: String,
    line: usize,
    /// `"low"`, `"medium"`, or `"high"` — see the tool description and
    /// `repowise_health::find_dead_code` for the exact tiering logic.
    /// Not a runtime-safety guarantee at any tier.
    confidence: String,
    /// Why this candidate isn't `"high"` confidence (empty for `"high"`).
    risk_factors: Vec<String>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct DeadCodeOutput {
    candidates: Vec<DeadCodeCandidateOutput>,
    /// Total candidates matching the requested `min_confidence`/
    /// `safe_only` filter, before `limit` truncated the list — lets a
    /// caller tell "there were only 3" from "there were 300 and you're
    /// seeing the first 50".
    total_matching: usize,
}

#[derive(Serialize, schemars::JsonSchema)]
struct RepoStatusOutput {
    name: String,
    path: String,
    indexed: bool,
    file_count: Option<usize>,
    other_file_count: Option<usize>,
}

impl From<repowise_workspace::RepoStatus> for RepoStatusOutput {
    fn from(s: repowise_workspace::RepoStatus) -> Self {
        RepoStatusOutput {
            name: s.name,
            path: s.path.display().to_string(),
            indexed: s.indexed,
            file_count: s.file_count,
            other_file_count: s.other_file_count,
        }
    }
}

#[derive(Serialize, schemars::JsonSchema)]
struct ListReposOutput {
    repos: Vec<RepoStatusOutput>,
}

const ARCHITECTURE_EDGES_LIMIT: usize = 200;

#[derive(Serialize, schemars::JsonSchema)]
struct RepoEdgeSummaryOutput {
    from_repo: String,
    to_repo: String,
    edge_count: usize,
}

impl From<repowise_workspace::RepoEdgeSummary> for RepoEdgeSummaryOutput {
    fn from(e: repowise_workspace::RepoEdgeSummary) -> Self {
        RepoEdgeSummaryOutput {
            from_repo: e.from_repo,
            to_repo: e.to_repo,
            edge_count: e.edge_count,
        }
    }
}

#[derive(Serialize, schemars::JsonSchema)]
struct CrossRepoEdgeOutput {
    from_repo: String,
    from_file: String,
    line: usize,
    to_repo: String,
    to_file: String,
    import_path: String,
}

impl From<repowise_graph::CrossRepoImportEdge> for CrossRepoEdgeOutput {
    fn from(e: repowise_graph::CrossRepoImportEdge) -> Self {
        CrossRepoEdgeOutput {
            from_repo: e.from_repo,
            from_file: e.from_file.display().to_string(),
            line: e.line,
            to_repo: e.to_repo,
            to_file: e.to_file.display().to_string(),
            import_path: e.import_path,
        }
    }
}

#[derive(Serialize, schemars::JsonSchema)]
struct ArchitectureOutput {
    repos: Vec<RepoStatusOutput>,
    repo_edges: Vec<RepoEdgeSummaryOutput>,
    edges: Vec<CrossRepoEdgeOutput>,
    /// Total resolved cross-repo edges before `edges` was truncated to
    /// `ARCHITECTURE_EDGES_LIMIT` -- lets a caller tell "there were only
    /// 3" from "there were 300 and you're seeing the first 200", same
    /// transparency `DeadCodeOutput.total_matching` gives.
    total_edges: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct BlastRadiusParams {
    /// Name of the workspace repo the target file lives in, as
    /// configured in the workspace TOML this server was started with.
    repo: String,
    /// Path to the file within that repo, absolute or relative to that
    /// repo's own root -- NOT this server's own indexed root.
    file: String,
}

#[derive(Serialize, schemars::JsonSchema)]
struct BlastRadiusOutput {
    /// Other workspace repos' files that directly (one hop, not
    /// transitive) cross-repo-import the target file -- files that
    /// would need review if the target's public API changed.
    importers: Vec<CrossRepoEdgeOutput>,
}

fn display_rel(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[tool_router(server_handler)]
impl RepowiseServer {
    #[tool(
        name = "get_overview",
        description = "Summary stats about the indexed codebase: file/language/symbol counts, dependency-graph edge counts, and the most depended-on files. Requires a prior `repowise init`/`update`."
    )]
    fn get_overview(&self) -> Result<Json<Envelope<OverviewOutput>>, ErrorData> {
        let started = Instant::now();
        let (index, graph, cached) = self.load()?;
        let overview = graph.overview(&index);
        Ok(self.indexed(
            OverviewOutput {
                file_count: overview.file_count,
                other_file_count: overview.other_file_count,
                total_lines: overview.total_lines,
                by_language: overview
                    .by_language
                    .into_iter()
                    .map(|(language, file_count)| LanguageCount {
                        language,
                        file_count,
                    })
                    .collect(),
                symbol_counts: overview
                    .symbol_counts
                    .into_iter()
                    .map(|(kind, count)| SymbolKindCount { kind, count })
                    .collect(),
                import_edges: overview.import_edges,
                call_edges: overview.call_edges,
                unresolved_imports: overview.unresolved_imports,
                unresolved_calls: overview.unresolved_calls,
                most_depended_on: overview
                    .most_depended_on
                    .into_iter()
                    .map(|(file, dependent_count)| DependedOnFile {
                        file: display_rel(&file, &index.root),
                        dependent_count,
                    })
                    .collect(),
            },
            &index,
            started,
            cached,
        ))
    }

    #[tool(
        name = "search_codebase",
        description = "Case-insensitive substring search over indexed symbol names (functions, methods, classes, structs, etc.), returning each match's kind, file, and line number."
    )]
    fn search_codebase(
        &self,
        Parameters(SearchParams { query }): Parameters<SearchParams>,
    ) -> Result<Json<Envelope<SearchOutput>>, ErrorData> {
        let started = Instant::now();
        if query.trim().is_empty() {
            return Err(ErrorData::invalid_params("query must not be empty", None));
        }
        let (index, graph, cached) = self.load()?;
        let mut matches: Vec<SymbolMatch> = graph
            .search(&query)
            .into_iter()
            .map(|sym| SymbolMatch {
                id: sym.id.clone(),
                name: sym.name.clone(),
                kind: sym.kind.label().to_string(),
                file: display_rel(&sym.file, &index.root),
                line: sym.start_line,
            })
            .collect();
        matches.sort_by(|a, b| a.name.cmp(&b.name).then(a.file.cmp(&b.file)));
        Ok(self.indexed(SearchOutput { matches }, &index, started, cached))
    }

    #[tool(
        name = "get_context",
        description = "Complete context for one file in a single call: its symbols, resolved dependencies/dependents, and health findings/score. Built to replace the several separate reads (search, deps, health) an agent would otherwise need to piece this together itself."
    )]
    fn get_context(
        &self,
        Parameters(ContextParams { file }): Parameters<ContextParams>,
    ) -> Result<Json<Envelope<ContextOutput>>, ErrorData> {
        let started = Instant::now();
        let (index, graph, cached) = self.load()?;
        let target = self.resolve_file(&file);

        let Some(record) = index.files.iter().find(|f| f.path == target) else {
            return Err(ErrorData::resource_not_found(
                format!(
                    "{file} is not an indexed file under {}",
                    index.root.display()
                ),
                None,
            ));
        };

        let mut symbols: Vec<SymbolMatch> = record
            .symbols
            .iter()
            .filter(|s| !matches!(s.kind, SymbolKind::Module))
            .map(|sym| SymbolMatch {
                id: sym.id.clone(),
                name: sym.name.clone(),
                kind: sym.kind.label().to_string(),
                file: display_rel(&sym.file, &index.root),
                line: sym.start_line,
            })
            .collect();
        symbols.sort_by_key(|s| s.line);

        let dependencies = graph
            .dependencies_of(&target)
            .into_iter()
            .map(|p| display_rel(&p, &index.root))
            .collect();
        let dependents = graph
            .dependents_of(&target)
            .into_iter()
            .map(|p| display_rel(&p, &index.root))
            .collect();

        let health = repowise_health::analyze(&index, &graph);
        let file_health = health
            .file_scores
            .iter()
            .find(|f| f.file == target)
            .map(|f| f.score)
            .unwrap_or(10.0);
        let health_findings = health
            .findings
            .iter()
            .filter(|f| f.file == target)
            .map(|f| HealthFindingOutput {
                kind: f.kind.label().to_string(),
                symbol: f.symbol.clone(),
                line: f.line,
                detail: f.detail.clone(),
            })
            .collect();

        Ok(self.indexed(
            ContextOutput {
                file: display_rel(&target, &index.root),
                symbols,
                dependencies,
                dependents,
                health_score: file_health,
                health_findings,
            },
            &index,
            started,
            cached,
        ))
    }

    #[tool(
        name = "get_risk",
        description = "Risk assessment from git-history analytics and health findings, essentially `get_context` plus hotspot data. Given `file`, returns that file's hotspot score, churn, bug-fix-commit count, and health findings. Given no `file`, returns the `top_n` riskiest files repo-wide, ranked by (recency-weighted) hotspot score. Git data degrades to zero/empty when the indexed root isn't a git repository, rather than erroring."
    )]
    fn get_risk(
        &self,
        Parameters(RiskParams { file, top_n }): Parameters<RiskParams>,
    ) -> Result<Json<Envelope<RiskOutput>>, ErrorData> {
        let started = Instant::now();
        let (index, graph, cached) = self.load()?;
        let health = repowise_health::analyze(&index, &graph);
        // Not every indexed root is a git repository (or has git
        // available at all) — degrade to "no git data" rather than
        // failing the whole call, same tradeoff `repowise-dashboard`
        // already makes for its hotspots section.
        let analytics = repowise_git::GitAnalytics::collect(&self.root).ok();

        if let Some(file) = file {
            let target = self.resolve_file(&file);
            if !index.files.iter().any(|f| f.path == target) {
                return Err(ErrorData::resource_not_found(
                    format!(
                        "{file} is not an indexed file under {}",
                        index.root.display()
                    ),
                    None,
                ));
            }
            let risk = file_risk(&target, &index, analytics.as_ref(), &health);
            return Ok(self.indexed(RiskOutput { files: vec![risk] }, &index, started, cached));
        }

        let files = analytics
            .as_ref()
            .map(|a| repowise_git::hotspots(&index, a))
            .unwrap_or_default()
            .into_iter()
            .take(top_n)
            .map(|h| file_risk(&h.file, &index, analytics.as_ref(), &health))
            .collect();
        Ok(self.indexed(RiskOutput { files }, &index, started, cached))
    }

    #[tool(
        name = "get_change_risk",
        description = "Deterministic diff-shape risk score for a single commit or a `base..head` range: lines added/deleted, files touched, subsystems (top-level directories) touched, change concentration (how evenly the diff is spread across files), and the head commit's author's prior-commit count as an experience proxy. These combine into a documented fixed-weight 0-10 score. This is a heuristic approximation of the reference repowise's `get_change_risk`, NOT its ML-calibrated score — this port has no trained model or labeled defect corpus, so treat the number as a rough signal, not a probability."
    )]
    fn get_change_risk(
        &self,
        Parameters(ChangeRiskParams { revspec }): Parameters<ChangeRiskParams>,
    ) -> Result<Json<Envelope<ChangeRiskOutput>>, ErrorData> {
        let started = Instant::now();
        let risk = repowise_git::change_risk(&self.root, revspec.as_deref()).map_err(|e| {
            ErrorData::invalid_params(format!("failed to compute change risk: {e}"), None)
        })?;
        Ok(self.untracked(
            ChangeRiskOutput {
                revspec: risk.revspec,
                lines_added: risk.lines_added,
                lines_deleted: risk.lines_deleted,
                files_touched: risk.files_touched,
                subsystems_touched: risk.subsystems_touched,
                concentration: risk.concentration,
                author: risk.author,
                author_prior_commits: risk.author_prior_commits,
                score: risk.score,
            },
            started,
        ))
    }

    #[tool(
        name = "get_symbol",
        description = "Raw source text for one indexed symbol by id (as returned by `search_codebase`/`get_context`), sliced from the symbol's own file at its `start_line..end_line` span. `context_lines` (default 0) pads that span by the same number of lines on each side, clamped to the file's actual bounds. Re-reads the file fresh from disk rather than trusting the index, so edits since the last `repowise init`/`update` are reflected (the returned span may then be off if line numbers have shifted)."
    )]
    fn get_symbol(
        &self,
        Parameters(GetSymbolParams {
            symbol_id,
            context_lines,
        }): Parameters<GetSymbolParams>,
    ) -> Result<Json<Envelope<GetSymbolOutput>>, ErrorData> {
        let started = Instant::now();
        let (index, _graph, cached) = self.load()?;

        let Some(sym) = index
            .files
            .iter()
            .flat_map(|f| &f.symbols)
            .find(|s| s.id == symbol_id)
        else {
            return Err(ErrorData::resource_not_found(
                format!("no indexed symbol with id {symbol_id}"),
                None,
            ));
        };

        let source = std::fs::read_to_string(&sym.file).map_err(|e| {
            ErrorData::internal_error(format!("failed to read {}: {e}", sym.file.display()), None)
        })?;
        let lines: Vec<&str> = source.lines().collect();

        // Clamp independently to the file's real (freshly re-read) line
        // count, then clamp `start_line` to never exceed `end_line` — the
        // file may have shrunk since this symbol was indexed.
        let end_line = (sym.end_line + context_lines).min(lines.len());
        let start_line = sym
            .start_line
            .saturating_sub(context_lines)
            .clamp(1, end_line.max(1));
        let snippet = lines[(start_line - 1)..end_line].join("\n");

        Ok(self.indexed(
            GetSymbolOutput {
                id: sym.id.clone(),
                name: sym.name.clone(),
                kind: sym.kind.label().to_string(),
                file: display_rel(&sym.file, &index.root),
                start_line,
                end_line,
                source: snippet,
            },
            &index,
            started,
            cached,
        ))
    }

    #[tool(
        name = "get_why",
        description = "Architectural decisions mined from docs/adr/*.md and decision-like commit messages (via repowise-adr), same data as `repowise decisions --for-file`. Given `targets` (file paths or symbol ids), returns only decisions whose body links to at least one target's file. Given no targets (or an empty list), returns every mined decision."
    )]
    fn get_why(
        &self,
        Parameters(WhyParams { targets }): Parameters<WhyParams>,
    ) -> Result<Json<Envelope<WhyOutput>>, ErrorData> {
        let started = Instant::now();
        let (index, _graph, cached) = self.load()?;
        let mut decisions = repowise_adr::mine(&index).map_err(|e| {
            ErrorData::internal_error(format!("failed to mine decisions: {e}"), None)
        })?;

        if !targets.is_empty() {
            let target_files: Vec<PathBuf> = targets
                .iter()
                .map(|t| self.resolve_target(t, &index))
                .collect();
            decisions.retain(|d| d.linked_files.iter().any(|f| target_files.contains(f)));
        }

        let decisions = decisions
            .into_iter()
            .map(|d| {
                let source = match &d.source {
                    repowise_adr::DecisionSource::Adr { file } => {
                        format!("adr:{}", display_rel(file, &index.root))
                    }
                    repowise_adr::DecisionSource::CommitMessage { hash, author } => {
                        format!("commit:{} by {author}", &hash[..hash.len().min(7)])
                    }
                    repowise_adr::DecisionSource::PullRequest { number, author } => {
                        format!("pr:{number} by {author}")
                    }
                    repowise_adr::DecisionSource::CodeComment { file, line } => {
                        format!("comment:{}:{line}", display_rel(file, &index.root))
                    }
                    repowise_adr::DecisionSource::InlineMarker { file, line, marker } => {
                        format!("marker:{marker}:{}:{line}", display_rel(file, &index.root))
                    }
                    repowise_adr::DecisionSource::Changelog { file, section } => {
                        format!("changelog:{section}:{}", display_rel(file, &index.root))
                    }
                };
                DecisionOutput {
                    id: d.id,
                    title: d.title,
                    source,
                    status: d.status,
                    superseded_by: d.superseded_by,
                    date: d.date,
                    linked_files: d
                        .linked_files
                        .iter()
                        .map(|f| display_rel(f, &index.root))
                        .collect(),
                }
            })
            .collect();

        Ok(self.indexed(WhyOutput { decisions }, &index, started, cached))
    }

    #[tool(
        name = "get_dead_code",
        description = "Confidence-tiered dead-code candidates: functions/methods with zero resolved in-repo callers, tiered `low`/`medium`/`high` by how much two cheap risk factors (an ambiguous same-named symbol elsewhere, or an unresolved import that might have targeted this file) undercut that signal — see repowise_health::find_dead_code for the exact logic. `min_confidence` filters to that tier and above; `safe_only` narrows to `high` only, the closest this tool gets to the reference's 'safe to delete' designation. Even `high` confidence is a claim about this port's own static call graph, NOT a runtime-safety guarantee: reflection, dynamic dispatch, and entry points are invisible to it. `limit` caps the returned list (default 50); `total_matching` in the response reports how many matched before truncation."
    )]
    fn get_dead_code(
        &self,
        Parameters(DeadCodeParams {
            min_confidence,
            safe_only,
            limit,
        }): Parameters<DeadCodeParams>,
    ) -> Result<Json<Envelope<DeadCodeOutput>>, ErrorData> {
        let started = Instant::now();
        let (index, graph, cached) = self.load()?;
        let candidates = repowise_health::find_dead_code(&index, &graph);

        let threshold = if safe_only {
            repowise_health::DeadCodeConfidence::High
        } else {
            match min_confidence.as_deref() {
                None => repowise_health::DeadCodeConfidence::Low,
                Some(s) if s.eq_ignore_ascii_case("low") => {
                    repowise_health::DeadCodeConfidence::Low
                }
                Some(s) if s.eq_ignore_ascii_case("medium") => {
                    repowise_health::DeadCodeConfidence::Medium
                }
                Some(s) if s.eq_ignore_ascii_case("high") => {
                    repowise_health::DeadCodeConfidence::High
                }
                Some(other) => {
                    return Err(ErrorData::invalid_params(
                        format!("min_confidence must be low/medium/high, got {other:?}"),
                        None,
                    ));
                }
            }
        };

        let matching: Vec<_> = candidates
            .into_iter()
            .filter(|c| c.confidence >= threshold)
            .collect();
        let total_matching = matching.len();

        let candidates = matching
            .into_iter()
            .take(limit)
            .map(|c| DeadCodeCandidateOutput {
                file: display_rel(&c.file, &index.root),
                symbol: c.symbol,
                line: c.line,
                confidence: c.confidence.label().to_string(),
                risk_factors: c.risk_factors,
            })
            .collect();

        Ok(self.indexed(
            DeadCodeOutput {
                candidates,
                total_matching,
            },
            &index,
            started,
            cached,
        ))
    }

    #[tool(
        name = "list_repos",
        description = "List every repo configured in the workspace file this server was started with (`--workspace <path>`), each with its name, path, and indexed status (file counts if a prior `repowise init`/`update` has run there). Returns an empty list if no --workspace was given."
    )]
    fn list_repos(&self) -> Result<Json<Envelope<ListReposOutput>>, ErrorData> {
        let started = Instant::now();
        let repos = self
            .workspace_repos
            .as_ref()
            .map(|repos| {
                repos
                    .iter()
                    .map(repowise_workspace::repo_status)
                    .map(RepoStatusOutput::from)
                    .collect()
            })
            .unwrap_or_default();
        Ok(self.untracked(ListReposOutput { repos }, started))
    }

    #[tool(
        name = "get_architecture",
        description = "Workspace-wide cross-repo Rust import resolution: which workspace repos depend on which others, and the individual `use` sites behind each dependency. Rust-only (the only language this port anchors to a Cargo.toml-derived crate name); every other language's cross-repo imports are left unresolved. Returns empty lists (not an error) when no --workspace was given, same degrade-gracefully shape as list_repos."
    )]
    fn get_architecture(&self) -> Result<Json<Envelope<ArchitectureOutput>>, ErrorData> {
        let started = Instant::now();
        let Some(repos) = self.workspace_repos.as_ref() else {
            return Ok(self.untracked(
                ArchitectureOutput {
                    repos: Vec::new(),
                    repo_edges: Vec::new(),
                    edges: Vec::new(),
                    total_edges: 0,
                },
                started,
            ));
        };

        let report = repowise_workspace::workspace_architecture(repos);
        let total_edges = report.edges.len();
        let edges = report
            .edges
            .into_iter()
            .take(ARCHITECTURE_EDGES_LIMIT)
            .map(CrossRepoEdgeOutput::from)
            .collect();

        Ok(self.untracked(
            ArchitectureOutput {
                repos: report
                    .repos
                    .into_iter()
                    .map(RepoStatusOutput::from)
                    .collect(),
                repo_edges: report
                    .repo_edges
                    .into_iter()
                    .map(RepoEdgeSummaryOutput::from)
                    .collect(),
                edges,
                total_edges,
            },
            started,
        ))
    }

    #[tool(
        name = "get_blast_radius",
        description = "Direct (one-hop, not transitive -- matching get_context's dependents_of) cross-repo importers of one file in one workspace repo: which OTHER repos' files would need review if this file's public API changed. Requires --workspace; errors if no workspace is configured, the repo name is unknown, or the file isn't indexed there."
    )]
    fn get_blast_radius(
        &self,
        Parameters(BlastRadiusParams { repo, file }): Parameters<BlastRadiusParams>,
    ) -> Result<Json<Envelope<BlastRadiusOutput>>, ErrorData> {
        let started = Instant::now();
        let Some(repos) = self.workspace_repos.as_ref() else {
            return Err(ErrorData::invalid_params(
                "no workspace configured; start the MCP server with --workspace",
                None,
            ));
        };

        let Some(target_repo) = repos.iter().find(|r| r.name == repo) else {
            return Err(ErrorData::resource_not_found(
                format!("no repo named {repo:?} in the configured workspace"),
                None,
            ));
        };

        let target_file = self.resolve_file_in_repo(&file, &target_repo.path);
        let indexed = RepoIndex::load(&target_repo.path)
            .map(|index| index.files.iter().any(|f| f.path == target_file))
            .unwrap_or(false);
        if !indexed {
            return Err(ErrorData::resource_not_found(
                format!("{file} is not an indexed file under repo {repo:?}"),
                None,
            ));
        }

        let importers = repowise_workspace::workspace_blast_radius(repos, &repo, &target_file)
            .into_iter()
            .map(CrossRepoEdgeOutput::from)
            .collect();

        Ok(self.untracked(BlastRadiusOutput { importers }, started))
    }
}

/// One file's risk profile: hotspot/churn/bug-fix data from `analytics`
/// (`None` when git data isn't available, reading as all-zero rather
/// than erroring) plus its health score/findings.
fn file_risk(
    file: &Path,
    index: &RepoIndex,
    analytics: Option<&repowise_git::GitAnalytics>,
    health: &repowise_health::HealthReport,
) -> FileRisk {
    let total_complexity: usize = index
        .files
        .iter()
        .find(|f| f.path == file)
        .map(|f| f.symbols.iter().map(|s| s.complexity).sum())
        .unwrap_or(0);
    let churn = analytics.map(|a| a.churn_of(file)).unwrap_or(0);
    let bugfix_commits = analytics.map(|a| a.bugfix_commits_of(file)).unwrap_or(0);
    let health_score = health
        .file_scores
        .iter()
        .find(|f| f.file == file)
        .map(|f| f.score)
        .unwrap_or(10.0);
    let health_findings = health
        .findings
        .iter()
        .filter(|f| f.file == file)
        .map(|f| HealthFindingOutput {
            kind: f.kind.label().to_string(),
            symbol: f.symbol.clone(),
            line: f.line,
            detail: f.detail.clone(),
        })
        .collect();

    FileRisk {
        file: display_rel(file, &index.root),
        hotspot_score: churn * total_complexity,
        churn,
        bugfix_commits,
        health_score,
        health_findings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repowise_core::{discover_files, FileRecord, Language};
    use rmcp::model::ErrorCode;

    /// Runs the real indexing pipeline (discover + parse) against real
    /// files on disk, then saves the index the tools load from — no
    /// hand-built fixtures standing in for what `repowise init` produces.
    fn build_and_save_index(root: &Path) {
        let discovered = discover_files(root).unwrap();
        let mut files: Vec<FileRecord> = Vec::new();
        let mut other_files = 0;
        for entry in discovered {
            if matches!(entry.language, Language::Other) {
                other_files += 1;
                continue;
            }
            let source = std::fs::read_to_string(&entry.path).unwrap();
            match repowise_parser::parse_file(&entry.path, entry.language, &source).unwrap() {
                Some(record) => files.push(record),
                None => other_files += 1,
            }
        }
        let index = RepoIndex {
            root: root.to_path_buf(),
            files,
            other_files,
            // Unstamped, matching what most of these tests exercise:
            // a plain directory with no git behind it.
            indexed_commit: None,
        };
        index.save(root).unwrap();
    }

    /// Build and save an index stamped with the repo's current HEAD, the
    /// way `repowise-cli`'s init/update does. Needed for anything that
    /// exercises `_meta`'s staleness comparison, which has nothing to
    /// compare without a stamp.
    fn build_and_save_stamped_index(root: &Path) {
        build_and_save_index(root);
        let mut index = RepoIndex::load(root).unwrap();
        index.indexed_commit = repowise_git::head_sha(root);
        index.save(root).unwrap();
    }

    /// The wiring test: `Meta`'s own unit tests cover the decision
    /// table, but nothing there proves the block actually reaches a real
    /// tool response with real values in it.
    #[test]
    fn tool_response_carries_meta_with_the_indexed_commit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git_init(&root);
        std::fs::write(root.join("lib.rs"), "pub fn helper() -> i32 { 1 }\n").unwrap();
        git(&root, &["add", "lib.rs"]);
        git(&root, &["commit", "-q", "-m", "Add lib"]);
        build_and_save_stamped_index(&root);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(envelope) = server.get_overview().unwrap();

        assert_eq!(
            envelope.meta.indexed_commit,
            repowise_git::head_sha(&root),
            "the response should name the commit it was built from"
        );
        assert_eq!(
            envelope.meta.stale_warning, None,
            "an index stamped at HEAD is not stale"
        );
        assert_eq!(envelope.meta.live_head, None, "nothing to report: no drift");
        // The payload must still be reachable and correct -- flattening
        // is what keeps this additive for existing callers.
        assert_eq!(envelope.data.file_count, 1);
    }

    /// The case the whole feature exists for: HEAD moved on, the index
    /// didn't, and the answer must say so instead of reading as current.
    #[test]
    fn tool_response_warns_when_head_has_moved_past_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git_init(&root);
        std::fs::write(root.join("lib.rs"), "pub fn helper() -> i32 { 1 }\n").unwrap();
        git(&root, &["add", "lib.rs"]);
        git(&root, &["commit", "-q", "-m", "Add lib"]);
        build_and_save_stamped_index(&root);
        let indexed = repowise_git::head_sha(&root).unwrap();

        // Move HEAD without re-indexing -- exactly what happens between
        // a `repowise update` and the next few commits.
        std::fs::write(root.join("other.rs"), "pub fn later() {}\n").unwrap();
        git(&root, &["add", "other.rs"]);
        git(&root, &["commit", "-q", "-m", "Add other"]);
        let live = repowise_git::head_sha(&root).unwrap();
        assert_ne!(indexed, live, "the test needs HEAD to have actually moved");

        let server = RepowiseServer::new(root.clone(), None);
        let Json(envelope) = server.get_overview().unwrap();

        assert_eq!(envelope.meta.indexed_commit.as_deref(), Some(&*indexed));
        assert_eq!(envelope.meta.live_head.as_deref(), Some(&*live));
        let warning = envelope
            .meta
            .stale_warning
            .expect("a moved HEAD must produce a warning");
        assert!(warning.contains(&indexed), "{warning}");
        assert!(warning.contains(&live), "{warning}");
    }

    /// `get_change_risk` answers from git alone. Attaching this index's
    /// age to it would make a current answer look doubtful.
    #[test]
    fn git_only_tools_report_timing_without_index_staleness() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git_init(&root);
        std::fs::write(root.join("lib.rs"), "pub fn helper() -> i32 { 1 }\n").unwrap();
        git(&root, &["add", "lib.rs"]);
        git(&root, &["commit", "-q", "-m", "Add lib"]);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(envelope) = server
            .get_change_risk(Parameters(ChangeRiskParams::default()))
            .unwrap();

        assert_eq!(envelope.meta.indexed_commit, None);
        assert_eq!(envelope.meta.index_age_days, None);
        assert_eq!(envelope.meta.stale_warning, None);
    }

    #[test]
    fn get_overview_reports_file_and_symbol_counts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "pub fn helper() -> i32 { 1 }\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data: overview, .. }) = server.get_overview().unwrap();
        assert_eq!(overview.file_count, 1);
        assert_eq!(
            overview
                .symbol_counts
                .iter()
                .find(|c| c.kind == "function")
                .unwrap()
                .count,
            1
        );
    }

    #[test]
    fn search_codebase_finds_symbols_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "pub fn HelperFunc() -> i32 { 1 }\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data: result, .. }) = server
            .search_codebase(Parameters(SearchParams {
                query: "helperfunc".to_string(),
            }))
            .unwrap();
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].name, "HelperFunc");
    }

    #[test]
    fn search_codebase_rejects_empty_query() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let result = server.search_codebase(Parameters(SearchParams {
            query: "  ".to_string(),
        }));
        let Err(err) = result else {
            panic!("expected an error for a blank query");
        };
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn get_context_returns_symbols_deps_and_health_for_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("lib.rs"),
            "mod util;\n\nfn caller() { util::helper(); }\n",
        )
        .unwrap();
        std::fs::write(root.join("util.rs"), "pub fn helper() -> i32 { 1 }\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data: ctx, .. }) = server
            .get_context(Parameters(ContextParams {
                file: "lib.rs".to_string(),
            }))
            .unwrap();
        assert_eq!(ctx.file, "lib.rs");
        assert!(ctx.symbols.iter().any(|s| s.name == "caller"));
        assert_eq!(ctx.dependencies, vec!["util.rs".to_string()]);
        // `caller` has no callers of its own, so it picks up a
        // possibly-dead-code finding (-0.2) — same heuristic the
        // repowise-health tests already establish.
        assert_eq!(ctx.health_score, 9.8);
        assert_eq!(ctx.health_findings.len(), 1);
    }

    #[test]
    fn get_context_errors_on_unindexed_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let result = server.get_context(Parameters(ContextParams {
            file: "missing.rs".to_string(),
        }));
        let Err(err) = result else {
            panic!("expected an error for an unindexed file");
        };
        assert_eq!(err.code, ErrorCode::RESOURCE_NOT_FOUND);
    }

    /// Runs `git`, clearing the sandbox's own commit-identity env vars so
    /// they can't leak into these disposable test repos and override the
    /// local `user.name`/`user.email` set by `git_init`.
    fn git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env_remove("GIT_AUTHOR_NAME")
            .env_remove("GIT_AUTHOR_EMAIL")
            .env_remove("GIT_COMMITTER_NAME")
            .env_remove("GIT_COMMITTER_EMAIL")
            .output()
            .expect("failed to run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_init(dir: &Path) {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.name", "Default Author"]);
        git(dir, &["config", "user.email", "default@example.com"]);
    }

    #[test]
    fn get_risk_for_a_specific_file_reports_hotspot_and_health_data() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git_init(&root);

        std::fs::write(root.join("lib.rs"), "fn caller() { 1; }\n").unwrap();
        git(&root, &["add", "lib.rs"]);
        git(&root, &["commit", "-q", "-m", "Add lib"]);
        std::fs::write(
            root.join("lib.rs"),
            "fn caller() { 1; }\nfn caller2() { 2; }\n",
        )
        .unwrap();
        git(&root, &["commit", "-q", "-am", "Fix a bug in lib"]);

        build_and_save_index(&root);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data: risk, .. }) = server
            .get_risk(Parameters(RiskParams {
                file: Some("lib.rs".to_string()),
                top_n: 10,
            }))
            .unwrap();

        assert_eq!(risk.files.len(), 1);
        let file_risk = &risk.files[0];
        assert_eq!(file_risk.file, "lib.rs");
        assert_eq!(file_risk.churn, 2);
        assert_eq!(file_risk.bugfix_commits, 1);
        // hotspot_score = churn * total_complexity, and both functions
        // contribute complexity 1 each (no branches) -> 2 * 2 = 4.
        assert_eq!(file_risk.hotspot_score, 4);
        // Both functions are uncalled -> 2 possibly-dead-code findings.
        assert_eq!(file_risk.health_findings.len(), 2);
    }

    #[test]
    fn get_risk_without_a_file_returns_top_hotspots_repo_wide() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git_init(&root);

        std::fs::write(root.join("hot.rs"), "fn a() { 1; }\n").unwrap();
        std::fs::write(root.join("cold.rs"), "fn b() { 1; }\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "Add files"]);
        std::fs::write(root.join("hot.rs"), "fn a() { 1; }\nfn a2() { 2; }\n").unwrap();
        git(&root, &["commit", "-q", "-am", "Touch hot.rs again"]);

        build_and_save_index(&root);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data: risk, .. }) = server
            .get_risk(Parameters(RiskParams {
                file: None,
                top_n: 1,
            }))
            .unwrap();

        assert_eq!(risk.files.len(), 1);
        assert_eq!(risk.files[0].file, "hot.rs");
        assert_eq!(risk.files[0].churn, 2);
    }

    #[test]
    fn get_risk_degrades_gracefully_when_not_a_git_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "fn helper() -> i32 { 1 }\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data: risk, .. }) = server
            .get_risk(Parameters(RiskParams {
                file: Some("lib.rs".to_string()),
                top_n: 10,
            }))
            .unwrap();

        assert_eq!(risk.files.len(), 1);
        assert_eq!(risk.files[0].churn, 0);
        assert_eq!(risk.files[0].hotspot_score, 0);
    }

    #[test]
    fn get_risk_errors_on_unindexed_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let result = server.get_risk(Parameters(RiskParams {
            file: Some("missing.rs".to_string()),
            top_n: 10,
        }));
        let Err(err) = result else {
            panic!("expected an error for an unindexed file");
        };
        assert_eq!(err.code, ErrorCode::RESOURCE_NOT_FOUND);
    }

    #[test]
    fn get_change_risk_defaults_to_head_and_reports_diff_shape() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git_init(&root);

        std::fs::write(root.join("lib.rs"), "fn a() {}\n").unwrap();
        git(&root, &["add", "lib.rs"]);
        git(&root, &["commit", "-q", "-m", "Add lib"]);
        std::fs::write(root.join("lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        git(&root, &["commit", "-q", "-am", "Grow lib"]);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data: risk, .. }) = server
            .get_change_risk(Parameters(ChangeRiskParams { revspec: None }))
            .unwrap();

        assert_eq!(risk.revspec, "HEAD");
        assert_eq!(risk.lines_added, 1);
        assert_eq!(risk.lines_deleted, 0);
        assert_eq!(risk.files_touched, 1);
        assert_eq!(risk.author, "default@example.com");
        assert!((0.0..=10.0).contains(&risk.score));
    }

    #[test]
    fn get_change_risk_accepts_an_explicit_range() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git_init(&root);

        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        git(&root, &["add", "a.txt"]);
        git(&root, &["commit", "-q", "-m", "Add a"]);
        git(&root, &["tag", "base"]);
        std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
        git(&root, &["commit", "-q", "-am", "Grow a"]);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data: risk, .. }) = server
            .get_change_risk(Parameters(ChangeRiskParams {
                revspec: Some("base..HEAD".to_string()),
            }))
            .unwrap();

        assert_eq!(risk.revspec, "base..HEAD");
        assert_eq!(risk.lines_added, 1);
        assert_eq!(risk.files_touched, 1);
    }

    #[test]
    fn get_change_risk_errors_when_not_a_git_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let server = RepowiseServer::new(root, None);
        let result = server.get_change_risk(Parameters(ChangeRiskParams { revspec: None }));
        let Err(err) = result else {
            panic!("expected an error when the root isn't a git repository");
        };
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn get_symbol_returns_its_own_line_span_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("lib.rs"),
            "fn before() {}\n\nfn target() {\n    1\n}\n\nfn after() {}\n",
        )
        .unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data: search, .. }) = server
            .search_codebase(Parameters(SearchParams {
                query: "target".to_string(),
            }))
            .unwrap();
        let symbol_id = search.matches[0].id.clone();

        let Json(Envelope { data: sym, .. }) = server
            .get_symbol(Parameters(GetSymbolParams {
                symbol_id,
                context_lines: 0,
            }))
            .unwrap();

        assert_eq!(sym.name, "target");
        assert_eq!(sym.file, "lib.rs");
        assert_eq!(sym.start_line, 3);
        assert_eq!(sym.end_line, 5);
        assert_eq!(sym.source, "fn target() {\n    1\n}");
    }

    #[test]
    fn get_symbol_pads_and_clamps_context_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("lib.rs"),
            "fn before() {}\n\nfn target() {\n    1\n}\n\nfn after() {}\n",
        )
        .unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data: search, .. }) = server
            .search_codebase(Parameters(SearchParams {
                query: "target".to_string(),
            }))
            .unwrap();
        let symbol_id = search.matches[0].id.clone();

        // Requesting far more context than the file has on either side
        // should clamp to the file's real bounds (lines 1..7) rather than
        // panicking or going out of range.
        let Json(Envelope { data: sym, .. }) = server
            .get_symbol(Parameters(GetSymbolParams {
                symbol_id,
                context_lines: 100,
            }))
            .unwrap();

        assert_eq!(sym.start_line, 1);
        assert_eq!(sym.end_line, 7);
        assert_eq!(
            sym.source,
            "fn before() {}\n\nfn target() {\n    1\n}\n\nfn after() {}"
        );
    }

    #[test]
    fn get_symbol_errors_on_an_unknown_id() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let result = server.get_symbol(Parameters(GetSymbolParams {
            symbol_id: "nonexistent".to_string(),
            context_lines: 0,
        }));
        let Err(err) = result else {
            panic!("expected an error for an unknown symbol id");
        };
        assert_eq!(err.code, ErrorCode::RESOURCE_NOT_FOUND);
    }

    /// Two ADRs, each linking to a different indexed file via a mentioned
    /// symbol name (see `repowise_adr::link_to_index`) — no git repo
    /// needed, since ADR-file mining doesn't depend on commit history and
    /// `repowise_adr::mine` degrades commit-mining to empty when the root
    /// isn't a git repository.
    fn build_two_decision_fixture(root: &Path) {
        std::fs::create_dir_all(root.join("docs/adr")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/queue.rs"), "pub struct TaskQueue;\n").unwrap();
        std::fs::write(root.join("src/other.rs"), "pub struct OtherThing;\n").unwrap();
        std::fs::write(
            root.join("docs/adr/0001-queue.md"),
            "# ADR-0001: Use TaskQueue\n\nStatus: Accepted\nDate: 2026-01-01\n\n## Decision\nIntroduce TaskQueue for job scheduling.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("docs/adr/0002-other.md"),
            "# ADR-0002: Use OtherThing\n\nStatus: Accepted\nDate: 2026-02-01\n\n## Decision\nIntroduce OtherThing for config loading.\n",
        )
        .unwrap();
        build_and_save_index(root);
    }

    #[test]
    fn get_why_with_no_targets_returns_every_mined_decision() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_two_decision_fixture(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data: why, .. }) = server
            .get_why(Parameters(WhyParams { targets: vec![] }))
            .unwrap();

        assert_eq!(why.decisions.len(), 2);
    }

    #[test]
    fn get_why_filters_by_file_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_two_decision_fixture(&root);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data: why, .. }) = server
            .get_why(Parameters(WhyParams {
                targets: vec!["src/queue.rs".to_string()],
            }))
            .unwrap();

        assert_eq!(why.decisions.len(), 1);
        assert_eq!(why.decisions[0].title, "Use TaskQueue");
        assert_eq!(why.decisions[0].linked_files, vec!["src/queue.rs"]);
    }

    #[test]
    fn get_why_filters_by_symbol_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_two_decision_fixture(&root);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data: search, .. }) = server
            .search_codebase(Parameters(SearchParams {
                query: "OtherThing".to_string(),
            }))
            .unwrap();
        let symbol_id = search.matches[0].id.clone();

        let Json(Envelope { data: why, .. }) = server
            .get_why(Parameters(WhyParams {
                targets: vec![symbol_id],
            }))
            .unwrap();

        assert_eq!(why.decisions.len(), 1);
        assert_eq!(why.decisions[0].title, "Use OtherThing");
    }

    #[test]
    fn get_why_with_unmatched_target_returns_no_decisions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_two_decision_fixture(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data: why, .. }) = server
            .get_why(Parameters(WhyParams {
                targets: vec!["src/nonexistent.rs".to_string()],
            }))
            .unwrap();

        assert_eq!(why.decisions.len(), 0);
    }

    #[test]
    fn get_dead_code_reports_high_confidence_for_an_uncalled_unambiguous_symbol() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("solo.rs"), "fn solo() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data: dead, .. }) = server
            .get_dead_code(Parameters(DeadCodeParams::default()))
            .unwrap();

        assert_eq!(dead.total_matching, 1);
        assert_eq!(dead.candidates.len(), 1);
        assert_eq!(dead.candidates[0].symbol, "solo");
        assert_eq!(dead.candidates[0].confidence, "high");
        assert!(dead.candidates[0].risk_factors.is_empty());
    }

    #[test]
    fn get_dead_code_safe_only_excludes_ambiguous_name_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("solo.rs"), "fn solo() {}\n").unwrap();
        std::fs::write(root.join("a.rs"), "fn dup() {}\n").unwrap();
        std::fs::write(root.join("b.rs"), "fn dup() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data: all, .. }) = server
            .get_dead_code(Parameters(DeadCodeParams::default()))
            .unwrap();
        assert_eq!(all.total_matching, 3);

        let Json(Envelope { data: safe, .. }) = server
            .get_dead_code(Parameters(DeadCodeParams {
                min_confidence: None,
                safe_only: true,
                limit: 50,
            }))
            .unwrap();
        assert_eq!(safe.total_matching, 1);
        assert_eq!(safe.candidates[0].symbol, "solo");
        assert_eq!(safe.candidates[0].confidence, "high");
    }

    #[test]
    fn get_dead_code_limit_truncates_but_total_matching_reports_the_full_count() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.rs"), "fn dup() {}\n").unwrap();
        std::fs::write(root.join("b.rs"), "fn dup() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data: dead, .. }) = server
            .get_dead_code(Parameters(DeadCodeParams {
                min_confidence: None,
                safe_only: false,
                limit: 1,
            }))
            .unwrap();

        assert_eq!(dead.candidates.len(), 1);
        assert_eq!(dead.total_matching, 2);
    }

    #[test]
    fn get_dead_code_rejects_an_invalid_min_confidence() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let result = server.get_dead_code(Parameters(DeadCodeParams {
            min_confidence: Some("extreme".to_string()),
            safe_only: false,
            limit: 50,
        }));
        let Err(err) = result else {
            panic!("expected an error for an invalid min_confidence");
        };
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn list_repos_returns_empty_list_when_no_workspace_configured() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data: output, .. }) = server.list_repos().unwrap();

        assert!(output.repos.is_empty());
    }

    #[test]
    fn list_repos_reports_indexed_and_unindexed_repos() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let indexed_repo = dir.path().join("indexed");
        let unindexed_repo = dir.path().join("unindexed");
        std::fs::create_dir_all(&indexed_repo).unwrap();
        std::fs::create_dir_all(&unindexed_repo).unwrap();
        build_and_save_index(&indexed_repo);

        let server = RepowiseServer::new(
            root,
            Some(vec![
                repowise_workspace::ResolvedWorkspaceRepo {
                    name: "indexed".to_string(),
                    path: indexed_repo,
                },
                repowise_workspace::ResolvedWorkspaceRepo {
                    name: "unindexed".to_string(),
                    path: unindexed_repo,
                },
            ]),
        );
        let Json(Envelope { data: output, .. }) = server.list_repos().unwrap();

        assert_eq!(output.repos.len(), 2);
        assert_eq!(output.repos[0].name, "indexed");
        assert!(output.repos[0].indexed);
        assert_eq!(output.repos[1].name, "unindexed");
        assert!(!output.repos[1].indexed);
        assert_eq!(output.repos[1].file_count, None);
    }

    fn write_crate(root: &Path, crate_name: &str, files: &[(&str, &str)]) {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"
            ),
        )
        .unwrap();
        for (rel_path, contents) in files {
            std::fs::write(root.join(rel_path), contents).unwrap();
        }
    }

    fn two_repo_workspace(dir: &Path) -> Vec<repowise_workspace::ResolvedWorkspaceRepo> {
        let repo_a = dir.join("repo-a");
        write_crate(
            &repo_a,
            "repo-a",
            &[("src/foo.rs", "pub fn bar() -> i32 { 42 }\n")],
        );
        build_and_save_index(&repo_a);

        let repo_b = dir.join("repo-b");
        write_crate(
            &repo_b,
            "repo-b",
            &[("src/lib.rs", "use repo_a::foo::bar;\n")],
        );
        build_and_save_index(&repo_b);

        vec![
            repowise_workspace::ResolvedWorkspaceRepo {
                name: "repo-a".to_string(),
                path: repo_a,
            },
            repowise_workspace::ResolvedWorkspaceRepo {
                name: "repo-b".to_string(),
                path: repo_b,
            },
        ]
    }

    #[test]
    fn get_architecture_returns_empty_without_a_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data: output, .. }) = server.get_architecture().unwrap();

        assert!(output.repos.is_empty());
        assert!(output.edges.is_empty());
        assert_eq!(output.total_edges, 0);
    }

    #[test]
    fn get_architecture_reports_cross_repo_edges() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let workspace_repos = two_repo_workspace(&root);

        let server = RepowiseServer::new(root.clone(), Some(workspace_repos));
        let Json(Envelope { data: output, .. }) = server.get_architecture().unwrap();

        assert_eq!(output.repos.len(), 2);
        assert_eq!(output.total_edges, 1);
        assert_eq!(output.edges.len(), 1);
        assert_eq!(output.edges[0].from_repo, "repo-b");
        assert_eq!(output.edges[0].to_repo, "repo-a");
        assert_eq!(output.repo_edges.len(), 1);
        assert_eq!(output.repo_edges[0].edge_count, 1);
    }

    #[test]
    fn get_blast_radius_errors_without_a_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let result = server.get_blast_radius(Parameters(BlastRadiusParams {
            repo: "repo-a".to_string(),
            file: "src/foo.rs".to_string(),
        }));
        let Err(err) = result else {
            panic!("expected an error with no workspace configured");
        };
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn get_blast_radius_errors_on_unknown_repo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let workspace_repos = two_repo_workspace(&root);

        let server = RepowiseServer::new(root, Some(workspace_repos));
        let result = server.get_blast_radius(Parameters(BlastRadiusParams {
            repo: "nonexistent".to_string(),
            file: "src/foo.rs".to_string(),
        }));
        let Err(err) = result else {
            panic!("expected an error for an unknown repo name");
        };
        assert_eq!(err.code, ErrorCode::RESOURCE_NOT_FOUND);
    }

    #[test]
    fn get_blast_radius_returns_direct_importers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let workspace_repos = two_repo_workspace(&root);

        let server = RepowiseServer::new(root, Some(workspace_repos));
        let Json(Envelope { data: output, .. }) = server
            .get_blast_radius(Parameters(BlastRadiusParams {
                repo: "repo-a".to_string(),
                file: "src/foo.rs".to_string(),
            }))
            .unwrap();

        assert_eq!(output.importers.len(), 1);
        assert_eq!(output.importers[0].from_repo, "repo-b");
    }

    #[test]
    fn mtime_caching_caches_index_and_graph_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "pub fn helper() -> i32 { 1 }\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);

        // First load populates cache
        let (idx1, _, cached1) = server.load().unwrap();
        assert_eq!(idx1.files.len(), 1);
        assert!(!cached1, "the first load has nothing to hit");

        // Second load hits cache. Asserting the *flag*, not just that
        // the data matches -- equal data proves nothing here, since a
        // full re-read would produce exactly the same index.
        let (idx2, _, cached2) = server.load().unwrap();
        assert_eq!(idx2.files.len(), 1);
        assert_eq!(idx1.files[0].path, idx2.files[0].path);
        assert!(cached2, "the second load should have hit the cache");
    }
}
