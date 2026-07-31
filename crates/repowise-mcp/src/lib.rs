//! An MCP (Model Context Protocol) server exposing this port's index,
//! dependency graph, and health scoring as agent-facing tools over
//! stdio, using the official `rmcp` SDK.
//!
//! Implements `get_overview`, `search_codebase`, `get_context`,
//! `get_risk`, `get_change_risk`, `get_symbol`, `get_why`, `get_answer`,
//! `get_dead_code`, `get_health`, `get_refactor_candidates`,
//! `get_doc_coverage`, `get_coupling`, plus
//! `list_repos`, `get_architecture` and `get_blast_radius` (the last
//! five have no counterpart in the reference) — the ones
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
//! cross-repo import resolution (via
//! `repowise_workspace::workspace_architecture`/`workspace_blast_radius`,
//! themselves built on `repowise_graph::cross_repo_import_edges`).
//! `get_architecture` degrades to empty lists like `list_repos` (no
//! specific target, nothing to error about). `get_blast_radius` takes a
//! specific repo+file target, so it errors like `get_context` instead:
//! no workspace configured, an unknown repo name, or an unindexed
//! file/repo are all reported as errors rather than an empty result.
//! Both cover every language resolved single-repo via a name -> file
//! module map (Rust, Python, Java/Kotlin/Scala, Go, C#, PHP -- see
//! `repowise_graph::cross_repo::MODULE_MAP_LANGUAGES`); every other
//! language's cross-repo imports are left unresolved, deliberately, for
//! a future slice.
//!
//! `search_codebase`'s `repo` parameter (issue #337) is the first slice
//! of federated workspace queries: every prior workspace tool answers a
//! question *about* the workspace as a whole (which repos exist, which
//! import which), but every question-about-one-repo tool (`get_symbol`,
//! `get_context`, `get_risk`, ...) still only ever sees this server's own
//! `root`. `search_codebase` is the first to break that -- `repo` names a
//! specific configured workspace repo to search instead, or `"all"` to
//! federate the same search across every one of them in a single call,
//! each match then labeled with which repo it came from
//! (`RepowiseServer::resolve_search_targets`). Deliberately narrow in two
//! ways: only this one tool (the most naturally federatable, and the
//! surface upstream's own `repo="all"` feature specifically targets),
//! and only the lexical modes (`symbol`/`path`/`hybrid`) -- `semantic`
//! mode's embedding index is tied to this server's own root and isn't
//! federated. Each named/federated repo's index is loaded fresh
//! per call (`RepoIndex::load`+`RepoGraph::build`, no persistent
//! multi-repo-resident cache): this port has no telemetry suggesting the
//! query volume justifies holding N repos in memory at once, and every
//! other MCP tool already re-loads its own index fresh per call the same
//! way -- federating just means doing that N times instead of once.

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

    /// Record a tool response against the files it covered, for
    /// `repowise saved`'s modelled estimate.
    ///
    /// `covered` is the set of files the answer described. Their total
    /// on-disk size is the baseline: what reading them instead would
    /// have cost. That is a **counterfactual** -- the caller might have
    /// read only part of them, or might have read more -- but it is
    /// grounded in real file sizes rather than invented, and every
    /// surface that reports it labels it as modelled.
    ///
    /// Only called for tools whose covered-file set is unambiguous.
    /// `get_overview` and `search_codebase` answer *about* the repo
    /// rather than about a knowable set of files, so there is no
    /// defensible baseline for them and none is recorded -- an
    /// estimate with an arbitrary denominator would be worse than no
    /// estimate.
    ///
    /// Best-effort throughout: accounting must never be able to fail a
    /// tool call.
    fn record_savings(&self, tool: &str, covered: &[PathBuf], response: &impl Serialize) {
        let baseline: u64 = covered
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .sum();
        if baseline == 0 {
            // Nothing readable behind the answer means no baseline to
            // compare against. Recording a zero would drag the reported
            // ratio toward "saved nothing" for reasons unrelated to the
            // tool's behavior.
            return;
        }
        let Ok(body) = serde_json::to_string(response) else {
            return;
        };
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let dir = repowise_distill::store::store_dir(&self.root, home.as_deref());
        repowise_distill::ledger::record_mcp_response(&dir, tool, baseline as usize, body.len());
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

    /// `search_codebase`'s semantic branch.
    ///
    /// Split out because it shares nothing with the substring path but
    /// the response envelope: it embeds the query, ranks files against
    /// the stored index, and reports coverage.
    ///
    /// Unavailability is an **error**, not an empty `semantic_matches`.
    /// An agent reading zero results would conclude the repo has nothing
    /// matching, when the truth is that no search ran — and it would act
    /// on that. The CLI bails for the same reason, so the two surfaces
    /// agree on what "unavailable" looks like.
    fn search_semantic(
        &self,
        index: &RepoIndex,
        query: &str,
        limit: usize,
        started: Instant,
        cached: bool,
    ) -> Result<Json<Envelope<SearchOutput>>, ErrorData> {
        use repowise_llm::embedding_index::{self, Unavailable};

        let config = repowise_llm::LlmConfig::from_env();
        let (hits, coverage) = embedding_index::search(&self.root, index, query, config.as_ref())
            .map_err(|why| {
            let message = why.explain();
            match why {
                // A live endpoint that failed is this server's
                // problem to report, not the caller's to fix by
                // changing an argument.
                Unavailable::EndpointFailed { .. } => ErrorData::internal_error(message, None),
                _ => ErrorData::invalid_params(message, None),
            }
        })?;

        let total = hits.len();
        let limit = limit.clamp(1, SEARCH_MAX_LIMIT);
        let semantic_matches: Vec<SemanticMatch> = hits
            .into_iter()
            .take(limit)
            .map(|hit| SemanticMatch {
                file: display_rel(&hit.file, &index.root),
                similarity: hit.similarity,
            })
            .collect();

        let note = (!coverage.is_complete()).then(|| {
            format!(
                "{} of {} indexed file(s) have embeddings{}. Files without one could not \
                 be ranked and are absent from these results entirely -- their absence is \
                 not evidence they don't match. Run `repowise update` with an embedding \
                 endpoint configured to cover them.",
                coverage.embedded,
                coverage.total,
                coverage
                    .percent()
                    .map(|p| format!(" ({p:.0}%)"))
                    .unwrap_or_default()
            )
        });

        Ok(self.indexed(
            SearchOutput {
                matches: Vec::new(),
                file_matches: Vec::new(),
                semantic_matches,
                semantic_matches_total: Some(total),
                coverage: Some(CoverageOutput {
                    embedded: coverage.embedded,
                    total: coverage.total,
                    note,
                }),
                filters: format!("mode=semantic, limit={limit}"),
            },
            index,
            started,
            cached,
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchParams {
    /// Case-insensitive substring to match.
    query: String,
    /// What to match against: `symbol` (default), `path`, `hybrid`, or
    /// `semantic`. The first three are case-insensitive substring
    /// matches over symbol names and/or paths. `semantic` instead ranks
    /// whole files by embedding similarity against the stored index and
    /// needs an LLM endpoint plus a built index; when either is missing
    /// it errors naming the missing piece rather than degrading to
    /// substring matching, which would answer a different question.
    #[serde(default)]
    mode: Option<String>,
    /// Max results in `semantic` mode only -- default 20, capped at 200.
    /// Ignored by the substring modes, which return every match.
    ///
    /// Semantic search scores *every* embedded file rather than
    /// producing match/no-match, so an unlimited response would be the
    /// whole repo sorted. `semantic_matches_total` reports how many were
    /// ranked, so a truncated list can't be read as the full ranking.
    #[serde(default = "default_search_limit")]
    limit: usize,
    /// Restrict to files of one role: `implementation`, `test`,
    /// `config`, `doc`, or `unknown`. Inferred from path conventions.
    #[serde(default)]
    kind: Option<String>,
    /// Restrict symbol hits to one kind (`function`, `method`,
    /// `struct`, `enum`, `trait`, `class`, `module`, `mixin`).
    #[serde(default)]
    symbol_kind: Option<String>,
    /// Which workspace repo to search. Omit to search this server's own
    /// indexed root only (the default, unchanged from before this
    /// parameter existed) -- a specific repo's name, as configured in
    /// the workspace file this server was started with, to search just
    /// that repo instead; or `"all"` to federate the same search across
    /// every configured workspace repo at once, merging results with
    /// each carrying which repo it came from. Requires `--workspace` for
    /// any value other than omitted; `semantic` mode does not support
    /// this parameter yet (its embedding index is tied to this server's
    /// own root).
    #[serde(default)]
    repo: Option<String>,
}

/// Hand-written rather than derived so `limit` defaults to
/// `SEARCH_DEFAULT_LIMIT` and not to `usize`'s zero, which would mean
/// "return nothing" in the one mode that reads it.
impl Default for SearchParams {
    fn default() -> Self {
        Self {
            query: String::new(),
            mode: None,
            kind: None,
            symbol_kind: None,
            limit: default_search_limit(),
            repo: None,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct ContextParams {
    /// Path to the file, absolute or relative to the indexed root.
    file: String,
    /// Max symbols and max health findings returned. Default 50, capped
    /// at 500. `symbols_total`/`health_findings_total` always report the
    /// true counts, so a truncated answer can't be read as a complete
    /// one.
    #[serde(default = "default_context_limit")]
    limit: usize,
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

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct SymbolMatch {
    /// Stable identifier for this symbol, usable with `get_symbol` to
    /// fetch its raw source text.
    id: String,
    name: String,
    kind: String,
    file: String,
    line: usize,
    /// Which workspace repo this match came from. Present only when
    /// `search_codebase`'s `repo` parameter was given (a named repo or
    /// `"all"`) -- the default (unscoped) search omits it, since every
    /// match already shares one implicit repo and repeating it on every
    /// entry would be noise.
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct FileMatch {
    file: String,
    /// See `SymbolMatch::repo`'s own doc comment -- same presence rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
}

/// One repo to run `search_codebase` against -- see
/// `RepowiseServer::resolve_search_targets`.
struct SearchTarget {
    /// `None` for this server's own indexed root (the unscoped default);
    /// `Some(name)` for a named or `"all"`-federated workspace repo.
    repo: Option<String>,
    root: PathBuf,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct SemanticMatch {
    file: String,
    /// Cosine similarity between the query's embedding and the file's,
    /// in [-1, 1]. A *relative* score: the top result is the best match
    /// among embedded files, which is not the same as it being a good
    /// one. There is no threshold below which a file is excluded, so a
    /// query about something the repo doesn't contain still returns a
    /// ranked list.
    similarity: f32,
}

/// How much of the repo the embedding index covers, reported alongside
/// every semantic result.
///
/// Without it a search over 60% of a repo is indistinguishable from a
/// search over all of it, and the files that lost are indistinguishable
/// from the files that were never in the running.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct CoverageOutput {
    embedded: usize,
    total: usize,
    /// Present only when coverage is partial, spelling out what the
    /// ranking therefore excludes.
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct SearchOutput {
    matches: Vec<SymbolMatch>,
    /// Files whose path matched. Populated in `path`/`hybrid` mode.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    file_matches: Vec<FileMatch>,
    /// Files ranked by embedding similarity, best first. Populated in
    /// `semantic` mode only, and never mixed with the substring hits
    /// above -- they answer different questions and their scores aren't
    /// comparable.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    semantic_matches: Vec<SemanticMatch>,
    /// Files ranked before `limit` truncated the list. `None` outside
    /// semantic mode; `Some` there even when nothing was truncated, so
    /// a short list can be read as complete rather than guessed at.
    #[serde(skip_serializing_if = "Option::is_none")]
    semantic_matches_total: Option<usize>,
    /// Index coverage. `None` outside semantic mode, where it doesn't
    /// apply -- substring search reads the code index, which covers
    /// every file by construction.
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage: Option<CoverageOutput>,
    /// The filters actually applied. Echoed back so an empty result is
    /// readable: "nothing matches" and "your filters excluded
    /// everything" are otherwise indistinguishable.
    filters: String,
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
    /// Symbols in this file before `limit` truncated the list. Lets a
    /// caller tell "this file has 12 symbols" from "you're seeing 50 of
    /// 300".
    symbols_total: usize,
    dependencies: Vec<String>,
    dependents: Vec<String>,
    health_score: f64,
    health_findings: Vec<HealthFindingOutput>,
    /// Findings for this file before truncation.
    health_findings_total: usize,
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
    /// `"adr:<file>"`, `"commit:<short hash> by <author>"`,
    /// `"inferred:<file>:<line> by <model>"`, etc.
    source: String,
    /// True when a **model inferred** this decision from code rather
    /// than reading it from something a person wrote.
    ///
    /// A separate boolean and not just a `source` prefix, because
    /// filtering on a string prefix is the kind of thing a caller
    /// forgets to do. Omitted when false, so its presence is the
    /// signal — every decision without it came from a written artifact.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    inferred: bool,
    /// How much to trust this decision as recorded intent, in `[0, 1]` --
    /// derived from the source alone (an ADR file outranks a freeform
    /// README paragraph), not from how confidently the text itself reads.
    confidence: f64,
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
    /// What the LLM-inferred decision source contributed, and why, if it
    /// contributed nothing.
    ///
    /// Always present. An absent contribution is ambiguous between "this
    /// repo has no inferred decisions" and "the pass that infers them
    /// was never run", and only one of those is a fact about the
    /// codebase.
    inferred_source: String,
    /// Set when any returned decision is model-inferred, telling the
    /// reader what that means for how to weigh it. Absent when every
    /// decision came from a written artifact, so it stays worth reading.
    #[serde(skip_serializing_if = "Option::is_none")]
    inferred_caveat: Option<String>,
}

/// Attached to any `get_why` response containing inferred decisions.
const INFERRED_CAVEAT: &str = "Some decisions here were inferred by a model from code, not read \
from an ADR, commit message, or comment. They are marked `inferred: true` and anchored to code \
the model quoted (a quote that no longer appears in the file drops the decision). Treat them as \
a reading of the code, not as recorded intent.";

/// Default rows for `get_refactor_candidates`.
///
/// Deliberately capped, unlike `repowise-refactor`'s own
/// `find_refactor_candidates` (which returns everything -- capping is
/// this tool's job, not the library's). Running it against this port's
/// own workspace surfaced why: `extract-duplicate` candidates alone
/// numbered in the thousands, mostly structurally-similar test fixtures
/// across many crates. See `repowise-refactor`'s own module doc for the
/// full account.
const REFACTOR_DEFAULT_LIMIT: usize = 20;

/// Hard cap, however large a `limit` is requested.
const REFACTOR_MAX_LIMIT: usize = 100;

fn default_refactor_limit() -> usize {
    REFACTOR_DEFAULT_LIMIT
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RefactorParams {
    /// Restrict to one kind: `break-import-cycle`, `split-god-class`,
    /// `split-by-cohesion`, or `extract-duplicate`.
    #[serde(default)]
    kind: Option<String>,
    /// Max candidates returned. Default 20, capped at 100. Candidates
    /// come back ranked strongest-first (within `extract-duplicate`:
    /// exact matches, then near-duplicates by descending overlap), so a
    /// cap keeps the signal rather than an arbitrary slice.
    #[serde(default = "default_refactor_limit")]
    limit: usize,
}

impl Default for RefactorParams {
    fn default() -> Self {
        RefactorParams {
            kind: None,
            limit: default_refactor_limit(),
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct RefactorCandidateOutput {
    id: String,
    /// `break-import-cycle`, `split-god-class`, `split-by-cohesion`, or
    /// `extract-duplicate`.
    kind: String,
    title: String,
    /// Always traceable to a specific measured number (a method count,
    /// a component count, an overlap ratio) -- never a vague judgment
    /// call, since nothing here is LLM-generated.
    rationale: String,
    /// Repo-relative files this candidate concerns. Never empty.
    files: Vec<String>,
    /// Symbol names involved (class or function names). Empty for
    /// `break-import-cycle`, which is file-scoped rather than
    /// symbol-scoped.
    symbols: Vec<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct RefactorOutput {
    candidates: Vec<RefactorCandidateOutput>,
    /// Total matching the requested `kind` filter, before `limit`
    /// truncated the list -- lets a caller tell "there were only 3"
    /// from "there were 3000 and you're seeing the strongest 20".
    total_matching: usize,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct DocCoverageEntryOutput {
    file: String,
    /// `"missing"` (no wiki page yet), `"fresh"` (the page's embedded
    /// content hash matches the file's current content), or `"stale"`
    /// (the file changed since the page was last generated).
    status: &'static str,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct DocCoverageOutput {
    entries: Vec<DocCoverageEntryOutput>,
    missing: usize,
    fresh: usize,
    stale: usize,
}

/// Default rows for `get_coupling`.
const COUPLING_DEFAULT_LIMIT: usize = 30;

fn default_coupling_limit() -> usize {
    COUPLING_DEFAULT_LIMIT
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CouplingParams {
    /// Max pairs returned. Default 30.
    #[serde(default = "default_coupling_limit")]
    limit: usize,
}

impl Default for CouplingParams {
    fn default() -> Self {
        CouplingParams {
            limit: default_coupling_limit(),
        }
    }
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct CouplingPairOutput {
    file_a: String,
    file_b: String,
    /// Number of commits in the walked history that touched both files.
    count: usize,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct CouplingOutput {
    pairs: Vec<CouplingPairOutput>,
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

/// Default rows in `get_health`'s ranked lists.
const HEALTH_DEFAULT_LIMIT: usize = 20;

/// Hard cap on `get_health`'s ranked lists, matching the reference.
/// A caller asking for 5000 worst files doesn't want them; it wants a
/// ranked list, and an unbounded one is a token bill, not an answer.
const HEALTH_MAX_LIMIT: usize = 50;

fn default_health_limit() -> usize {
    HEALTH_DEFAULT_LIMIT
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct HealthParams {
    /// File paths to score individually. Omit (or pass an empty list)
    /// for repo-wide mode: KPIs plus the lowest-scoring files.
    #[serde(default)]
    targets: Vec<String>,
    /// Max rows in every ranked list. Default 20, capped at 50.
    #[serde(default = "default_health_limit")]
    limit: usize,
}

/// A requested target that produced no score, and why.
///
/// The reason matters more than the fact. "This file isn't in the
/// index" (run `repowise update`) and "there is no such path" are
/// different problems with different fixes, and collapsing them into an
/// absent row makes both look like "healthy, nothing to report".
#[derive(Serialize, schemars::JsonSchema)]
struct UnresolvedTarget {
    target: String,
    /// `not_indexed` — the path exists on disk but isn't in the index.
    /// `no_such_path` — nothing on disk matches it.
    reason: &'static str,
    hint: &'static str,
}

#[derive(Serialize, schemars::JsonSchema)]
struct FileHealthOutput {
    file: String,
    /// 0.0 (unhealthy) to 10.0 (no markers triggered).
    score: f64,
    lines: usize,
    findings: Vec<HealthFindingOutput>,
}

#[derive(Serialize, schemars::JsonSchema)]
struct FindingKindCount {
    kind: String,
    count: usize,
}

#[derive(Serialize, schemars::JsonSchema)]
struct HealthOutput {
    /// `"repo"` (no targets given) or `"targeted"`.
    mode: &'static str,
    /// Line-count-weighted mean score across every indexed file.
    /// `None` when there are no indexed files to average.
    #[serde(skip_serializing_if = "Option::is_none")]
    average_score: Option<f64>,
    /// Plain per-file mean, unweighted. Reported alongside
    /// `average_score` because the gap between them is the signal: when
    /// the weighted number is materially lower, the problem is in big
    /// files, not the long tail.
    #[serde(skip_serializing_if = "Option::is_none")]
    average_score_unweighted: Option<f64>,
    /// Names what `average_score` is weighted by, so the two averages
    /// above can't be mistaken for each other.
    #[serde(skip_serializing_if = "Option::is_none")]
    average_score_weighting: Option<&'static str>,
    /// Repo mode: the worst files, capped by `limit`. Targeted mode:
    /// one entry per resolved target, worst first.
    files: Vec<FileHealthOutput>,
    /// How many files the list above was drawn from, before `limit`.
    /// Lets a caller tell "there are only 3" from "there are 300 and
    /// you're seeing 20".
    files_total: usize,
    /// Repo-wide finding counts by marker kind. Present in repo mode.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    findings_by_kind: Vec<FindingKindCount>,
    /// Targets that produced no score, each with a reason. Empty in
    /// repo mode. An empty `files` list with entries here means "we
    /// couldn't look", not "everything is fine".
    #[serde(skip_serializing_if = "Vec::is_empty")]
    unresolved: Vec<UnresolvedTarget>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct AnswerParams {
    /// A natural-language question about the codebase.
    question: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct AnswerOutput {
    /// `false` when no LLM endpoint is configured. `answer` is absent,
    /// and the reason says so -- an unconfigured feature must not return
    /// an empty-but-confident answer.
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    answer: Option<String>,
    /// Why no answer, when `available` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    unavailable_reason: Option<String>,
    /// Repo-relative files the answer drew on, best first.
    ///
    /// **Empty means the answer is ungrounded.** Retrieval found nothing
    /// to cite, so whatever the model said came from its own priors
    /// rather than from this codebase -- exactly the case a caller most
    /// needs to distinguish, and the one that reads most confidently.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cited: Vec<String>,
    /// `semantic` or `keyword`.
    retrieval_mode: String,
    /// Present only when retrieval degraded to keyword matching.
    #[serde(skip_serializing_if = "Option::is_none")]
    retrieval_caveat: Option<String>,
    /// How many files' vectors came from the persisted embedding index
    /// vs. were embedded fresh for this call, in `semantic` mode only.
    ///
    /// A performance fact, not a caveat: whatever the stored index
    /// doesn't cover is embedded on the spot, so coverage at answer time
    /// is always complete regardless of these numbers. Absent for
    /// `keyword` mode, where no vectors are involved at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    vectors_reused: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vectors_embedded_now: Option<usize>,
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

/// Default cap on `get_context`'s symbol and finding lists.
///
/// 50, matching `get_dead_code`'s shape. Chosen from measurement, not
/// taste: an uncapped `get_context` on a 300-symbol file returned
/// **120 KB for an 8.5 KB file** -- fourteen times the cost of simply
/// reading it, which makes the tool actively worse than the thing it
/// replaces.
const CONTEXT_DEFAULT_LIMIT: usize = 50;

/// Hard cap, however large a `limit` is requested.
const CONTEXT_MAX_LIMIT: usize = 500;

fn default_context_limit() -> usize {
    CONTEXT_DEFAULT_LIMIT
}

/// Default top-N for `search_codebase`'s semantic mode.
///
/// Smaller than the context limit on purpose: semantic mode scores every
/// embedded file, so the tail is noise sorted by a number, not results.
const SEARCH_DEFAULT_LIMIT: usize = 20;

/// Hard cap, however large a `limit` is requested.
const SEARCH_MAX_LIMIT: usize = 200;

fn default_search_limit() -> usize {
    SEARCH_DEFAULT_LIMIT
}

/// A symbol id with the repo-relative path, rather than the absolute one
/// `Symbol::make_id` bakes in.
///
/// Two problems with the stored form, both fixed here:
///
/// - It leaks the producing machine's directory layout to every caller.
///   `repowise-graph`'s JGF export already rebuilds ids for exactly this
///   reason; the MCP surface should not be the one place that still
///   emits them.
/// - It is *enormous* in aggregate. On a 300-symbol file the absolute
///   prefix accounted for 34882 of the response's bytes -- 59% of the
///   symbol list -- repeating the same path 300 times.
fn portable_symbol_id(sym: &repowise_core::Symbol, root: &Path) -> String {
    format!(
        "{}::{}@{}",
        display_rel(&sym.file, root),
        sym.name,
        sym.start_line
    )
}

/// Find a symbol by either id form.
///
/// Accepts the stored absolute id and the portable relative one, so ids
/// handed out by `get_context` before and after this change both keep
/// working. Matching on the portable form is the primary path; the
/// absolute comparison is the compatibility tail.
fn find_symbol<'a>(index: &'a RepoIndex, wanted: &str) -> Option<&'a repowise_core::Symbol> {
    index
        .files
        .iter()
        .flat_map(|f| &f.symbols)
        .find(|s| s.id == wanted || portable_symbol_id(s, &index.root) == wanted)
}

/// Line-count-weighted mean health score.
///
/// Weighted rather than a plain file mean because a repo's health is
/// dominated by where its code actually is: a thousand tiny perfect
/// files shouldn't wash out one enormous bad one. Reported alongside
/// the unweighted mean, since the *gap* between them is the actionable
/// part — when weighted is materially lower, the problem is in big
/// files.
///
/// `None` when there is nothing to average. Files with zero lines
/// contribute zero weight, so a repo of only empty files falls back to
/// the unweighted mean rather than dividing by zero.
fn weighted_average_score(
    report: &repowise_health::HealthReport,
    index: &RepoIndex,
) -> Option<f64> {
    if report.file_scores.is_empty() {
        return None;
    }
    let mut weighted = 0.0f64;
    let mut total_lines = 0usize;
    for fh in &report.file_scores {
        let lines = index
            .files
            .iter()
            .find(|f| f.path == fh.file)
            .map(|f| f.lines)
            .unwrap_or(0);
        weighted += fh.score * lines as f64;
        total_lines += lines;
    }
    if total_lines == 0 {
        return Some(report.average_score);
    }
    Some(weighted / total_lines as f64)
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
        description = "Search the index four ways, chosen by `mode`. `symbol` (default), `path`, and `hybrid` are case-insensitive substring matches over indexed symbol names and/or file paths, returning each match's kind, file, and line number — cheap, exact, and always available. `semantic` instead ranks whole files by embedding similarity to the query, for questions phrased by meaning rather than by name (\"where is retry logic handled\"); it requires REPOWISE_LLM_BASE_URL and an embedding index built by `repowise init`/`update`, and errors naming whichever is missing rather than falling back to substring matching. Semantic results carry index coverage, since a ranking over part of a repo must not read as a ranking over all of it. The three lexical modes accept a `repo` parameter: a workspace repo's name to search just that repo, or `\"all\"` to federate the search across every configured workspace repo at once (each match then carries which repo it came from) -- semantic mode does not support it yet."
    )]
    fn search_codebase(
        &self,
        Parameters(SearchParams {
            query,
            mode,
            kind,
            symbol_kind,
            limit,
            repo,
        }): Parameters<SearchParams>,
    ) -> Result<Json<Envelope<SearchOutput>>, ErrorData> {
        let started = Instant::now();
        if query.trim().is_empty() {
            return Err(ErrorData::invalid_params("query must not be empty", None));
        }
        let mode = repowise_graph::SearchMode::parse(mode.as_deref().unwrap_or("symbol"))
            .map_err(|e| ErrorData::invalid_params(e, None))?;
        let kind = kind
            .as_deref()
            .map(repowise_graph::FileKind::parse)
            .transpose()
            .map_err(|e| ErrorData::invalid_params(e, None))?;
        let symbol_kind = symbol_kind
            .as_deref()
            .map(repowise_graph::parse_symbol_kind)
            .transpose()
            .map_err(|e| ErrorData::invalid_params(e, None))?;

        // Semantic shares none of the filtering/scoping below: it ranks
        // files, not symbols, so `kind`/`symbol_kind` have nothing to
        // act on and are rejected rather than silently ignored -- a
        // filter that looks applied but isn't is how a caller ends up
        // trusting a result that didn't honour their constraint. `repo`
        // is rejected too: its embedding index is tied to this server's
        // own indexed root (see `search_semantic`), and federating that
        // is real added complexity this first slice of issue #337 didn't
        // take on -- the lexical modes below are where federation lives.
        if !mode.is_lexical() {
            if repo.is_some() {
                return Err(ErrorData::invalid_params(
                    "semantic mode does not support the `repo` parameter yet -- its \
                     embedding index is tied to this server's own indexed root. Omit \
                     `repo`, or use symbol/path/hybrid mode instead.",
                    None,
                ));
            }
            if kind.is_some() || symbol_kind.is_some() {
                return Err(ErrorData::invalid_params(
                    "semantic mode ranks whole files, so `kind` and `symbol_kind` don't \
                     apply to it. They are rejected rather than ignored, so a result can't \
                     look filtered when it isn't. Use `hybrid` if you need those filters.",
                    None,
                ));
            }
            let (index, _graph, cached) = self.load()?;
            return self.search_semantic(&index, &query, limit, started, cached);
        }

        let targets = self.resolve_search_targets(repo.as_deref())?;

        // Only the single implicit-self target (no `repo` given at all)
        // reuses this server's own `_meta` provenance -- a named or
        // federated search answers from other repos' indexes too, the
        // same "doesn't apply" reasoning `list_repos`/`get_architecture`/
        // `get_blast_radius` already use for `Meta::timing_only`.
        let mut self_meta: Option<(RepoIndex, bool)> = None;
        let mut matches: Vec<SymbolMatch> = Vec::new();
        let mut file_matches: Vec<FileMatch> = Vec::new();

        for target in &targets {
            let (index, graph, cached) = if target.repo.is_none() {
                self.load()?
            } else {
                let index = RepoIndex::load(&target.root).map_err(|e| {
                    ErrorData::internal_error(
                        format!("failed to load index at {}: {e}", target.root.display()),
                        None,
                    )
                })?;
                let graph = RepoGraph::build(&index);
                (index, graph, false)
            };

            let file_allowed = |file: &Path| -> bool {
                let Some(want) = kind else { return true };
                index
                    .files
                    .iter()
                    .find(|f| f.path == file)
                    .map(|f| repowise_graph::classify(f, &index.root) == want)
                    .unwrap_or(false)
            };

            if matches!(
                mode,
                repowise_graph::SearchMode::Symbol | repowise_graph::SearchMode::Hybrid
            ) {
                matches.extend(
                    graph
                        .search(&query)
                        .into_iter()
                        .filter(|s| symbol_kind.is_none_or(|k| s.kind == k))
                        .filter(|s| file_allowed(&s.file))
                        .map(|sym| SymbolMatch {
                            id: sym.id.clone(),
                            name: sym.name.clone(),
                            kind: sym.kind.label().to_string(),
                            file: display_rel(&sym.file, &index.root),
                            line: sym.start_line,
                            repo: target.repo.clone(),
                        }),
                );
            }

            if matches!(
                mode,
                repowise_graph::SearchMode::Path | repowise_graph::SearchMode::Hybrid
            ) {
                file_matches.extend(
                    index
                        .files
                        .iter()
                        .filter(|f| repowise_graph::path_matches(&f.path, &index.root, &query))
                        .filter(|f| {
                            kind.is_none_or(|k| repowise_graph::classify(f, &index.root) == k)
                        })
                        .map(|f| FileMatch {
                            file: display_rel(&f.path, &index.root),
                            repo: target.repo.clone(),
                        }),
                );
            }

            if target.repo.is_none() {
                self_meta = Some((index, cached));
            }
        }

        matches.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then(a.file.cmp(&b.file))
                .then(a.repo.cmp(&b.repo))
        });
        file_matches.sort_by(|a, b| a.file.cmp(&b.file).then(a.repo.cmp(&b.repo)));

        let mut filters = vec![format!("mode={}", mode.label())];
        if let Some(k) = kind {
            filters.push(format!("kind={}", k.label()));
        }
        if let Some(k) = symbol_kind {
            filters.push(format!("symbol_kind={}", k.label()));
        }
        if let Some(r) = &repo {
            filters.push(format!("repo={r}"));
        }

        let output = SearchOutput {
            matches,
            file_matches,
            semantic_matches: Vec::new(),
            semantic_matches_total: None,
            coverage: None,
            filters: filters.join(", "),
        };

        Ok(match self_meta {
            Some((index, cached)) => self.indexed(output, &index, started, cached),
            None => self.untracked(output, started),
        })
    }

    /// Resolve `search_codebase`'s `repo` parameter into the list of
    /// repo roots to search: `None` means just this server's own indexed
    /// root (one target, `repo: None` so its matches stay unlabeled,
    /// exactly the pre-#337 behavior); `Some("all")` federates across
    /// every configured workspace repo; `Some(name)` searches just that
    /// one named repo. The latter two both require `--workspace` and
    /// error clearly when it's missing, rather than silently falling
    /// back to searching only this server's own root -- that would look
    /// like an answer to "search my whole workspace" while quietly
    /// answering a much narrower question.
    fn resolve_search_targets(&self, repo: Option<&str>) -> Result<Vec<SearchTarget>, ErrorData> {
        let Some(repo) = repo else {
            return Ok(vec![SearchTarget {
                repo: None,
                root: self.root.clone(),
            }]);
        };
        let Some(repos) = self.workspace_repos.as_ref() else {
            return Err(ErrorData::invalid_params(
                format!(
                    "repo={repo:?} requires a workspace; start the MCP server with --workspace"
                ),
                None,
            ));
        };
        if repo == "all" {
            return Ok(repos
                .iter()
                .map(|r| SearchTarget {
                    repo: Some(r.name.clone()),
                    root: r.path.clone(),
                })
                .collect());
        }
        let Some(target) = repos.iter().find(|r| r.name == repo) else {
            return Err(ErrorData::resource_not_found(
                format!("no repo named {repo:?} in the configured workspace"),
                None,
            ));
        };
        Ok(vec![SearchTarget {
            repo: Some(target.name.clone()),
            root: target.path.clone(),
        }])
    }

    #[tool(
        name = "get_context",
        description = "Complete context for one file in a single call: its symbols, resolved dependencies/dependents, and health findings/score. Built to replace the several separate reads (search, deps, health) an agent would otherwise need to piece this together itself."
    )]
    fn get_context(
        &self,
        Parameters(ContextParams { file, limit }): Parameters<ContextParams>,
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

        let limit = limit.clamp(1, CONTEXT_MAX_LIMIT);

        let mut all_symbols: Vec<&repowise_core::Symbol> = record
            .symbols
            .iter()
            .filter(|s| !matches!(s.kind, SymbolKind::Module))
            .collect();
        all_symbols.sort_by_key(|s| s.start_line);
        let symbols_total = all_symbols.len();
        let symbols: Vec<SymbolMatch> = all_symbols
            .into_iter()
            .take(limit)
            .map(|sym| SymbolMatch {
                // Portable, and far smaller: the stored id embeds this
                // machine's absolute path, which on a dense file was 59%
                // of the whole symbol list.
                id: portable_symbol_id(sym, &index.root),
                name: sym.name.clone(),
                kind: sym.kind.label().to_string(),
                file: display_rel(&sym.file, &index.root),
                line: sym.start_line,
                // `get_context` is scoped to this server's own root only
                // -- no `repo` parameter, so no repo label to attach.
                repo: None,
            })
            .collect();

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
        let matching_findings: Vec<_> = health
            .findings
            .iter()
            .filter(|f| f.file == target)
            .collect();
        let health_findings_total = matching_findings.len();
        let health_findings: Vec<HealthFindingOutput> = matching_findings
            .into_iter()
            .take(limit)
            .map(|f| HealthFindingOutput {
                kind: f.kind.label().to_string(),
                symbol: f.symbol.clone(),
                line: f.line,
                detail: f.detail.clone(),
            })
            .collect();

        let output = ContextOutput {
            file: display_rel(&target, &index.root),
            symbols,
            symbols_total,
            dependencies,
            dependents,
            health_score: file_health,
            health_findings,
            health_findings_total,
        };
        // The baseline: reading this file instead of asking for its
        // context. One file, unambiguous.
        self.record_savings("get_context", std::slice::from_ref(&target), &output);
        Ok(self.indexed(output, &index, started, cached))
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
        // failing the whole call, same tradeoff `repowise-server`'s
        // `/api/hotspots` endpoint already makes.
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

        let Some(sym) = find_symbol(&index, &symbol_id) else {
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

        let file = sym.file.clone();
        let output = GetSymbolOutput {
            // The same portable form `get_context` hands out, so an id
            // taken from one tool and passed to the other round-trips.
            id: portable_symbol_id(sym, &index.root),
            name: sym.name.clone(),
            kind: sym.kind.label().to_string(),
            file: display_rel(&sym.file, &index.root),
            start_line,
            end_line,
            source: snippet,
        };
        // The baseline: reading the whole file to get at one symbol.
        self.record_savings("get_symbol", std::slice::from_ref(&file), &output);
        Ok(self.indexed(output, &index, started, cached))
    }

    #[tool(
        name = "get_why",
        description = "Architectural decisions mined from docs/adr/*.md, decision-like commit messages, merged PR bodies, code comments, inline WHY:/DECISION: markers, CHANGELOG sections, and decision-flavored README/ARCHITECTURE prose (via repowise-adr), same data as `repowise decisions --for-file`. Given `targets` (file paths or symbol ids), returns only decisions whose body links to at least one target's file. Given no targets (or an empty list), returns every mined decision. Each decision carries a `confidence` in [0, 1], derived from how trustworthy its source is as a record of actual intent (an ADR file or a `repowise decide` record outranks a freeform README paragraph or code comment) -- weigh a low-confidence hit more skeptically. One source is NOT a written artifact: decisions marked `inferred: true` were inferred by a model from code during `repowise generate`, anchored to a verbatim quote that is re-checked against the file on every read. `inferred_source` always reports what that source contributed, so an absent contribution can't be mistaken for a repo with nothing to infer."
    )]
    fn get_why(
        &self,
        Parameters(WhyParams { targets }): Parameters<WhyParams>,
    ) -> Result<Json<Envelope<WhyOutput>>, ErrorData> {
        let started = Instant::now();
        let (index, _graph, cached) = self.load()?;
        let (mut decisions, inferred_state) =
            repowise_adr::mine_reporting(&index).map_err(|e| {
                ErrorData::internal_error(format!("failed to mine decisions: {e}"), None)
            })?;

        if !targets.is_empty() {
            let target_files: Vec<PathBuf> = targets
                .iter()
                .map(|t| self.resolve_target(t, &index))
                .collect();
            decisions.retain(|d| d.linked_files.iter().any(|f| target_files.contains(f)));
        }

        // Computed after the target filter, so the caveat describes what
        // was actually returned rather than what the repo happens to
        // hold. A response with no inferred decisions in it shouldn't
        // carry a warning about them.
        let any_inferred = decisions.iter().any(|d| d.source.is_inferred());

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
                    repowise_adr::DecisionSource::ReadmeMining {
                        file,
                        line,
                        heading,
                    } => {
                        format!(
                            "readme:{}:{line} ({heading:?})",
                            display_rel(file, &index.root)
                        )
                    }
                    repowise_adr::DecisionSource::Inferred { file, line, model } => {
                        format!(
                            "inferred:{}:{line} by {model}",
                            display_rel(file, &index.root)
                        )
                    }
                    repowise_adr::DecisionSource::Manual { recorded_at } => {
                        format!("manual:{recorded_at}")
                    }
                };
                let inferred = d.source.is_inferred();
                DecisionOutput {
                    id: d.id,
                    title: d.title,
                    source,
                    inferred,
                    confidence: d.confidence,
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

        Ok(self.indexed(
            WhyOutput {
                decisions,
                inferred_source: inferred_state.describe(),
                inferred_caveat: any_inferred.then(|| INFERRED_CAVEAT.to_string()),
            },
            &index,
            started,
            cached,
        ))
    }

    #[tool(
        name = "get_answer",
        description = "Answer a natural-language question about this codebase, with citations. Retrieves relevant files by embedding similarity, then answers from them. Requires an LLM endpoint (REPOWISE_LLM_BASE_URL); reports `available: false` with a reason when unconfigured rather than guessing. Reuses vectors from the persisted embedding index (built by `init`/`update`) and embeds only the files it doesn't cover, in one batched call alongside the question -- coverage is always complete regardless of how much of the index that call had to fill in, so there's a `vectors_reused`/`vectors_embedded_now` split (semantic mode only) but no caveat to go with it. Still a considered-question tool rather than a cheap lookup -- use search_codebase for finding things by name."
    )]
    fn get_answer(
        &self,
        Parameters(AnswerParams { question }): Parameters<AnswerParams>,
    ) -> Result<Json<Envelope<AnswerOutput>>, ErrorData> {
        let started = Instant::now();
        if question.trim().is_empty() {
            return Err(ErrorData::invalid_params(
                "question must not be empty",
                None,
            ));
        }
        let (index, _graph, cached) = self.load()?;

        // Unconfigured is a real, reportable state -- not an error, and
        // certainly not an empty answer. An agent needs to distinguish
        // "this feature is off" from "the codebase has nothing to say".
        let Some(config) = repowise_llm::LlmConfig::from_env() else {
            return Ok(self.indexed(
                AnswerOutput {
                    available: false,
                    answer: None,
                    unavailable_reason: Some(
                        "no LLM endpoint configured -- set REPOWISE_LLM_BASE_URL (and \
                         REPOWISE_LLM_MODEL / REPOWISE_LLM_API_KEY as needed). Every \
                         other tool on this server works without it."
                            .to_string(),
                    ),
                    cited: Vec::new(),
                    retrieval_mode: String::new(),
                    retrieval_caveat: None,
                    vectors_reused: None,
                    vectors_embedded_now: None,
                },
                &index,
                started,
                cached,
            ));
        };

        let retrieval = repowise_llm::retrieve(&self.root, &index, &question, &config);
        let turns = vec![
            repowise_llm::Turn::system(retrieval.context.clone()),
            repowise_llm::Turn::user(question),
        ];
        let answer = repowise_llm::complete_messages(&config, &turns)
            .map_err(|e| ErrorData::internal_error(format!("the LLM request failed: {e}"), None))?;

        Ok(self.indexed(
            AnswerOutput {
                available: true,
                answer: Some(answer),
                unavailable_reason: None,
                cited: retrieval.cited,
                retrieval_mode: retrieval.mode.label().to_string(),
                retrieval_caveat: retrieval.mode.caveat().map(str::to_string),
                vectors_reused: retrieval.vectors.map(|v| v.reused),
                vectors_embedded_now: retrieval.vectors.map(|v| v.embedded_now),
            },
            &index,
            started,
            cached,
        ))
    }

    #[tool(
        name = "get_health",
        description = "Deterministic code-health marker scores, including six organizational-signal markers derived from git history: prior-defect, churn-risk, knowledge-loss (low bus factor), co-change-scatter, hidden-coupling (files that co-change often but share no import/call edge), and developer-congestion. With no `targets`, returns repo-wide KPIs and the lowest-scoring files -- use this to find what to fix first. With `targets`, returns each file's score and its individual findings. Requires a prior `repowise init`/`update`. NOTE: the organizational-signal markers need one `git blame` per indexed file on top of a history walk -- measured at several seconds on this port's own workspace, so this is a full-report call, not a cheap lookup. They're silently skipped (not reported as zero risk) if the root isn't a git repository."
    )]
    fn get_health(
        &self,
        Parameters(HealthParams { targets, limit }): Parameters<HealthParams>,
    ) -> Result<Json<Envelope<HealthOutput>>, ErrorData> {
        let started = Instant::now();
        let (index, graph, cached) = self.load()?;
        // One `GitAnalytics::collect` walk feeds the organizational
        // signals; a repo with no git history (or that isn't a git repo
        // at all) simply skips those six markers, the same degrade-
        // gracefully convention `coverage`/`hot_files` already use here.
        let analytics = repowise_git::GitAnalytics::collect(&self.root).ok();
        let org_signals = analytics.as_ref().and_then(|a| {
            repowise_git::org_signals::collect_org_signals(&self.root, &index, a).ok()
        });
        let report = repowise_health::analyze_with_context(
            &index,
            &graph,
            &repowise_health::HealthWeights::default(),
            &std::collections::HashSet::new(),
            None,
            org_signals.as_ref(),
        );
        let limit = limit.clamp(1, HEALTH_MAX_LIMIT);

        let render = |fh: &repowise_health::FileHealth| FileHealthOutput {
            file: display_rel(&fh.file, &index.root),
            score: fh.score,
            lines: index
                .files
                .iter()
                .find(|f| f.path == fh.file)
                .map(|f| f.lines)
                .unwrap_or(0),
            findings: report
                .findings
                .iter()
                .filter(|f| f.file == fh.file)
                .map(|f| HealthFindingOutput {
                    kind: f.kind.label().to_string(),
                    symbol: f.symbol.clone(),
                    line: f.line,
                    detail: f.detail.clone(),
                })
                .collect(),
        };

        if targets.is_empty() {
            // `analyze` returns file_scores sorted worst-first already.
            let files: Vec<_> = report.file_scores.iter().take(limit).map(render).collect();
            return Ok(self.indexed(
                HealthOutput {
                    mode: "repo",
                    average_score: weighted_average_score(&report, &index),
                    average_score_unweighted: (!report.file_scores.is_empty())
                        .then_some(report.average_score),
                    average_score_weighting: (!report.file_scores.is_empty()).then_some("lines"),
                    files,
                    files_total: report.file_scores.len(),
                    findings_by_kind: report
                        .findings_by_kind()
                        .into_iter()
                        .map(|(kind, count)| FindingKindCount {
                            kind: kind.label().to_string(),
                            count,
                        })
                        .collect(),
                    unresolved: Vec::new(),
                },
                &index,
                started,
                cached,
            ));
        }

        let mut files = Vec::new();
        let mut unresolved = Vec::new();
        for target in &targets {
            let resolved = self.resolve_file(target);
            match report.file_scores.iter().find(|fh| fh.file == resolved) {
                Some(fh) => files.push(render(fh)),
                // Distinguishing these two is the point: one is fixed by
                // re-indexing, the other by correcting the path. An
                // absent row would look like neither.
                None if resolved.exists() => unresolved.push(UnresolvedTarget {
                    target: target.clone(),
                    reason: "not_indexed",
                    hint: "the path exists but isn't in the index -- run `repowise update`",
                }),
                None => unresolved.push(UnresolvedTarget {
                    target: target.clone(),
                    reason: "no_such_path",
                    hint: "nothing on disk matches this path",
                }),
            }
        }
        files.sort_by(|a, b| a.score.total_cmp(&b.score));
        let files_total = files.len();

        Ok(self.indexed(
            HealthOutput {
                mode: "targeted",
                // Repo-wide averages would be a non-sequitur next to a
                // two-file answer, and worse, could be read as those
                // files' average.
                average_score: None,
                average_score_unweighted: None,
                average_score_weighting: None,
                files,
                files_total,
                findings_by_kind: Vec::new(),
                unresolved,
            },
            &index,
            started,
            cached,
        ))
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
        name = "get_refactor_candidates",
        description = "Deterministic refactor candidates: file-level import cycles, god classes (too many methods), low-cohesion classes (methods split into disjoint field-access groups), and duplicate/near-duplicate functions -- every signal already computed by repowise-graph/repowise-health, synthesized here as named, located problems. Read-only: this names a problem and where it is, and never generates a diff or writes to a file -- this port doesn't have a refactoring-diff feature (see issue #304). `kind` restricts to one category. `limit` (default 20, max 100) caps the list; `total_matching` reports the true count before truncation, since extract-duplicate candidates alone can number in the thousands on a large codebase -- candidates are ranked strongest-first (exact duplicates before near-duplicates, near-duplicates by descending overlap) so a cap keeps the signal. NOTE: near-duplicate detection is the same cost as `repowise health`'s equivalent marker -- on a real multi-crate codebase this can take several seconds (not a cheap lookup), since the full computation runs before `limit` truncates the response."
    )]
    fn get_refactor_candidates(
        &self,
        Parameters(RefactorParams { kind, limit }): Parameters<RefactorParams>,
    ) -> Result<Json<Envelope<RefactorOutput>>, ErrorData> {
        let started = Instant::now();
        let (index, graph, cached) = self.load()?;

        let kind_filter = kind
            .as_deref()
            .map(|k| match k {
                "break-import-cycle" | "split-god-class" | "split-by-cohesion"
                | "extract-duplicate" => Ok(k.to_string()),
                other => Err(ErrorData::invalid_params(
                    format!(
                        "unknown kind {other:?} -- expected break-import-cycle, \
                         split-god-class, split-by-cohesion, or extract-duplicate"
                    ),
                    None,
                )),
            })
            .transpose()?;

        let mut candidates = repowise_refactor::find_refactor_candidates(&index, &graph);
        if let Some(k) = &kind_filter {
            candidates.retain(|c| c.kind.label() == k);
        }
        let total_matching = candidates.len();
        let limit = limit.clamp(1, REFACTOR_MAX_LIMIT);

        let candidates = candidates
            .into_iter()
            .take(limit)
            .map(|c| RefactorCandidateOutput {
                id: c.id,
                kind: c.kind.label().to_string(),
                title: c.title,
                rationale: c.rationale,
                files: c.files,
                symbols: c.symbols,
            })
            .collect();

        Ok(self.indexed(
            RefactorOutput {
                candidates,
                total_matching,
            },
            &index,
            started,
            cached,
        ))
    }

    #[tool(
        name = "get_doc_coverage",
        description = "Every indexed file's wiki-page freshness, without generating anything: `missing` (no wiki page yet), `fresh` (the page's embedded content hash matches the file's current content), or `stale` (the file changed since `repowise docs` last generated its page). Read-only -- never writes a page; run `repowise docs`/`repowise generate` to update one."
    )]
    fn get_doc_coverage(&self) -> Result<Json<Envelope<DocCoverageOutput>>, ErrorData> {
        let started = Instant::now();
        let (index, _graph, cached) = self.load()?;

        let report = repowise_docs::check_freshness(&index);
        let (missing, fresh, stale) = report.counts();
        let entries = report
            .entries
            .into_iter()
            .map(|e| DocCoverageEntryOutput {
                file: display_rel(&e.file, &index.root),
                status: match e.status {
                    repowise_docs::FreshnessStatus::Missing => "missing",
                    repowise_docs::FreshnessStatus::Fresh => "fresh",
                    repowise_docs::FreshnessStatus::Stale => "stale",
                },
            })
            .collect();

        Ok(self.indexed(
            DocCoverageOutput {
                entries,
                missing,
                fresh,
                stale,
            },
            &index,
            started,
            cached,
        ))
    }

    #[tool(
        name = "get_coupling",
        description = "Repo-wide change coupling: the file pairs that most often change together in the same commit, regardless of whether an import edge connects them -- catches architectural coupling static analysis alone can't see. Ranks every pair in the repo, strongest first (unlike `get_health`'s hidden-coupling/co-change-scatter markers, which fold this signal into a per-file score rather than exposing the raw pairs). `limit` (default 30) caps the list."
    )]
    fn get_coupling(
        &self,
        Parameters(CouplingParams { limit }): Parameters<CouplingParams>,
    ) -> Result<Json<Envelope<CouplingOutput>>, ErrorData> {
        let started = Instant::now();
        let analytics = repowise_git::GitAnalytics::collect(&self.root).map_err(|e| {
            ErrorData::invalid_params(format!("failed to compute change coupling: {e}"), None)
        })?;

        let pairs = analytics
            .top_co_changed_pairs(limit)
            .into_iter()
            .map(|(a, b, count)| CouplingPairOutput {
                file_a: display_rel(&a, &self.root),
                file_b: display_rel(&b, &self.root),
                count,
            })
            .collect();

        Ok(self.untracked(CouplingOutput { pairs }, started))
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
        description = "Workspace-wide cross-repo import resolution: which workspace repos depend on which others, and the individual import sites behind each dependency. Covers Rust, Python, Java, Kotlin, Scala, Go, C#, and PHP (every language resolved single-repo via a name-to-file module map); every other language's cross-repo imports are left unresolved. Returns empty lists (not an error) when no --workspace was given, same degrade-gracefully shape as list_repos."
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

    /// A file big enough and gnarly enough to trigger markers, so the
    /// health tests aren't asserting against a uniformly perfect repo.
    fn messy_source() -> String {
        let mut s =
            String::from("pub fn tangled(a: i32, b: i32, c: i32, d: i32, e: i32) -> i32 {\n");
        for i in 0..12 {
            s.push_str(&format!(
                "    if a > {i} {{ if b > {i} {{ if c > {i} {{ if d > {i} {{ return e; }} }} }} }}\n"
            ));
        }
        s.push_str("    0\n}\n");
        s
    }

    #[test]
    fn get_health_repo_mode_ranks_worst_first_and_reports_both_averages() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("messy.rs"), messy_source()).unwrap();
        std::fs::write(root.join("clean.rs"), "pub fn ok() -> i32 { 1 }\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .get_health(Parameters(HealthParams {
                targets: Vec::new(),
                limit: HEALTH_DEFAULT_LIMIT,
            }))
            .unwrap();

        assert_eq!(data.mode, "repo");
        assert_eq!(data.files_total, 2);
        assert_eq!(data.files.len(), 2);
        assert!(
            data.files[0].score <= data.files[1].score,
            "worst file must come first: {:?}",
            data.files
                .iter()
                .map(|f| (&f.file, f.score))
                .collect::<Vec<_>>()
        );
        assert_eq!(data.average_score_weighting, Some("lines"));
        assert!(data.average_score.is_some());
        assert!(data.average_score_unweighted.is_some());
        // Line counts ride along so a caller can see why the two
        // averages differ without a second round-trip.
        assert!(data.files.iter().all(|f| f.lines > 0));
    }

    #[test]
    fn get_health_repo_mode_caps_the_list_but_reports_the_true_total() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for i in 0..8 {
            std::fs::write(
                root.join(format!("f{i}.rs")),
                format!("pub fn f{i}() -> i32 {{ {i} }}\n"),
            )
            .unwrap();
        }
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .get_health(Parameters(HealthParams {
                targets: Vec::new(),
                limit: 3,
            }))
            .unwrap();

        assert_eq!(data.files.len(), 3, "limit applies");
        assert_eq!(
            data.files_total, 8,
            "but the true count must still be reported, or 3 reads as 'there are only 3'"
        );
    }

    #[test]
    fn get_health_clamps_an_absurd_limit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.rs"), "pub fn a() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .get_health(Parameters(HealthParams {
                targets: Vec::new(),
                limit: 100_000,
            }))
            .unwrap();
        assert_eq!(data.files.len(), 1);
        assert_eq!(data.files_total, 1);
    }

    #[test]
    fn get_health_targeted_mode_scores_just_the_named_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("messy.rs"), messy_source()).unwrap();
        std::fs::write(root.join("clean.rs"), "pub fn ok() -> i32 { 1 }\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .get_health(Parameters(HealthParams {
                targets: vec!["messy.rs".to_string()],
                limit: HEALTH_DEFAULT_LIMIT,
            }))
            .unwrap();

        assert_eq!(data.mode, "targeted");
        assert_eq!(data.files.len(), 1);
        assert!(data.files[0].file.ends_with("messy.rs"));
        assert!(data.unresolved.is_empty());
        // Repo-wide averages must not ride along on a single-file
        // answer -- they'd read as that file's average.
        assert_eq!(data.average_score, None);
        assert_eq!(data.average_score_unweighted, None);
    }

    /// The reason `unresolved` exists: an empty `files` list with no
    /// explanation reads as "healthy", when the truth is "we couldn't
    /// look". The two reasons need different fixes, so they're reported
    /// separately rather than as one generic miss.
    #[test]
    fn get_health_names_unresolved_targets_with_distinct_reasons() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.rs"), "pub fn a() {}\n").unwrap();
        build_and_save_index(&root);
        // Exists on disk, deliberately created after indexing.
        std::fs::write(root.join("late.rs"), "pub fn late() {}\n").unwrap();

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .get_health(Parameters(HealthParams {
                targets: vec!["late.rs".to_string(), "ghost.rs".to_string()],
                limit: HEALTH_DEFAULT_LIMIT,
            }))
            .unwrap();

        assert!(data.files.is_empty());
        assert_eq!(data.files_total, 0);
        let reasons: Vec<_> = data.unresolved.iter().map(|u| u.reason).collect();
        assert!(
            reasons.contains(&"not_indexed"),
            "a file present on disk but absent from the index needs a re-index, \
             and the response must say so: {reasons:?}"
        );
        assert!(
            reasons.contains(&"no_such_path"),
            "a path that doesn't exist is a different problem: {reasons:?}"
        );
    }

    #[test]
    fn weighted_average_is_dragged_down_by_a_large_bad_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("messy.rs"), messy_source()).unwrap();
        for i in 0..6 {
            std::fs::write(
                root.join(format!("tiny{i}.rs")),
                format!("pub fn t{i}() {{}}\n"),
            )
            .unwrap();
        }
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .get_health(Parameters(HealthParams {
                targets: Vec::new(),
                limit: HEALTH_DEFAULT_LIMIT,
            }))
            .unwrap();

        let weighted = data.average_score.unwrap();
        let plain = data.average_score_unweighted.unwrap();
        assert!(
            weighted < plain,
            "one large low-scoring file among tiny clean ones should pull the \
             line-weighted mean below the file mean (weighted {weighted}, plain {plain})"
        );
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
                ..Default::default()
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
            ..Default::default()
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
                limit: CONTEXT_DEFAULT_LIMIT,
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
            limit: CONTEXT_DEFAULT_LIMIT,
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
    fn get_coupling_ranks_the_most_co_changed_pair_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git_init(&root);

        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        std::fs::write(root.join("b.txt"), "b\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "add a and b together"]);
        std::fs::write(root.join("a.txt"), "a2\n").unwrap();
        std::fs::write(root.join("b.txt"), "b2\n").unwrap();
        git(
            &root,
            &["commit", "-q", "-am", "change a and b together again"],
        );
        std::fs::write(root.join("c.txt"), "c\n").unwrap();
        std::fs::write(root.join("a.txt"), "a3\n").unwrap();
        git(&root, &["add", "."]);
        git(
            &root,
            &["commit", "-q", "-am", "change a and c together once"],
        );

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .get_coupling(Parameters(CouplingParams::default()))
            .unwrap();

        assert!(!data.pairs.is_empty());
        assert_eq!(data.pairs[0].file_a, "a.txt");
        assert_eq!(data.pairs[0].file_b, "b.txt");
        assert_eq!(data.pairs[0].count, 2);
    }

    #[test]
    fn get_coupling_errors_when_not_a_git_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let server = RepowiseServer::new(root, None);
        let result = server.get_coupling(Parameters(CouplingParams::default()));
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
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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

    /// Without the inferred pass having run, `get_why` must say so.
    /// An absent contribution is otherwise indistinguishable from a repo
    /// with nothing to infer.
    #[test]
    fn get_why_reports_the_inferred_source_state_even_when_it_contributed_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_two_decision_fixture(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data: why, .. }) = server
            .get_why(Parameters(WhyParams { targets: vec![] }))
            .unwrap();

        assert!(
            why.inferred_source.contains("opt-in"),
            "{}",
            why.inferred_source
        );
        // No inferred decisions were returned, so the caveat about them
        // must not fire -- a warning that's always present stops being
        // read as a warning.
        assert!(why.inferred_caveat.is_none());
        assert!(why.decisions.iter().all(|d| !d.inferred));
    }

    /// The whole point of the source: an inferred decision must arrive
    /// flagged, caveated, and distinguishable from the mined ones beside
    /// it in the same list.
    #[test]
    fn get_why_flags_inferred_decisions_and_caveats_the_response() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_two_decision_fixture(&root);
        std::fs::write(root.join("anchored.rs"), "let q = Bounded::new(128);\n").unwrap();
        repowise_adr::InferredStore {
            model: "fixture-model".to_string(),
            decisions: vec![repowise_adr::InferredDecision {
                title: "Bounded queue".to_string(),
                rationale: "Backpressure over unbounded growth.".to_string(),
                file: "anchored.rs".to_string(),
                anchor: "let q = Bounded::new(128);".to_string(),
            }],
        }
        .save(&root)
        .unwrap();

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data: why, .. }) = server
            .get_why(Parameters(WhyParams { targets: vec![] }))
            .unwrap();

        let inferred: Vec<_> = why.decisions.iter().filter(|d| d.inferred).collect();
        assert_eq!(
            inferred.len(),
            1,
            "{:?}",
            why.decisions.iter().map(|d| &d.source).collect::<Vec<_>>()
        );
        assert_eq!(inferred[0].title, "Bounded queue");
        assert!(
            inferred[0].source.starts_with("inferred:"),
            "the source string must say so too, not only the flag: {}",
            inferred[0].source
        );
        assert!(
            inferred[0].source.contains("fixture-model"),
            "a reader judging an inferred claim is entitled to know what inferred it: {}",
            inferred[0].source
        );

        // The mined decisions in the same response stay unflagged.
        assert!(why.decisions.iter().filter(|d| !d.inferred).count() >= 2);

        let caveat = why
            .inferred_caveat
            .expect("a response containing inferred decisions must carry the caveat");
        assert!(caveat.contains("not") && caveat.contains("recorded intent"));
    }

    /// Filtering to a file with no inferred decisions must drop the
    /// caveat with them: it describes what was returned, not what the
    /// repo happens to hold.
    #[test]
    fn get_why_drops_the_caveat_when_the_filter_excludes_every_inferred_decision() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_two_decision_fixture(&root);
        std::fs::write(root.join("anchored.rs"), "let q = Bounded::new(128);\n").unwrap();
        repowise_adr::InferredStore {
            model: "fixture-model".to_string(),
            decisions: vec![repowise_adr::InferredDecision {
                title: "Bounded queue".to_string(),
                rationale: "Backpressure over unbounded growth.".to_string(),
                file: "anchored.rs".to_string(),
                anchor: "let q = Bounded::new(128);".to_string(),
            }],
        }
        .save(&root)
        .unwrap();

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data: why, .. }) = server
            .get_why(Parameters(WhyParams {
                targets: vec!["src/queue.rs".to_string()],
            }))
            .unwrap();

        assert!(why.decisions.iter().all(|d| !d.inferred));
        assert!(why.inferred_caveat.is_none());
        // The state line still reports the source, because "you never
        // ran the pass" stays worth knowing even under a filter.
        assert!(why.inferred_source.contains("fixture-model"));
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
    fn get_refactor_candidates_finds_a_god_class() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut body = String::from("class Big:\n");
        for i in 0..(repowise_health::GOD_CLASS_METHODS + 1) {
            body.push_str(&format!("    def m{i}(self):\n        return {i}\n"));
        }
        std::fs::write(root.join("big.py"), body).unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .get_refactor_candidates(Parameters(RefactorParams::default()))
            .unwrap();

        let god = data
            .candidates
            .iter()
            .find(|c| c.kind == "split-god-class")
            .expect("Big must be flagged");
        assert_eq!(god.symbols, vec!["Big".to_string()]);
        assert!(god
            .rationale
            .contains(&(repowise_health::GOD_CLASS_METHODS + 1).to_string()));
    }

    #[test]
    fn get_refactor_candidates_kind_filter_narrows_to_one_category() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut body = String::from("class Big:\n");
        for i in 0..(repowise_health::GOD_CLASS_METHODS + 1) {
            body.push_str(&format!("    def m{i}(self):\n        return {i}\n"));
        }
        std::fs::write(root.join("big.py"), body).unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .get_refactor_candidates(Parameters(RefactorParams {
                kind: Some("extract-duplicate".to_string()),
                limit: default_refactor_limit(),
            }))
            .unwrap();

        assert!(
            data.candidates
                .iter()
                .all(|c| c.kind == "extract-duplicate"),
            "{:?}",
            data.candidates
        );
        assert_eq!(data.total_matching, 0, "no duplicates in this fixture");
    }

    #[test]
    fn get_refactor_candidates_rejects_an_unknown_kind() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let result = server.get_refactor_candidates(Parameters(RefactorParams {
            kind: Some("rewrite-everything".to_string()),
            limit: default_refactor_limit(),
        }));
        let Err(err) = result else {
            panic!("expected an error for an unknown kind");
        };
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    /// The regression this tool exists to guard against: running it
    /// against a repo with many structurally-similar functions must
    /// cap the response and report the true count, not dump everything.
    #[test]
    fn get_refactor_candidates_limit_truncates_but_total_matching_reports_the_full_count() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        // Four files, each pairing up with every other -- 6 duplicate
        // pairs from one repeated 4-line body.
        for name in ["a", "b", "c", "d"] {
            std::fs::write(
                root.join(format!("{name}.py")),
                "def compute_total(items):\n    total = 0\n    for x in items:\n        total += x\n    return total\n",
            )
            .unwrap();
        }
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .get_refactor_candidates(Parameters(RefactorParams {
                kind: Some("extract-duplicate".to_string()),
                limit: 2,
            }))
            .unwrap();

        assert_eq!(data.candidates.len(), 2);
        assert_eq!(data.total_matching, 6, "{:?}", data.candidates);
        assert!(data
            .candidates
            .iter()
            .all(|c| matches!(&c.kind[..], "extract-duplicate")),);
    }

    #[test]
    fn get_doc_coverage_reports_missing_for_a_file_with_no_wiki_page() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("solo.py"), "def solo():\n    return 1\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server.get_doc_coverage().unwrap();

        assert_eq!(data.missing, 1);
        assert_eq!(data.fresh, 0);
        assert_eq!(data.stale, 0);
        assert_eq!(data.entries[0].status, "missing");
    }

    #[test]
    fn get_doc_coverage_reports_fresh_right_after_generation_and_stale_after_an_edit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("solo.py"), "def solo():\n    return 1\n").unwrap();
        build_and_save_index(&root);

        let index = RepoIndex::load(&root).unwrap();
        let graph = RepoGraph::build(&index);
        let health = repowise_health::analyze(&index, &graph);
        repowise_docs::generate(&index, &graph, &health).unwrap();

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data, .. }) = server.get_doc_coverage().unwrap();
        assert_eq!(data.fresh, 1);
        assert_eq!(data.entries[0].status, "fresh");

        std::fs::write(root.join("solo.py"), "def solo():\n    return 2\n").unwrap();
        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server.get_doc_coverage().unwrap();
        assert_eq!(data.stale, 1);
        assert_eq!(data.entries[0].status, "stale");
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

    /// Path search was previously impossible to express -- only symbol
    /// names were searchable, so "which file is the config loader in"
    /// had no query that answered it.
    #[test]
    fn search_codebase_path_mode_matches_files_not_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(root.join("config/loader.rs"), "pub fn load() {}\n").unwrap();
        std::fs::write(root.join("other.rs"), "pub fn config_thing() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .search_codebase(Parameters(SearchParams {
                query: "config".to_string(),
                mode: Some("path".to_string()),
                ..Default::default()
            }))
            .unwrap();

        assert!(
            data.file_matches
                .iter()
                .any(|f| f.file.contains("loader.rs")),
            "{:?}",
            data.file_matches
        );
        assert!(
            data.matches.is_empty(),
            "path mode must not return symbol hits: {:?}",
            data.matches
        );
    }

    #[test]
    fn search_codebase_hybrid_mode_returns_both() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(root.join("config/loader.rs"), "pub fn load() {}\n").unwrap();
        std::fs::write(root.join("other.rs"), "pub fn config_thing() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .search_codebase(Parameters(SearchParams {
                query: "config".to_string(),
                mode: Some("hybrid".to_string()),
                ..Default::default()
            }))
            .unwrap();

        assert!(!data.matches.is_empty(), "expected the symbol hit");
        assert!(!data.file_matches.is_empty(), "expected the path hit");
    }

    #[test]
    fn search_codebase_kind_filter_excludes_tests() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(root.join("thing.rs"), "pub fn parse_it() {}\n").unwrap();
        std::fs::write(root.join("tests/thing.rs"), "pub fn parse_it_test() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .search_codebase(Parameters(SearchParams {
                query: "parse_it".to_string(),
                kind: Some("implementation".to_string()),
                ..Default::default()
            }))
            .unwrap();

        assert_eq!(data.matches.len(), 1, "{:?}", data.matches);
        assert!(
            !data.matches[0].file.contains("tests/"),
            "{:?}",
            data.matches
        );
    }

    /// Semantic mode without an embedding endpoint must **error**, not
    /// return an empty ranking. Zero results reads as "the repo has
    /// nothing matching" — a false negative an agent will act on —
    /// when the truth is that no search ran at all.
    #[test]
    fn search_codebase_semantic_without_an_endpoint_errors_rather_than_ranking_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.rs"), "pub fn a() {}\n").unwrap();
        build_and_save_index(&root);

        // Guard the premise: with a base URL set this would exercise a
        // different branch and quietly stop testing what it claims to.
        if std::env::var("REPOWISE_LLM_BASE_URL").is_ok() {
            return;
        }

        let server = RepowiseServer::new(root, None);
        let err = server
            .search_codebase(Parameters(SearchParams {
                query: "how does auth work".to_string(),
                mode: Some("semantic".to_string()),
                ..Default::default()
            }))
            .err()
            .expect("semantic without an endpoint must error, not return an empty ranking");
        assert!(
            err.message.contains("REPOWISE_LLM_BASE_URL"),
            "must name the missing piece so it can be fixed: {}",
            err.message
        );
    }

    /// A filter that can't be honoured must be refused, not dropped:
    /// silently ignoring `kind` would return unfiltered results the
    /// caller believes are filtered.
    #[test]
    fn search_codebase_semantic_refuses_filters_it_cannot_apply() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.rs"), "pub fn a() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let err = server
            .search_codebase(Parameters(SearchParams {
                query: "how does auth work".to_string(),
                mode: Some("semantic".to_string()),
                kind: Some("test".to_string()),
                ..Default::default()
            }))
            .err()
            .expect("an inapplicable filter must be refused");
        assert!(
            err.message.contains("kind"),
            "must say which argument doesn't apply: {}",
            err.message
        );
        // Specifically not the unavailability error: the filter is
        // wrong regardless of whether an endpoint is configured, so it
        // has to be caught first.
        assert!(
            !err.message.contains("REPOWISE_LLM_BASE_URL"),
            "the argument error must be reported before the availability one: {}",
            err.message
        );
    }

    /// The substring modes must not sprout semantic fields. Serializing
    /// `coverage: 0 of N` on a symbol search would claim the search was
    /// limited by an index that has nothing to do with it.
    #[test]
    fn substring_modes_report_no_semantic_coverage() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.rs"), "pub fn alpha() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        for mode in ["symbol", "path", "hybrid"] {
            let out = server
                .search_codebase(Parameters(SearchParams {
                    query: "a".to_string(),
                    mode: Some(mode.to_string()),
                    ..Default::default()
                }))
                .unwrap()
                .0
                .data;
            assert!(out.coverage.is_none(), "{mode} must not report coverage");
            assert!(
                out.semantic_matches_total.is_none(),
                "{mode} must not report a semantic total"
            );
            assert!(out.semantic_matches.is_empty());
        }
    }

    /// An empty result with filters active is ambiguous unless the
    /// response says what was filtered.
    #[test]
    fn search_codebase_echoes_the_filters_it_applied() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.rs"), "pub fn a() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .search_codebase(Parameters(SearchParams {
                query: "zzz-no-such-thing".to_string(),
                kind: Some("test".to_string()),
                symbol_kind: Some("function".to_string()),
                ..Default::default()
            }))
            .unwrap();

        assert!(data.matches.is_empty());
        assert!(data.filters.contains("kind=test"), "{}", data.filters);
        assert!(
            data.filters.contains("symbol_kind=function"),
            "{}",
            data.filters
        );
    }

    /// A file with many small symbols was the case that made
    /// `get_context` cost more than reading the file it described.
    fn dense_source(symbols: usize) -> String {
        (0..symbols)
            .map(|i| format!("pub fn f{i}() -> i32 {{ {i} }}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn get_context_caps_its_lists_but_reports_the_true_totals() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), dense_source(300)).unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .get_context(Parameters(ContextParams {
                file: "lib.rs".to_string(),
                limit: CONTEXT_DEFAULT_LIMIT,
            }))
            .unwrap();

        assert_eq!(data.symbols.len(), CONTEXT_DEFAULT_LIMIT);
        assert_eq!(
            data.symbols_total, 300,
            "a truncated list must not be readable as the whole file"
        );
    }

    #[test]
    fn get_context_clamps_an_absurd_limit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), dense_source(20)).unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .get_context(Parameters(ContextParams {
                file: "lib.rs".to_string(),
                limit: 100_000,
            }))
            .unwrap();
        assert_eq!(data.symbols.len(), 20);
        assert_eq!(data.symbols_total, 20);
    }

    /// The stored id embeds this machine's absolute path. Emitting it
    /// leaked the producing machine's layout and, on a dense file,
    /// accounted for most of the response.
    #[test]
    fn get_context_ids_are_repo_relative_not_absolute() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "pub fn only() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root.clone(), None);
        let Json(Envelope { data, .. }) = server
            .get_context(Parameters(ContextParams {
                file: "lib.rs".to_string(),
                limit: CONTEXT_DEFAULT_LIMIT,
            }))
            .unwrap();

        let id = &data.symbols[0].id;
        assert_eq!(id, "lib.rs::only@1", "{id}");
        assert!(
            !id.contains(&root.display().to_string()),
            "the response must not carry the producing machine's paths: {id}"
        );
    }

    /// Both id forms have to resolve: ids handed out before this change
    /// are still in agent transcripts and scrollback.
    #[test]
    fn get_symbol_accepts_both_the_portable_and_the_legacy_id() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "pub fn only() -> i32 { 1 }\n").unwrap();
        build_and_save_index(&root);
        let index = RepoIndex::load(&root).unwrap();
        let stored = index.files[0].symbols[0].id.clone();
        assert!(
            stored.contains(&root.display().to_string()),
            "precondition: the stored id is absolute"
        );

        let server = RepowiseServer::new(root, None);
        for id in [stored.as_str(), "lib.rs::only@1"] {
            let Json(Envelope { data, .. }) = server
                .get_symbol(Parameters(GetSymbolParams {
                    symbol_id: id.to_string(),
                    context_lines: 0,
                }))
                .unwrap_or_else(|e| panic!("id {id:?} should resolve: {}", e.message));
            assert_eq!(
                data.id, "lib.rs::only@1",
                "both forms must come back as the portable one"
            );
        }
    }

    /// The measurable point of the change, asserted rather than assumed.
    #[test]
    fn capping_shrinks_a_dense_file_response_by_an_order_of_magnitude() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), dense_source(300)).unwrap();
        build_and_save_index(&root);
        let server = RepowiseServer::new(root, None);

        let size = |limit: usize| {
            let Json(env) = server
                .get_context(Parameters(ContextParams {
                    file: "lib.rs".to_string(),
                    limit,
                }))
                .unwrap();
            serde_json::to_string(&env).unwrap().len()
        };

        let capped = size(CONTEXT_DEFAULT_LIMIT);
        let uncapped = size(CONTEXT_MAX_LIMIT);
        assert!(
            capped * 5 < uncapped,
            "the default cap should be a large win on a dense file \
             (capped {capped}, uncapped {uncapped})"
        );
    }

    /// Unconfigured is a reportable state, not an error and not an
    /// empty answer. An agent must be able to tell "the feature is off"
    /// from "the codebase has nothing to say".
    #[test]
    fn get_answer_reports_unavailable_without_an_llm_endpoint() {
        // The env var is process-global; this asserts the behaviour for
        // the unset case, which is how CI runs.
        if std::env::var_os("REPOWISE_LLM_BASE_URL").is_some() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "pub fn helper() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let Json(Envelope { data, .. }) = server
            .get_answer(Parameters(AnswerParams {
                question: "how does this work".to_string(),
            }))
            .unwrap();

        assert!(!data.available);
        assert!(data.answer.is_none(), "must not fabricate an answer");
        let reason = data.unavailable_reason.expect("must say why");
        assert!(reason.contains("REPOWISE_LLM_BASE_URL"), "{reason}");
        assert!(
            reason.contains("Every other tool"),
            "an agent shouldn't conclude the whole server is broken: {reason}"
        );
    }

    #[test]
    fn get_answer_rejects_an_empty_question() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "pub fn helper() {}\n").unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let err = server
            .get_answer(Parameters(AnswerParams {
                question: "   ".to_string(),
            }))
            .err()
            .expect("an empty question is invalid params");
        assert!(err.message.contains("must not be empty"), "{}", err.message);
    }

    /// Two repos each with their own `helper_*` function, so a search for
    /// `"helper"` matches in both -- unlike `two_repo_workspace`, which has
    /// no shared search term across its two repos.
    fn two_repo_workspace_with_matching_symbols(
        dir: &Path,
    ) -> Vec<repowise_workspace::ResolvedWorkspaceRepo> {
        let repo_a = dir.join("repo-a");
        write_crate(
            &repo_a,
            "repo-a",
            &[("src/foo.rs", "pub fn helper_alpha() -> i32 { 1 }\n")],
        );
        build_and_save_index(&repo_a);

        let repo_b = dir.join("repo-b");
        write_crate(
            &repo_b,
            "repo-b",
            &[("src/lib.rs", "pub fn helper_beta() -> i32 { 2 }\n")],
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
    fn search_codebase_with_no_repo_param_stays_scoped_to_this_servers_own_root() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let workspace_root = workspace_dir.path().canonicalize().unwrap();
        let workspace_repos = two_repo_workspace_with_matching_symbols(&workspace_root);

        // This server's own root is a separate directory entirely, not
        // nested under either workspace repo -- proves the unscoped
        // default never wanders into `workspace_repos` at all.
        let self_dir = tempfile::tempdir().unwrap();
        let root = self_dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, Some(workspace_repos));
        let Json(Envelope { data, .. }) = server
            .search_codebase(Parameters(SearchParams {
                query: "helper".to_string(),
                ..Default::default()
            }))
            .unwrap();

        assert!(data.matches.is_empty(), "{:?}", data.matches);
        assert!(!data.filters.contains("repo="), "{}", data.filters);
    }

    #[test]
    fn search_codebase_repo_all_federates_across_every_workspace_repo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let workspace_repos = two_repo_workspace_with_matching_symbols(&root);

        let server = RepowiseServer::new(root, Some(workspace_repos));
        let Json(Envelope { data, .. }) = server
            .search_codebase(Parameters(SearchParams {
                query: "helper".to_string(),
                repo: Some("all".to_string()),
                ..Default::default()
            }))
            .unwrap();

        assert_eq!(data.matches.len(), 2, "{:?}", data.matches);
        let mut repos: Vec<_> = data.matches.iter().map(|m| m.repo.as_deref()).collect();
        repos.sort();
        assert_eq!(repos, vec![Some("repo-a"), Some("repo-b")]);
        let mut names: Vec<_> = data.matches.iter().map(|m| m.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["helper_alpha", "helper_beta"]);
    }

    #[test]
    fn search_codebase_with_a_named_repo_searches_only_that_repo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let workspace_repos = two_repo_workspace_with_matching_symbols(&root);

        let server = RepowiseServer::new(root, Some(workspace_repos));
        let Json(Envelope { data, .. }) = server
            .search_codebase(Parameters(SearchParams {
                query: "helper".to_string(),
                repo: Some("repo-b".to_string()),
                ..Default::default()
            }))
            .unwrap();

        assert_eq!(data.matches.len(), 1, "{:?}", data.matches);
        assert_eq!(data.matches[0].name, "helper_beta");
        assert_eq!(data.matches[0].repo.as_deref(), Some("repo-b"));
    }

    #[test]
    fn search_codebase_repo_param_requires_a_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        build_and_save_index(&root);

        let server = RepowiseServer::new(root, None);
        let result = server.search_codebase(Parameters(SearchParams {
            query: "helper".to_string(),
            repo: Some("all".to_string()),
            ..Default::default()
        }));
        let Err(err) = result else {
            panic!("expected an error with no workspace configured");
        };
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn search_codebase_repo_param_errors_on_unknown_repo_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let workspace_repos = two_repo_workspace_with_matching_symbols(&root);

        let server = RepowiseServer::new(root, Some(workspace_repos));
        let result = server.search_codebase(Parameters(SearchParams {
            query: "helper".to_string(),
            repo: Some("nonexistent".to_string()),
            ..Default::default()
        }));
        let Err(err) = result else {
            panic!("expected an error for an unknown repo name");
        };
        assert_eq!(err.code, ErrorCode::RESOURCE_NOT_FOUND);
    }

    #[test]
    fn search_codebase_semantic_mode_rejects_the_repo_parameter() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let workspace_repos = two_repo_workspace_with_matching_symbols(&root);

        let server = RepowiseServer::new(root, Some(workspace_repos));
        let result = server.search_codebase(Parameters(SearchParams {
            query: "helper".to_string(),
            mode: Some("semantic".to_string()),
            repo: Some("all".to_string()),
            ..Default::default()
        }));
        let Err(err) = result else {
            panic!("expected semantic mode to reject the repo parameter");
        };
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("repo"), "{}", err.message);
    }
}
